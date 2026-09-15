//! Relay display tunnel handling for daemon-owned local display endpoints.

use super::*;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use std::io::Read;
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::{HeaderName, HeaderValue};
use tokio_tungstenite::tungstenite::Message;

const DISPLAY_PROXY_CHUNK_BYTES: usize = 8 * 1024;
const DISPLAY_PROXY_MAX_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;
const SELKIES_ACTIVE_LEASE: Duration = Duration::from_secs(60);
const SELKIES_ACTIVE_LEASE_RENEWAL: Duration = Duration::from_secs(20);

pub(super) async fn handle_display_tunnel_open(
    state: Arc<RwLock<RelayClientState>>,
    outgoing_tx: RelayOutgoingSender,
    request: RelayDisplayTunnelOpenRequest,
    daemon_private_key: String,
) {
    let stream_id = request.stream_id.clone();
    let target = {
        let mut guard = state.write().await;
        guard.claim_display_tunnel_for_open(&request.tunnel_id, crate::session::unix_epoch_ms())
    };
    if display_request_is_websocket(&request) {
        if let Some(target) = target {
            handle_display_tunnel_websocket(
                state,
                outgoing_tx,
                request,
                target,
                daemon_private_key,
            )
            .await;
        } else {
            close_display_tunnel_stream(
                &outgoing_tx,
                stream_id,
                relay_error(
                    "display_tunnel_not_found",
                    "display tunnel is not registered or has expired",
                    false,
                ),
            );
        }
        return;
    }
    let result = match target {
        Some(target) => {
            let outgoing_tx = outgoing_tx.clone();
            tokio::task::spawn_blocking(move || {
                proxy_display_request(&outgoing_tx, &target, &request)
            })
            .await
            .map_err(|error| relay_error("display_proxy_join_failed", &error.to_string(), true))
            .and_then(|result| result)
        }
        None => Err(relay_error(
            "display_tunnel_not_found",
            "display tunnel is not registered or has expired",
            false,
        )),
    };
    match result {
        Ok(()) => {
            let _ = send_outgoing_envelope(
                &outgoing_tx,
                RelayEnvelope::DaemonDisplayTunnelClose {
                    stream_id,
                    error: None,
                },
            );
        }
        Err(error) => {
            close_display_tunnel_stream(&outgoing_tx, stream_id, error);
        }
    }
}

fn close_display_tunnel_stream(
    outgoing_tx: &RelayOutgoingSender,
    stream_id: String,
    error: RelayError,
) {
    let _ = send_outgoing_envelope(
        outgoing_tx,
        RelayEnvelope::DaemonDisplayTunnelClose {
            stream_id,
            error: Some(error),
        },
    );
}

async fn handle_display_tunnel_websocket(
    state: Arc<RwLock<RelayClientState>>,
    outgoing_tx: RelayOutgoingSender,
    request: RelayDisplayTunnelOpenRequest,
    target: RelayDisplayTunnelTarget,
    daemon_private_key: String,
) {
    let stream_id = request.stream_id.clone();
    let queue_capacity = if matches!(&target.kind, RelayDisplayTunnelTargetKind::Selkies { .. }) {
        16
    } else {
        128
    };
    let (client_tx, client_rx) = mpsc::channel(queue_capacity);
    state
        .write()
        .await
        .insert_display_stream(stream_id.clone(), client_tx);
    let selkies = matches!(&target.kind, RelayDisplayTunnelTargetKind::Selkies { .. });
    let result = if selkies {
        proxy_selkies_websocket(
            &outgoing_tx,
            &target,
            &request,
            client_rx,
            daemon_private_key,
        )
        .await
    } else {
        proxy_display_websocket(&outgoing_tx, &target, &request, client_rx).await
    };
    state.write().await.remove_display_stream(&stream_id);
    if selkies {
        let _ = outgoing_tx.try_send(RelayEnvelope::DaemonDisplayTunnelRevoke {
            tunnel_id: target.tunnel_id.clone(),
        });
    }
    match result {
        Ok(()) => {
            let _ = send_outgoing_envelope(
                &outgoing_tx,
                RelayEnvelope::DaemonDisplayTunnelClose {
                    stream_id,
                    error: None,
                },
            );
        }
        Err(error) => {
            let _ = send_outgoing_envelope(
                &outgoing_tx,
                RelayEnvelope::DaemonDisplayTunnelClose {
                    stream_id,
                    error: Some(error),
                },
            );
        }
    }
}

