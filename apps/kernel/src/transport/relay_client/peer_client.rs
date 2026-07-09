//! Outbound relay peer requests and pending response correlation.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};

use super::*;

static TEMPORARY_PEER_REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Leased prompt submission can synchronously start or relaunch the worker
/// provider. Codex may spend up to roughly 180s across MCP handshake retry
/// attempts before it can return the real provider run id, and home must not
/// invent a placeholder run id. Keep this timeout above that provider-start
/// envelope so remote prompt dispatch reports the authoritative worker result.
pub const LEASED_PROMPT_SUBMIT_RESPONSE_TIMEOUT: Duration = Duration::from_secs(240);

#[derive(Debug)]
struct RelayPeerRequestTrace {
    request_id: String,
    target_daemon_id: Option<String>,
    target_daemon_alias: Option<String>,
    transport: &'static str,
    started_at: Instant,
    timeout_ms: u64,
}

impl RelayPeerRequestTrace {
    fn new(
        request_id: &str,
        target: &ClientTarget,
        transport: &'static str,
        timeout_ms: u64,
    ) -> Self {
        Self {
            request_id: request_id.to_string(),
            target_daemon_id: target.daemon_id.clone(),
            target_daemon_alias: target.daemon_alias.clone(),
            transport,
            started_at: Instant::now(),
            timeout_ms,
        }
    }

    fn fields(
        &self,
        status: &'static str,
        error: Option<&str>,
        from_daemon_id: Option<&str>,
    ) -> serde_json::Value {
        serde_json::json!({
            "request_id": self.request_id,
            "target_daemon_id": self.target_daemon_id,
            "target_daemon_alias": self.target_daemon_alias,
            "from_daemon_id": from_daemon_id,
            "transport": self.transport,
            "status": status,
            "relay_rtt_ms": elapsed_ms_u64(self.started_at),
            "timeout_ms": self.timeout_ms,
            "error": error,
        })
    }

    fn log_completed(
        &self,
        status: &'static str,
        error: Option<&str>,
        from_daemon_id: Option<&str>,
    ) {
        let fields = self.fields(status, error, from_daemon_id);
        if error.is_some() {
            crate::logging::warn_with_fields(
                "daemon.relay_client",
                "relay peer request completed",
                fields,
            );
        } else {
            crate::logging::info_with_fields(
                "daemon.relay_client",
                "relay peer request completed",
                fields,
            );
        }
    }
}

fn elapsed_ms_u64(started_at: Instant) -> u64 {
    let elapsed_ms = started_at.elapsed().as_millis();
    elapsed_ms.min(u128::from(u64::MAX)) as u64
}

#[allow(dead_code)]
#[derive(Debug)]
pub(super) struct RelayPeerResponseEnvelope {
    pub(super) from_daemon_id: String,
    pub(super) encrypted_response: Option<EncryptedRelayPayload>,
    pub(super) error: Option<RelayError>,
}