async fn proxy_selkies_websocket(
    outgoing_tx: &RelayOutgoingSender,
    target: &RelayDisplayTunnelTarget,
    request: &RelayDisplayTunnelOpenRequest,
    mut client_rx: mpsc::Receiver<RelayDisplayTunnelClientEvent>,
    daemon_private_key: String,
) -> Result<(), RelayError> {
    let expected_path = format!("/display/{}/stream", target.tunnel_id);
    if !request.method.eq_ignore_ascii_case("GET") || request.path != expected_path {
        return Err(relay_error(
            "display_stream_path_invalid",
            "Selkies display stream path is invalid",
            false,
        ));
    }
    let RelayDisplayTunnelTargetKind::Selkies {
        viewer_public_key,
        command_program,
        command_args,
    } = &target.kind
    else {
        return Err(relay_error(
            "display_target_invalid",
            "display target is not a Selkies stream",
            false,
        ));
    };
    let now_ms = crate::session::unix_epoch_ms();
    // The target expiry bounds the one-time opening grant. Once claimed, the
    // live socket owns a separate short kernel lease that is renewed only
    // while this handler remains attached to the admitted viewer.
    target
        .expires_at_ms
        .checked_sub(now_ms)
        .filter(|remaining| *remaining > 0 && *remaining <= 60_000)
        .ok_or_else(|| {
            relay_error(
                "display_admission_expired",
                "Selkies display admission is expired or invalid",
                false,
            )
        })?;
    let cipher = crate::transport::secure_display::SecureDisplayChannel::new(
        daemon_private_key,
        viewer_public_key.clone(),
        &target.tunnel_id,
        crate::transport::secure_display::DisplayPeer::Kernel,
    )
    .map_err(|_| {
        relay_error(
            "display_viewer_key_invalid",
            "Selkies viewer key is invalid",
            false,
        )
    })?;
    let mut command = Command::new(command_program);
    command.args(command_args);
    let (input_tx, input_rx) = mpsc::channel(16);
    let (output_tx, mut output_rx) = mpsc::channel(16);
    let (lease_tx, lease_rx) =
        tokio::sync::watch::channel(Some(tokio::time::Instant::now() + SELKIES_ACTIVE_LEASE));
    let mut lease_renewal = tokio::time::interval(SELKIES_ACTIVE_LEASE_RENEWAL);
    lease_renewal.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut forward = tokio::spawn(crate::transport::selkies_stream::forward_selkies_stream(
        command, cipher, input_rx, output_tx, lease_rx,
    ));
    send_outgoing_envelope(
        outgoing_tx,
        RelayEnvelope::DaemonDisplayTunnelResponseStart {
            response: RelayDisplayTunnelResponseStart {
                stream_id: request.stream_id.clone(),
                status: 101,
                headers: Vec::new(),
            },
        },
    )
    .map_err(|error| relay_error("display_websocket_start_failed", &error.to_string(), true))?;
    loop {
        tokio::select! {
            _ = lease_renewal.tick() => {
                lease_tx.send_replace(Some(
                    tokio::time::Instant::now() + SELKIES_ACTIVE_LEASE,
                ));
            }
            result = &mut forward => {
                return match result {
                    Ok(Ok(())) => Ok(()),
                    _ => Err(relay_error(
                        "display_stream_closed",
                        "Selkies display stream closed",
                        true,
                    )),
                };
            }
            packet = output_rx.recv() => {
                let Some(packet) = packet else { break; };
                let bytes = serde_json::to_vec(&packet).map_err(|_| {
                    relay_error(
                        "display_stream_encode_failed",
                        "Selkies display packet could not be encoded",
                        false,
                    )
                })?;
                send_display_chunk(outgoing_tx, &request.stream_id, &bytes, Some("binary"))?;
            }
            event = client_rx.recv() => {
                match event {
                    Some(RelayDisplayTunnelClientEvent::Chunk(chunk)) => {
                        if chunk.message_kind.as_deref() != Some("binary") {
                            return Err(relay_error(
                                "display_stream_packet_invalid",
                                "Selkies display packets must be binary",
                                false,
                            ));
                        }
                        let bytes = BASE64_STANDARD.decode(chunk.data.as_bytes()).map_err(|_| {
                            relay_error(
                                "display_stream_packet_invalid",
                                "Selkies display packet encoding is invalid",
                                false,
                            )
                        })?;
                        let packet = serde_json::from_slice(&bytes).map_err(|_| {
                            relay_error(
                                "display_stream_packet_invalid",
                                "Selkies display packet is invalid",
                                false,
                            )
                        })?;
                        timeout(Duration::from_secs(2), input_tx.send(packet))
                            .await
                            .map_err(|_| relay_error(
                                "display_stream_backpressure",
                                "Selkies display input queue is full",
                                true,
                            ))?
                            .map_err(|_| relay_error(
                                "display_stream_closed",
                                "Selkies display stream closed",
                                true,
                            ))?;
                    }
                    Some(RelayDisplayTunnelClientEvent::Close) | None => break,
                }
            }
        }
    }
    drop(input_tx);
    drop(lease_tx);
    match timeout(Duration::from_secs(8), forward).await {
        Ok(Ok(Ok(()))) => Ok(()),
        _ => Err(relay_error(
            "display_stream_closed",
            "Selkies display stream did not stop cleanly",
            true,
        )),
    }
}

fn proxy_display_request(
    outgoing_tx: &RelayOutgoingSender,
    target: &RelayDisplayTunnelTarget,
    request: &RelayDisplayTunnelOpenRequest,
) -> Result<(), RelayError> {
    if display_request_is_optional_package_probe(request) {
        return send_package_probe_response_to_proxy(outgoing_tx, request);
    }
    let url = local_display_url(target, &request.path)?;
    let mut builder = ureq::request(&request.method, url.as_str());
    for header in request
        .headers
        .iter()
        .filter(|header| forward_request_header(&header.name))
    {
        builder = builder.set(header.name.as_str(), header.value.as_str());
    }
    let body = request
        .body_base64
        .as_ref()
        .map(|body| {
            BASE64_STANDARD.decode(body.as_bytes()).map_err(|error| {
                relay_error("display_proxy_body_invalid", &error.to_string(), false)
            })
        })
        .transpose()?;
    let response = match body {
        Some(body) => builder.send_bytes(&body),
        None if method_uses_empty_body(&request.method) => builder.send_bytes(&[]),
        None => builder.call(),
    };
    match response {
        Ok(response) => stream_response_to_proxy(outgoing_tx, request, response),
        Err(ureq::Error::Status(_, response)) => {
            stream_response_to_proxy(outgoing_tx, request, response)
        }
        Err(error) => Err(relay_error(
            "display_proxy_failed",
            &error.to_string(),
            true,
        )),
    }
}

fn display_request_is_optional_package_probe(request: &RelayDisplayTunnelOpenRequest) -> bool {
    request.method.eq_ignore_ascii_case("GET")
        && request
            .path
            .split_once('?')
            .map(|(path, _)| path)
            .unwrap_or(request.path.as_str())
            .ends_with("/package.json")
}

fn send_package_probe_response_to_proxy(
    outgoing_tx: &RelayOutgoingSender,
    request: &RelayDisplayTunnelOpenRequest,
) -> Result<(), RelayError> {
    send_outgoing_envelope(
        outgoing_tx,
        RelayEnvelope::DaemonDisplayTunnelResponseStart {
            response: RelayDisplayTunnelResponseStart {
                stream_id: request.stream_id.clone(),
                status: 200,
                headers: vec![RelayDisplayTunnelHeader {
                    name: "content-type".to_string(),
                    value: "application/json".to_string(),
                }],
            },
        },
    )
    .map_err(|error| relay_error("display_proxy_start_send_failed", &error.to_string(), true))?;
    send_display_chunk(outgoing_tx, &request.stream_id, b"{}", None)
}

fn method_uses_empty_body(method: &str) -> bool {
    matches!(
        method.to_ascii_uppercase().as_str(),
        "POST" | "PUT" | "PATCH"
    )
}

async fn proxy_display_websocket(
    outgoing_tx: &RelayOutgoingSender,
    target: &RelayDisplayTunnelTarget,
    request: &RelayDisplayTunnelOpenRequest,
    mut client_rx: mpsc::Receiver<RelayDisplayTunnelClientEvent>,
) -> Result<(), RelayError> {
    let url = local_display_websocket_url(target, &request.path)?;
    let mut local_request = url.as_str().into_client_request().map_err(|error| {
        relay_error(
            "display_websocket_request_invalid",
            &error.to_string(),
            false,
        )
    })?;
    for header in request
        .headers
        .iter()
        .filter(|header| forward_request_header(&header.name))
    {
        let name = HeaderName::from_bytes(header.name.as_bytes()).map_err(|error| {
            relay_error(
                "display_websocket_header_invalid",
                &error.to_string(),
                false,
            )
        })?;
        let value = HeaderValue::from_str(&header.value).map_err(|error| {
            relay_error(
                "display_websocket_header_invalid",
                &error.to_string(),
                false,
            )
        })?;
        local_request.headers_mut().append(name, value);
    }
    let (local_socket, _) = tokio_tungstenite::connect_async(local_request)
        .await
        .map_err(|error| {
            relay_error("display_websocket_connect_failed", &error.to_string(), true)
        })?;
    send_outgoing_envelope(
        outgoing_tx,
        RelayEnvelope::DaemonDisplayTunnelResponseStart {
            response: RelayDisplayTunnelResponseStart {
                stream_id: request.stream_id.clone(),
                status: 101,
                headers: Vec::new(),
            },
        },
    )
    .map_err(|error| relay_error("display_websocket_start_failed", &error.to_string(), true))?;
    let (mut local_write, mut local_read) = local_socket.split();
    loop {
        tokio::select! {
            local_message = local_read.next() => {
                match local_message {
                    Some(Ok(Message::Binary(data))) => {
                        send_display_chunk(outgoing_tx, &request.stream_id, data.as_ref(), Some("binary"))?;
                    }
                    Some(Ok(Message::Text(data))) => {
                        send_display_chunk(outgoing_tx, &request.stream_id, data.as_str().as_bytes(), Some("text"))?;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(data))) => {
                        let _ = local_write.send(Message::Pong(data)).await;
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Frame(_))) => {}
                    Some(Err(error)) => {
                        return Err(relay_error("display_websocket_read_failed", &error.to_string(), true));
                    }
                }
            }
            client_event = client_rx.recv() => {
                match client_event {
                    Some(RelayDisplayTunnelClientEvent::Chunk(chunk)) => {
                        let decoded = BASE64_STANDARD
                            .decode(chunk.data.as_bytes())
                            .map_err(|error| relay_error("display_websocket_chunk_invalid", &error.to_string(), false))?;
                        let message = match chunk.message_kind.as_deref() {
                            Some("text") => Message::Text(String::from_utf8_lossy(&decoded).to_string().into()),
                            _ => Message::Binary(decoded.into()),
                        };
                        local_write
                            .send(message)
                            .await
                            .map_err(|error| relay_error("display_websocket_write_failed", &error.to_string(), true))?;
                    }
                    Some(RelayDisplayTunnelClientEvent::Close) | None => {
                        let _ = local_write.close().await;
                        break;
                    }
                }
            }
        }
    }
    Ok(())
}