#[cfg(test)]
pub async fn send_peer_request_to_known_kernel_via_relay(
    config: &crate::config::DaemonConfig,
    state: &Arc<RwLock<RelayClientState>>,
    target: ClientTarget,
    target_public_key: &str,
    request: RelayPeerRequest,
) -> Result<RelayPeerResponse, DaemonError> {
    let _target_ref = target
        .daemon_id
        .as_deref()
        .or(target.daemon_alias.as_deref())
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "send relay peer request",
            message: "peer target must include daemon id or alias".to_string(),
        })?;
    let plaintext = serde_json::to_vec(&request).map_err(|error| DaemonError::LocalTransport {
        operation: "serialize relay peer request",
        message: error.to_string(),
    })?;
    let encrypted_request = relay_crypto::encrypt_payload_for_peer(
        &config.relay_private_key,
        target_public_key,
        &plaintext,
    )?;
    let (request_id, response_rx, outgoing_tx) = {
        let mut guard = state.write().await;
        let Some(outgoing_tx) = guard.outgoing_tx.clone() else {
            return Err(DaemonError::LocalTransport {
                operation: "send relay peer request",
                message: "relay is not connected".to_string(),
            });
        };
        guard.next_peer_request_id += 1;
        let request_id = format!("daemon-peer-{}", guard.next_peer_request_id);
        let (response_tx, response_rx) = oneshot::channel();
        guard
            .pending_peer_requests
            .insert(request_id.clone(), response_tx);
        (request_id, response_rx, outgoing_tx)
    };
    let trace = RelayPeerRequestTrace::new(
        &request_id,
        &target,
        "persistent",
        config.relay_request_timeout_ms,
    );
    if send_outgoing_envelope(
        &outgoing_tx,
        RelayEnvelope::DaemonPeerRequest {
            request_id: request_id.clone(),
            target,
            encrypted_request,
        },
    )
    .is_err()
    {
        let mut guard = state.write().await;
        guard.pending_peer_requests.remove(&request_id);
        trace.log_completed("send_failed", Some("relay is not connected"), None);
        return Err(DaemonError::LocalTransport {
            operation: "send relay peer request",
            message: "relay is not connected".to_string(),
        });
    }
    let envelope = match timeout(
        Duration::from_millis(config.relay_request_timeout_ms),
        response_rx,
    )
    .await
    {
        Ok(Ok(envelope)) => envelope,
        Ok(Err(_)) => {
            trace.log_completed("cancelled", Some("relay peer request was cancelled"), None);
            return Err(DaemonError::LocalTransport {
                operation: "read relay peer response",
                message: "relay peer request was cancelled".to_string(),
            });
        }
        Err(_) => {
            let mut guard = state.write().await;
            guard.pending_peer_requests.remove(&request_id);
            let message = format!(
                "timed out waiting for relay peer response after {}ms",
                config.relay_request_timeout_ms
            );
            trace.log_completed("timeout", Some(&message), None);
            return Err(DaemonError::LocalTransport {
                operation: "read relay peer response",
                message,
            });
        }
    };
    if let Some(error) = envelope.error {
        trace.log_completed(
            "relay_error",
            Some(&error.message),
            Some(&envelope.from_daemon_id),
        );
        return Err(DaemonError::LocalTransport {
            operation: "read relay peer response",
            message: error.message,
        });
    }
    let encrypted_response = match envelope.encrypted_response {
        Some(encrypted_response) => encrypted_response,
        None => {
            let message = format!(
                "peer `{}` returned no response payload",
                envelope.from_daemon_id
            );
            trace.log_completed(
                "empty_response",
                Some(&message),
                Some(&envelope.from_daemon_id),
            );
            return Err(DaemonError::LocalTransport {
                operation: "read relay peer response",
                message,
            });
        }
    };
    let decrypted = match relay_crypto::decrypt_payload_for_private_key(
        &config.relay_private_key,
        &encrypted_response,
    ) {
        Ok(decrypted) => decrypted,
        Err(error) => {
            let message = error.to_string();
            trace.log_completed(
                "decrypt_failed",
                Some(&message),
                Some(&envelope.from_daemon_id),
            );
            return Err(error);
        }
    };
    match serde_json::from_slice::<RelayPeerResponse>(&decrypted.plaintext) {
        Ok(response) => {
            trace.log_completed("success", None, Some(&envelope.from_daemon_id));
            Ok(response)
        }
        Err(error) => {
            let message = error.to_string();
            trace.log_completed(
                "decode_failed",
                Some(&message),
                Some(&envelope.from_daemon_id),
            );
            Err(DaemonError::LocalTransport {
                operation: "decode relay peer response",
                message,
            })
        }
    }
}

#[cfg(test)]
pub async fn send_peer_request_via_relay(
    app: &Arc<Mutex<DaemonApp>>,
    state: &Arc<RwLock<RelayClientState>>,
    target: ClientTarget,
    request: RelayPeerRequest,
) -> Result<RelayPeerResponse, DaemonError> {
    let target_ref = target
        .daemon_id
        .as_deref()
        .or(target.daemon_alias.as_deref())
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "send relay peer request",
            message: "peer target must include daemon id or alias".to_string(),
        })?;
    let config = {
        let app = app.lock().await;
        app.config().clone()
    };
    let kernel = relay_discovery::get_live_kernel(&config, target_ref).await?;
    send_peer_request_to_known_kernel_via_relay(&config, state, target, &kernel.public_key, request)
        .await
}

pub async fn send_peer_request_via_temporary_connection(
    config: &crate::config::DaemonConfig,
    target: ClientTarget,
    request: RelayPeerRequest,
) -> Result<RelayPeerResponse, DaemonError> {
    send_peer_request_via_temporary_connection_with_timeout(
        config,
        target,
        request,
        Duration::from_millis(config.relay_request_timeout_ms),
    )
    .await
}