fn send_display_chunk(
    outgoing_tx: &RelayOutgoingSender,
    stream_id: &str,
    data: &[u8],
    message_kind: Option<&str>,
) -> Result<(), RelayError> {
    send_outgoing_envelope(
        outgoing_tx,
        RelayEnvelope::DaemonDisplayTunnelChunk {
            chunk: RelayDisplayTunnelStreamChunk {
                stream_id: stream_id.to_string(),
                data: BASE64_STANDARD.encode(data),
                message_kind: message_kind.map(|value| value.to_string()),
            },
        },
    )
    .map_err(|error| {
        relay_error(
            "display_websocket_chunk_send_failed",
            &error.to_string(),
            true,
        )
    })
}

fn local_display_url(
    target: &RelayDisplayTunnelTarget,
    display_path: &str,
) -> Result<url::Url, RelayError> {
    let local_base_url = target.kind.local_base_url().ok_or_else(|| {
        relay_error(
            "display_target_invalid",
            "display target is not an HTTP proxy",
            false,
        )
    })?;
    let mut base = url::Url::parse(local_base_url)
        .map_err(|error| relay_error("display_target_invalid", &error.to_string(), false))?;
    let prefix = format!("/display/{}", target.tunnel_id);
    let path_and_query = display_path
        .strip_prefix(prefix.as_str())
        .filter(|value| value.is_empty() || value.starts_with('/') || value.starts_with('?'))
        .ok_or_else(|| {
            relay_error(
                "display_path_invalid",
                "display request path does not match the registered tunnel",
                false,
            )
        })?;
    let local_path = if path_and_query.is_empty() {
        "/"
    } else {
        path_and_query
    };
    let (path, query) = local_path
        .split_once('?')
        .map(|(path, query)| (path, Some(query)))
        .unwrap_or((local_path, None));
    base.set_path(if path.is_empty() { "/" } else { path });
    base.set_query(query);
    Ok(base)
}

fn local_display_websocket_url(
    target: &RelayDisplayTunnelTarget,
    display_path: &str,
) -> Result<url::Url, RelayError> {
    let mut url = local_display_url(target, display_path)?;
    match url.scheme() {
        "http" => url.set_scheme("ws").map_err(|_| {
            relay_error(
                "display_websocket_url_invalid",
                "invalid websocket url",
                false,
            )
        })?,
        "https" => url.set_scheme("wss").map_err(|_| {
            relay_error(
                "display_websocket_url_invalid",
                "invalid websocket url",
                false,
            )
        })?,
        _ => {
            return Err(relay_error(
                "display_websocket_url_invalid",
                "display websocket target must be http or https",
                false,
            ));
        }
    }
    Ok(url)
}

fn display_request_is_websocket(request: &RelayDisplayTunnelOpenRequest) -> bool {
    header_value(&request.headers, "upgrade")
        .is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
        && header_value(&request.headers, "connection").is_some_and(|value| {
            value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("upgrade"))
        })
}

fn header_value<'a>(headers: &'a [RelayDisplayTunnelHeader], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|header| header.name.eq_ignore_ascii_case(name))
        .map(|header| header.value.as_str())
}

fn stream_response_to_proxy(
    outgoing_tx: &RelayOutgoingSender,
    request: &RelayDisplayTunnelOpenRequest,
    response: ureq::Response,
) -> Result<(), RelayError> {
    let status = response.status();
    let headers = response
        .headers_names()
        .into_iter()
        .filter(|name| forward_response_header(name))
        .filter_map(|name| {
            response
                .header(&name)
                .map(|value| RelayDisplayTunnelHeader {
                    name,
                    value: value.to_string(),
                })
        })
        .collect::<Vec<_>>();
    send_outgoing_envelope(
        outgoing_tx,
        RelayEnvelope::DaemonDisplayTunnelResponseStart {
            response: RelayDisplayTunnelResponseStart {
                stream_id: request.stream_id.clone(),
                status,
                headers,
            },
        },
    )
    .map_err(|error| relay_error("display_proxy_start_send_failed", &error.to_string(), true))?;
    let mut reader = response.into_reader();
    let mut total = 0_u64;
    let mut buffer = [0_u8; DISPLAY_PROXY_CHUNK_BYTES];
    loop {
        let size = reader
            .read(&mut buffer)
            .map_err(|error| relay_error("display_proxy_read_failed", &error.to_string(), true))?;
        if size == 0 {
            break;
        }
        total = total.saturating_add(size as u64);
        if total > DISPLAY_PROXY_MAX_RESPONSE_BYTES {
            return Err(relay_error(
                "display_proxy_response_too_large",
                "display proxy response exceeded maximum size",
                false,
            ));
        }
        send_display_chunk(outgoing_tx, &request.stream_id, &buffer[..size], None)?;
    }
    Ok(())
}

fn forward_request_header(name: &str) -> bool {
    !matches!(
        name.to_ascii_lowercase().as_str(),
        "host"
            | "connection"
            | "content-length"
            | "upgrade"
            | "sec-websocket-key"
            | "sec-websocket-version"
            | "sec-websocket-extensions"
    )
}

fn forward_response_header(name: &str) -> bool {
    !matches!(
        name.to_ascii_lowercase().as_str(),
        "connection" | "transfer-encoding" | "content-length"
    )
}

fn relay_error(code: &str, message: &str, retryable: bool) -> RelayError {
    RelayError {
        code: code.to_string(),
        message: message.to_string(),
        retryable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tokio::net::TcpListener;
    use tokio::time::{timeout, Duration};

    #[tokio::test]
    async fn admitted_selkies_target_uses_encrypted_single_use_relay_stream() {
        let kernel_private = crate::transport::relay_crypto::generate_private_key_base64();
        let kernel_public =
            crate::transport::relay_crypto::public_key_from_private_key_base64(&kernel_private)
                .expect("kernel public key should derive");
        let viewer_private = crate::transport::relay_crypto::generate_private_key_base64();
        let viewer_public =
            crate::transport::relay_crypto::public_key_from_private_key_base64(&viewer_private)
                .expect("viewer public key should derive");
        let frame = [4_u8, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0];
        let script = format!(
            "printf '%s\\n' '{{\"kind\":\"ready\",\"protocol\":\"selkies-stdio-v1\",\"read_only\":true}}' '{{\"kind\":\"binary\",\"data_base64\":\"{}\"}}'; while IFS= read -r line; do :; done",
            BASE64_STANDARD.encode(frame)
        );
        let mut state = RelayClientState::default();
        state.upsert_display_tunnel(RelayDisplayTunnelTarget {
            tunnel_id: "display-secure".to_string(),
            slice_id: "slice-1".to_string(),
            kind: RelayDisplayTunnelTargetKind::Selkies {
                viewer_public_key: viewer_public,
                command_program: "/bin/sh".to_string(),
                command_args: vec!["-c".to_string(), script],
            },
            expires_at_ms: crate::session::unix_epoch_ms().saturating_add(30_000),
            capabilities: vec!["view".to_string(), "encrypted".to_string()],
        });
        let state = Arc::new(RwLock::new(state));
        let (outgoing_tx, mut priority_rx, mut event_rx) = RelayOutgoingSender::channel(16);
        let request = RelayDisplayTunnelOpenRequest {
            stream_id: "relay-stream-1".to_string(),
            tunnel_id: "display-secure".to_string(),
            method: "GET".to_string(),
            path: "/display/display-secure/stream".to_string(),
            headers: vec![
                RelayDisplayTunnelHeader {
                    name: "connection".to_string(),
                    value: "Upgrade".to_string(),
                },
                RelayDisplayTunnelHeader {
                    name: "upgrade".to_string(),
                    value: "websocket".to_string(),
                },
            ],
            body_base64: None,
        };
        let handle = tokio::spawn(handle_display_tunnel_open(
            Arc::clone(&state),
            outgoing_tx,
            request,
            kernel_private,
        ));
        assert!(matches!(
            timeout(Duration::from_secs(2), priority_rx.recv())
                .await
                .expect("Selkies response start should arrive"),
            Some(RelayEnvelope::DaemonDisplayTunnelResponseStart { response })
                if response.status == 101 && response.stream_id == "relay-stream-1"
        ));
        let mut viewer = crate::transport::secure_display::SecureDisplayChannel::new(
            viewer_private,
            kernel_public,
            "display-secure",
            crate::transport::secure_display::DisplayPeer::Viewer,
        )
        .expect("viewer cipher should initialize");
        let received = timeout(Duration::from_secs(2), async {
            loop {
                let envelope = event_rx
                    .recv()
                    .await
                    .expect("encrypted Selkies frame should arrive");
                let RelayEnvelope::DaemonDisplayTunnelChunk { chunk } = envelope else {
                    continue;
                };
                assert_eq!(chunk.message_kind.as_deref(), Some("binary"));
                let wire = BASE64_STANDARD
                    .decode(chunk.data)
                    .expect("relay chunk should decode");
                assert!(!wire.windows(frame.len()).any(|window| window == frame));
                let packet =
                    serde_json::from_slice(&wire).expect("encrypted display payload should decode");
                if let Some(message) = viewer.decode(&packet).expect("frame should decrypt") {
                    break message.data;
                }
            }
        })
        .await
        .expect("encrypted Selkies frame should not stall");
        assert_eq!(received, frame);
        let sender = state
            .read()
            .await
            .display_stream_sender("relay-stream-1")
            .expect("active display stream should be registered");
        sender
            .send(RelayDisplayTunnelClientEvent::Close)
            .await
            .expect("viewer close should be delivered");
        timeout(Duration::from_secs(10), handle)
            .await
            .expect("Selkies handler cleanup should be bounded")
            .expect("Selkies handler should finish");
        assert!(state
            .read()
            .await
            .display_tunnel("display-secure", crate::session::unix_epoch_ms())
            .is_none());
        assert!(matches!(
            priority_rx.try_recv(),
            Ok(RelayEnvelope::DaemonDisplayTunnelRevoke { tunnel_id })
                if tunnel_id == "display-secure"
        ));
    }

    #[tokio::test]
    async fn admitted_selkies_stream_outlives_the_one_time_opening_grant() {
        let kernel_private = crate::transport::relay_crypto::generate_private_key_base64();
        let viewer_private = crate::transport::relay_crypto::generate_private_key_base64();
        let viewer_public =
            crate::transport::relay_crypto::public_key_from_private_key_base64(&viewer_private)
                .expect("viewer public key should derive");
        let frame = [4_u8, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0];
        let script = format!(
            "printf '%s\\n' '{{\"kind\":\"ready\",\"protocol\":\"selkies-stdio-v1\",\"read_only\":true}}'; sleep 1.2; printf '%s\\n' '{{\"kind\":\"binary\",\"data_base64\":\"{}\"}}'; while IFS= read -r line; do :; done",
            BASE64_STANDARD.encode(frame)
        );
        let mut state = RelayClientState::default();
        state.upsert_display_tunnel(RelayDisplayTunnelTarget {
            tunnel_id: "display-short-grant".to_string(),
            slice_id: "slice-1".to_string(),
            kind: RelayDisplayTunnelTargetKind::Selkies {
                viewer_public_key: viewer_public,
                command_program: "/bin/sh".to_string(),
                command_args: vec!["-c".to_string(), script],
            },
            expires_at_ms: crate::session::unix_epoch_ms().saturating_add(1_000),
            capabilities: vec!["view".to_string(), "encrypted".to_string()],
        });
        let state = Arc::new(RwLock::new(state));
        let (outgoing_tx, mut priority_rx, mut event_rx) = RelayOutgoingSender::channel(16);
        let handle = tokio::spawn(handle_display_tunnel_open(
            Arc::clone(&state),
            outgoing_tx,
            RelayDisplayTunnelOpenRequest {
                stream_id: "relay-stream-short-grant".to_string(),
                tunnel_id: "display-short-grant".to_string(),
                method: "GET".to_string(),
                path: "/display/display-short-grant/stream".to_string(),
                headers: vec![
                    RelayDisplayTunnelHeader {
                        name: "connection".to_string(),
                        value: "Upgrade".to_string(),
                    },
                    RelayDisplayTunnelHeader {
                        name: "upgrade".to_string(),
                        value: "websocket".to_string(),
                    },
                ],
                body_base64: None,
            },
            kernel_private,
        ));
        assert!(matches!(
            timeout(Duration::from_secs(2), priority_rx.recv())
                .await
                .expect("Selkies response start should arrive"),
            Some(RelayEnvelope::DaemonDisplayTunnelResponseStart { response })
                if response.status == 101
        ));
        assert!(matches!(
            timeout(Duration::from_secs(2), event_rx.recv())
                .await
                .expect("the admitted stream should remain live after its opening grant expires"),
            Some(RelayEnvelope::DaemonDisplayTunnelChunk { chunk })
                if chunk.stream_id == "relay-stream-short-grant"
        ));
        let sender = state
            .read()
            .await
            .display_stream_sender("relay-stream-short-grant")
            .expect("active display stream should be registered");
        sender
            .send(RelayDisplayTunnelClientEvent::Close)
            .await
            .expect("viewer close should be delivered");
        timeout(Duration::from_secs(10), handle)
            .await
            .expect("Selkies handler cleanup should be bounded")
            .expect("Selkies handler should finish");
    }

    #[test]
    fn local_display_url_rewrites_relay_display_path_to_local_origin() {
        let target = RelayDisplayTunnelTarget {
            tunnel_id: "display-1".to_string(),
            slice_id: "slice-1".to_string(),
            kind: RelayDisplayTunnelTargetKind::HttpProxy {
                local_base_url: "http://127.0.0.1:5901".to_string(),
            },
            expires_at_ms: u64::MAX,
            capabilities: vec!["view".to_string()],
        };

        let url = local_display_url(&target, "/display/display-1/vnc.html?autoconnect=true")
            .expect("url should rewrite");

        assert_eq!(
            url.as_str(),
            "http://127.0.0.1:5901/vnc.html?autoconnect=true"
        );
        assert!(local_display_url(&target, "/display/other/vnc.html").is_err());
    }

    #[test]
    fn local_display_websocket_url_rewrites_to_ws_target() {
        let target = RelayDisplayTunnelTarget {
            tunnel_id: "display-1".to_string(),
            slice_id: "slice-1".to_string(),
            kind: RelayDisplayTunnelTargetKind::HttpProxy {
                local_base_url: "http://127.0.0.1:5901".to_string(),
            },
            expires_at_ms: u64::MAX,
            capabilities: vec!["view".to_string(), "websocket".to_string()],
        };

        let url = local_display_websocket_url(&target, "/display/display-1/websockify")
            .expect("url should rewrite");

        assert_eq!(url.as_str(), "ws://127.0.0.1:5901/websockify");
    }

    #[test]
    fn display_request_is_websocket_requires_upgrade_headers() {
        let request = RelayDisplayTunnelOpenRequest {
            stream_id: "stream-1".to_string(),
            tunnel_id: "display-1".to_string(),
            method: "GET".to_string(),
            path: "/display/display-1/websockify".to_string(),
            headers: vec![
                RelayDisplayTunnelHeader {
                    name: "connection".to_string(),
                    value: "keep-alive, Upgrade".to_string(),
                },
                RelayDisplayTunnelHeader {
                    name: "upgrade".to_string(),
                    value: "websocket".to_string(),
                },
            ],
            body_base64: None,
        };

        assert!(display_request_is_websocket(&request));
    }

    #[tokio::test]
    async fn display_websocket_proxy_bridges_local_socket_and_relay_chunks() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("local websocket listener should bind");
        let addr = listener
            .local_addr()
            .expect("local websocket listener should have addr");
        let local_task = tokio::spawn(async move {
            let (stream, _) = listener
                .accept()
                .await
                .expect("local websocket should accept");
            let mut socket = tokio_tungstenite::accept_hdr_async(
                stream,
                |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                    assert_eq!(
                        request
                            .headers()
                            .get("x-chariox-caller-claims")
                            .and_then(|value| value.to_str().ok()),
                        Some("signed-caller-claims")
                    );
                    assert_eq!(
                        request
                            .headers()
                            .get("x-chariox-invocation-id")
                            .and_then(|value| value.to_str().ok()),
                        Some("invocation-1")
                    );
                    Ok(response)
                },
            )
            .await
            .expect("local websocket handshake should complete");
            match socket.next().await {
                Some(Ok(Message::Binary(data))) => assert_eq!(data.as_ref(), b"from-browser"),
                other => panic!("unexpected local websocket input: {other:?}"),
            }
            socket
                .send(Message::Binary(Vec::from("from-local").into()))
                .await
                .expect("local websocket output should send");
            let _ = socket.close(None).await;
        });

        let mut state = RelayClientState::default();
        state.upsert_display_tunnel(RelayDisplayTunnelTarget {
            tunnel_id: "display-1".to_string(),
            slice_id: "slice-1".to_string(),
            kind: RelayDisplayTunnelTargetKind::HttpProxy {
                local_base_url: format!("http://{addr}"),
            },
            expires_at_ms: u64::MAX,
            capabilities: vec!["http".to_string()],
        });
        let state = Arc::new(RwLock::new(state));
        let (outgoing_tx, mut priority_rx, mut event_rx) = RelayOutgoingSender::channel(16);
        let request = RelayDisplayTunnelOpenRequest {
            stream_id: "stream-1".to_string(),
            tunnel_id: "display-1".to_string(),
            method: "GET".to_string(),
            path: "/display/display-1/websockify".to_string(),
            headers: vec![
                RelayDisplayTunnelHeader {
                    name: "connection".to_string(),
                    value: "Upgrade".to_string(),
                },
                RelayDisplayTunnelHeader {
                    name: "upgrade".to_string(),
                    value: "websocket".to_string(),
                },
                RelayDisplayTunnelHeader {
                    name: "x-chariox-caller-claims".to_string(),
                    value: "signed-caller-claims".to_string(),
                },
                RelayDisplayTunnelHeader {
                    name: "x-chariox-invocation-id".to_string(),
                    value: "invocation-1".to_string(),
                },
            ],
            body_base64: None,
        };
        let handle = tokio::spawn(handle_display_tunnel_open(
            Arc::clone(&state),
            outgoing_tx,
            request,
            crate::transport::relay_crypto::generate_private_key_base64(),
        ));

        match timeout(Duration::from_secs(2), priority_rx.recv())
            .await
            .expect("response start should arrive")
        {
            Some(RelayEnvelope::DaemonDisplayTunnelResponseStart { response }) => {
                assert_eq!(response.stream_id, "stream-1");
                assert_eq!(response.status, 101);
            }
            other => panic!("unexpected display websocket response start: {other:?}"),
        }

        let sender = state
            .read()
            .await
            .display_stream_sender("stream-1")
            .expect("display stream should be registered");
        sender
            .send(RelayDisplayTunnelClientEvent::Chunk(
                RelayDisplayTunnelStreamChunk {
                    stream_id: "stream-1".to_string(),
                    data: BASE64_STANDARD.encode("from-browser"),
                    message_kind: Some("binary".to_string()),
                },
            ))
            .await
            .expect("client chunk should send");

        match timeout(Duration::from_secs(2), event_rx.recv())
            .await
            .expect("daemon chunk should arrive")
        {
            Some(RelayEnvelope::DaemonDisplayTunnelChunk { chunk }) => {
                assert_eq!(chunk.stream_id, "stream-1");
                assert_eq!(chunk.message_kind.as_deref(), Some("binary"));
                assert_eq!(
                    BASE64_STANDARD
                        .decode(chunk.data)
                        .expect("display chunk should decode"),
                    b"from-local"
                );
            }
            other => panic!("unexpected display websocket chunk: {other:?}"),
        }

        handle.await.expect("display websocket proxy should finish");
        local_task
            .await
            .expect("local websocket task should finish");
    }

    #[tokio::test]
    async fn display_http_proxy_forwards_post_body_and_streams_response_chunks() {
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("local http listener should bind");
        let addr = listener
            .local_addr()
            .expect("local http listener should have addr");
        let local_task = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("local http should accept");
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            loop {
                let size = stream.read(&mut buffer).expect("request should read");
                if size == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..size]);
                if request.ends_with(b"{\"prompt\":\"hi\"}") {
                    break;
                }
            }
            let text = String::from_utf8_lossy(&request);
            assert!(text.starts_with("POST /invoke HTTP/1.1"), "{text}");
            assert!(text.contains("content-type: application/json"), "{text}");
            assert!(text.ends_with("{\"prompt\":\"hi\"}"), "{text}");
            stream
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\n")
                .expect("response headers should write");
            stream
                .write_all(b"event: queued\ndata: {}\n\n")
                .expect("first response chunk should write");
            stream.flush().expect("first response chunk should flush");
            std::thread::sleep(std::time::Duration::from_millis(100));
            stream
                .write_all(b"event: final\ndata: {\"value\":1842}\n\n")
                .expect("second response chunk should write");
        });

        let mut state = RelayClientState::default();
        state.upsert_display_tunnel(RelayDisplayTunnelTarget {
            tunnel_id: "display-1".to_string(),
            slice_id: "publication-1".to_string(),
            kind: RelayDisplayTunnelTargetKind::HttpProxy {
                local_base_url: format!("http://{addr}"),
            },
            expires_at_ms: u64::MAX,
            capabilities: vec!["http".to_string(), "publication".to_string()],
        });
        let state = Arc::new(RwLock::new(state));
        let (outgoing_tx, mut priority_rx, mut event_rx) = RelayOutgoingSender::channel(16);
        let request = RelayDisplayTunnelOpenRequest {
            stream_id: "stream-1".to_string(),
            tunnel_id: "display-1".to_string(),
            method: "POST".to_string(),
            path: "/display/display-1/invoke".to_string(),
            headers: vec![RelayDisplayTunnelHeader {
                name: "content-type".to_string(),
                value: "application/json".to_string(),
            }],
            body_base64: Some(BASE64_STANDARD.encode("{\"prompt\":\"hi\"}")),
        };
        tokio::spawn(handle_display_tunnel_open(
            Arc::clone(&state),
            outgoing_tx,
            request,
            crate::transport::relay_crypto::generate_private_key_base64(),
        ));

        match timeout(Duration::from_secs(2), priority_rx.recv())
            .await
            .expect("response start should arrive")
        {
            Some(RelayEnvelope::DaemonDisplayTunnelResponseStart { response }) => {
                assert_eq!(response.stream_id, "stream-1");
                assert_eq!(response.status, 200);
                assert_eq!(response.headers[0].name, "content-type");
                assert_eq!(response.headers[0].value, "text/event-stream");
            }
            other => panic!("unexpected display response start: {other:?}"),
        }

        let mut decoded_chunks = Vec::new();
        loop {
            match timeout(Duration::from_secs(2), event_rx.recv())
                .await
                .expect("display chunk should arrive")
            {
                Some(RelayEnvelope::DaemonDisplayTunnelChunk { chunk }) => {
                    decoded_chunks.extend(
                        BASE64_STANDARD
                            .decode(chunk.data)
                            .expect("display chunk should decode"),
                    );
                    if String::from_utf8_lossy(&decoded_chunks).contains("event: final") {
                        break;
                    }
                }
                other => match other {
                    None => panic!("display event lane closed before final chunk"),
                    Some(other) => panic!("unexpected display stream envelope: {other:?}"),
                },
            }
        }
        match timeout(Duration::from_secs(2), event_rx.recv())
            .await
            .expect("display close should arrive")
        {
            Some(RelayEnvelope::DaemonDisplayTunnelClose { error, .. }) => {
                assert_eq!(error, None);
            }
            other => panic!("unexpected display close envelope: {other:?}"),
        }
        let body = String::from_utf8(decoded_chunks).expect("response chunks should be utf8");
        assert!(body.contains("event: queued"), "{body}");
        assert!(body.contains("event: final"), "{body}");
        local_task.join().expect("local http task should finish");
    }

    #[tokio::test]
    async fn display_http_proxy_answers_optional_package_probe_with_valid_json() {
        let mut state = RelayClientState::default();
        state.upsert_display_tunnel(RelayDisplayTunnelTarget {
            tunnel_id: "display-1".to_string(),
            slice_id: "slice-1".to_string(),
            kind: RelayDisplayTunnelTargetKind::HttpProxy {
                local_base_url: "http://127.0.0.1:1".to_string(),
            },
            expires_at_ms: u64::MAX,
            capabilities: vec!["http".to_string()],
        });
        let state = Arc::new(RwLock::new(state));
        let (outgoing_tx, mut priority_rx, mut event_rx) = RelayOutgoingSender::channel(16);
        let request = RelayDisplayTunnelOpenRequest {
            stream_id: "stream-1".to_string(),
            tunnel_id: "display-1".to_string(),
            method: "GET".to_string(),
            path: "/display/display-1/package.json".to_string(),
            headers: Vec::new(),
            body_base64: None,
        };
        tokio::spawn(handle_display_tunnel_open(
            Arc::clone(&state),
            outgoing_tx,
            request,
            crate::transport::relay_crypto::generate_private_key_base64(),
        ));

        match timeout(Duration::from_secs(2), priority_rx.recv())
            .await
            .expect("response start should arrive")
        {
            Some(RelayEnvelope::DaemonDisplayTunnelResponseStart { response }) => {
                assert_eq!(response.stream_id, "stream-1");
                assert_eq!(response.status, 200);
                assert_eq!(response.headers[0].name, "content-type");
                assert_eq!(response.headers[0].value, "application/json");
            }
            other => panic!("unexpected display response start: {other:?}"),
        }

        match timeout(Duration::from_secs(2), event_rx.recv())
            .await
            .expect("display body should arrive")
        {
            Some(RelayEnvelope::DaemonDisplayTunnelChunk { chunk }) => {
                assert_eq!(
                    BASE64_STANDARD
                        .decode(chunk.data)
                        .expect("display chunk should decode"),
                    b"{}"
                );
            }
            other => panic!("unexpected display body: {other:?}"),
        }

        match timeout(Duration::from_secs(2), event_rx.recv())
            .await
            .expect("display close should arrive")
        {
            Some(RelayEnvelope::DaemonDisplayTunnelClose { stream_id, error }) => {
                assert_eq!(stream_id, "stream-1");
                assert_eq!(error, None);
            }
            other => panic!("unexpected display close: {other:?}"),
        }
    }
}