pub async fn send_peer_request_via_temporary_connection_with_timeout(
    config: &crate::config::DaemonConfig,
    target: ClientTarget,
    request: RelayPeerRequest,
    response_timeout: Duration,
) -> Result<RelayPeerResponse, DaemonError> {
    let target_ref = target
        .daemon_id
        .as_deref()
        .or(target.daemon_alias.as_deref())
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "send relay peer request",
            message: "peer target must include daemon id or alias".to_string(),
        })?;
    let relay_url = config
        .relay_url
        .clone()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "send relay peer request",
            message: "relay_url is not configured".to_string(),
        })?;
    let relay_token = config
        .relay_token
        .clone()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "send relay peer request",
            message: "relay_token is not configured".to_string(),
        })?;
    let kernel = relay_discovery::get_live_kernel(config, target_ref).await?;
    let plaintext = serde_json::to_vec(&request).map_err(|error| DaemonError::LocalTransport {
        operation: "serialize relay peer request",
        message: error.to_string(),
    })?;
    let encrypted_request = relay_crypto::encrypt_payload_for_peer(
        &config.relay_private_key,
        &kernel.public_key,
        &plaintext,
    )?;
    let (mut socket, _) = timeout(response_timeout, connect_async(&relay_url))
        .await
        .map_err(|_| DaemonError::LocalTransport {
            operation: "connect temporary relay peer socket",
            message: format!("timed out after {}ms", response_timeout.as_millis()),
        })?
        .map_err(|error| DaemonError::LocalTransport {
            operation: "connect temporary relay peer socket",
            message: error.to_string(),
        })?;
    let request_id = format!(
        "daemon-peer-tmp-{}-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms(),
        TEMPORARY_PEER_REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let register = RelayEnvelope::DaemonRegister {
        registration: arroba_relay::protocol::DaemonRegistration {
            auth_token: relay_token,
            daemon_id: format!("{}:peer-tmp:{}", config.daemon_id, request_id),
            machine_id: config.host_machine_id.clone(),
            machine_alias: config.host_machine_alias.clone(),
            os_name: Some(config.os_name.clone()),
            kernel_started_at_ms: crate::session::unix_epoch_ms(),
            daemon_alias: None,
            kernel_alias: None,
            public_key: config.relay_public_key.clone(),
            capabilities: vec!["relay_peer_transport".to_string()],
            available_providers: Vec::new(),
            provider_accounts: Vec::new(),
            accepting_remote_leases: false,
            leased_agent_count: 0,
            local_session_count: 0,
        },
    };
    socket
        .send(Message::Text(
            serde_json::to_string(&register)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "serialize temporary relay register",
                    message: error.to_string(),
                })?
                .into(),
        ))
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "write temporary relay register",
            message: error.to_string(),
        })?;
    let response_timeout_ms = u64::try_from(response_timeout.as_millis()).unwrap_or(u64::MAX);
    let trace = RelayPeerRequestTrace::new(&request_id, &target, "temporary", response_timeout_ms);
    socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonPeerRequest {
                request_id: request_id.clone(),
                target,
                encrypted_request,
            })
            .map_err(|error| {
                let message = error.to_string();
                trace.log_completed("serialize_failed", Some(&message), None);
                DaemonError::LocalTransport {
                    operation: "serialize temporary relay peer request",
                    message,
                }
            })?
            .into(),
        ))
        .await
        .map_err(|error| {
            let message = error.to_string();
            trace.log_completed("send_failed", Some(&message), None);
            DaemonError::LocalTransport {
                operation: "write temporary relay peer request",
                message,
            }
        })?;
    let response = timeout(response_timeout, async {
        loop {
            match socket.next().await {
                Some(Ok(Message::Text(text))) => {
                    let envelope =
                        serde_json::from_str::<RelayEnvelope>(&text).map_err(|error| {
                            let message = error.to_string();
                            trace.log_completed("decode_failed", Some(&message), None);
                            DaemonError::LocalTransport {
                                operation: "decode temporary relay peer response",
                                message,
                            }
                        })?;
                    if let RelayEnvelope::DaemonPeerResponse {
                        request_id: response_request_id,
                        from_daemon_id,
                        encrypted_response,
                        error,
                    } = envelope
                    {
                        if response_request_id != request_id {
                            continue;
                        }
                        if let Some(error) = error {
                            trace.log_completed(
                                "relay_error",
                                Some(&error.message),
                                Some(&from_daemon_id),
                            );
                            return Err(DaemonError::LocalTransport {
                                operation: "read temporary relay peer response",
                                message: error.message,
                            });
                        }
                        let encrypted_response = encrypted_response.ok_or_else(|| {
                            let message = "peer returned no response payload".to_string();
                            trace.log_completed(
                                "empty_response",
                                Some(&message),
                                Some(&from_daemon_id),
                            );
                            DaemonError::LocalTransport {
                                operation: "read temporary relay peer response",
                                message,
                            }
                        })?;
                        let decrypted = match relay_crypto::decrypt_payload_for_private_key(
                            &config.relay_private_key,
                            &encrypted_response,
                        ) {
                            Ok(decrypted) => decrypted,
                            Err(error) => {
                                let message = error.to_string();
                                trace.log_completed(
                                    "decrypt_failed",
                                    Some(&message),
                                    Some(&from_daemon_id),
                                );
                                return Err(error);
                            }
                        };
                        let response =
                            serde_json::from_slice::<RelayPeerResponse>(&decrypted.plaintext);
                        return match response {
                            Ok(response) => {
                                trace.log_completed("success", None, Some(&from_daemon_id));
                                Ok(response)
                            }
                            Err(error) => {
                                let message = error.to_string();
                                trace.log_completed(
                                    "decode_failed",
                                    Some(&message),
                                    Some(&from_daemon_id),
                                );
                                Err(DaemonError::LocalTransport {
                                    operation: "decode temporary relay peer response",
                                    message,
                                })
                            }
                        };
                    }
                }
                Some(Ok(Message::Close(_))) | None => {
                    trace.log_completed(
                        "connection_closed",
                        Some("relay closed temporary peer connection"),
                        None,
                    );
                    return Err(DaemonError::LocalTransport {
                        operation: "read temporary relay peer response",
                        message: "relay closed temporary peer connection".to_string(),
                    });
                }
                Some(Ok(_)) => {}
                Some(Err(error)) => {
                    let message = error.to_string();
                    trace.log_completed("read_failed", Some(&message), None);
                    return Err(DaemonError::LocalTransport {
                        operation: "read temporary relay peer response",
                        message,
                    });
                }
            }
        }
    })
    .await;
    let result = response.unwrap_or_else(|_| {
        let message = format!(
            "timed out waiting for relay peer response after {}ms",
            response_timeout_ms
        );
        trace.log_completed("timeout", Some(&message), None);
        Err(DaemonError::LocalTransport {
            operation: "read temporary relay peer response",
            message,
        })
    });
    let _ = socket.close(None).await;
    let _ = timeout(Duration::from_millis(250), async {
        while let Some(message) = socket.next().await {
            match message {
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {}
            }
        }
    })
    .await;
    result
}

pub(super) async fn resolve_pending_peer_response(
    state: &Arc<RwLock<RelayClientState>>,
    request_id: String,
    response: RelayPeerResponseEnvelope,
) {
    let sender = state
        .write()
        .await
        .pending_peer_requests
        .remove(&request_id);
    if let Some(sender) = sender {
        let _ = sender.send(response);
    }
}

#[cfg(test)]
mod relay_rtt_tests {
    use super::*;

    #[test]
    fn relay_peer_request_trace_fields_include_rtt_and_target() {
        let trace = RelayPeerRequestTrace::new(
            "request-1",
            &ClientTarget {
                daemon_id: Some("daemon-1".to_string()),
                daemon_alias: Some("worker".to_string()),
            },
            "persistent",
            250,
        );

        let fields = trace.fields("success", None, Some("daemon-1"));

        assert_eq!(fields["request_id"], serde_json::json!("request-1"));
        assert_eq!(fields["target_daemon_id"], serde_json::json!("daemon-1"));
        assert_eq!(fields["target_daemon_alias"], serde_json::json!("worker"));
        assert_eq!(fields["from_daemon_id"], serde_json::json!("daemon-1"));
        assert_eq!(fields["transport"], serde_json::json!("persistent"));
        assert_eq!(fields["status"], serde_json::json!("success"));
        assert_eq!(fields["timeout_ms"], serde_json::json!(250));
        assert!(fields["relay_rtt_ms"].as_u64().is_some());
        assert!(fields["error"].is_null());
    }

    #[test]
    fn relay_peer_request_trace_fields_include_errors() {
        let trace = RelayPeerRequestTrace::new(
            "request-2",
            &ClientTarget {
                daemon_id: None,
                daemon_alias: Some("worker".to_string()),
            },
            "temporary",
            500,
        );

        let fields = trace.fields("timeout", Some("timed out"), None);

        assert_eq!(fields["status"], serde_json::json!("timeout"));
        assert_eq!(fields["target_daemon_id"], serde_json::Value::Null);
        assert_eq!(fields["target_daemon_alias"], serde_json::json!("worker"));
        assert_eq!(fields["transport"], serde_json::json!("temporary"));
        assert_eq!(fields["error"], serde_json::json!("timed out"));
    }
}
