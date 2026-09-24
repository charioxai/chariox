use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use sha1::{Digest, Sha1};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, RwLock};
use tokio_tungstenite::tungstenite::protocol::Role;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

use crate::protocol::{
    RelayDisplayTunnelHeader, RelayDisplayTunnelOpenRequest, RelayEnvelope, RelayError,
};
use crate::registry::{DisplayStreamEvent, DisplayTunnelLookup, RelayRegistry, RelaySender};

const DISPLAY_PEEK_TIMEOUT: Duration = Duration::from_millis(250);
const DISPLAY_REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(5);
const DISPLAY_RESPONSE_START_TIMEOUT: Duration = Duration::from_secs(300);
const DISPLAY_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
const DISPLAY_WEBSOCKET_STREAM_IDLE_TIMEOUT: Option<Duration> = None;
const DISPLAY_MAX_HEADER_BYTES: usize = 16 * 1024;
const DISPLAY_MAX_BODY_BYTES: usize = 16 * 1024 * 1024;
const DISPLAY_STREAM_QUEUE_CAPACITY: usize = 128;
const ENCRYPTED_DISPLAY_STREAM_QUEUE_CAPACITY: usize = 16;

pub(crate) async fn is_display_http_request(stream: &TcpStream) -> bool {
    let mut buffer = [0_u8; 512];
    let Ok(Ok(size)) = tokio::time::timeout(DISPLAY_PEEK_TIMEOUT, stream.peek(&mut buffer)).await
    else {
        return false;
    };
    display_request_path(&buffer[..size]).is_some()
}

pub(crate) async fn handle_display_connection(
    mut stream: TcpStream,
    _peer_addr: SocketAddr,
    registry: Arc<RwLock<RelayRegistry>>,
    relay_request_counter: Arc<AtomicU64>,
) -> Result<(), std::io::Error> {
    let request = match read_display_request(&mut stream).await {
        Ok(request) => request,
        Err(status) => {
            write_response(&mut stream, status, "display request is invalid").await?;
            return Ok(());
        }
    };
    let Some(tunnel_id) = display_tunnel_id(&request.path) else {
        write_response(&mut stream, 404, "display tunnel not found").await?;
        return Ok(());
    };
    let tunnel_id = tunnel_id.to_string();
    let lookup = {
        let guard = registry.read().await;
        guard.display_tunnel_lookup(&tunnel_id, current_unix_ms())
    };
    match lookup {
        DisplayTunnelLookup::Missing => {
            write_response(&mut stream, 404, "display tunnel not found").await?;
        }
        DisplayTunnelLookup::Expired => {
            write_response(&mut stream, 410, "display tunnel expired").await?;
        }
        DisplayTunnelLookup::Active {
            daemon_key,
            daemon_sender,
            capabilities,
        } => {
            let Some(daemon_sender) = daemon_sender else {
                write_response(&mut stream, 502, "display tunnel daemon is disconnected").await?;
                return Ok(());
            };
            let stream_id = format!(
                "display-stream-{}",
                relay_request_counter.fetch_add(1, Ordering::Relaxed) + 1
            );
            let (event_tx, event_rx) = mpsc::channel(display_stream_queue_capacity(&capabilities));
            {
                let mut guard = registry.write().await;
                guard.insert_pending_display_stream(stream_id.clone(), daemon_key, event_tx);
            }
            let result = forward_display_http_stream(
                stream,
                &registry,
                daemon_sender,
                request,
                stream_id.clone(),
                tunnel_id,
                event_rx,
            )
            .await;
            registry
                .write()
                .await
                .remove_pending_display_stream(&stream_id);
            result?;
        }
    }
    Ok(())
}

fn display_stream_queue_capacity(capabilities: &[String]) -> usize {
    if capabilities
        .iter()
        .any(|capability| capability == "encrypted")
    {
        ENCRYPTED_DISPLAY_STREAM_QUEUE_CAPACITY
    } else {
        DISPLAY_STREAM_QUEUE_CAPACITY
    }
}

struct DisplayHttpRequest {
    method: String,
    path: String,
    headers: Vec<RelayDisplayTunnelHeader>,
    body: Vec<u8>,
    websocket_key: Option<String>,
}

async fn read_display_request(stream: &mut TcpStream) -> Result<DisplayHttpRequest, u16> {
    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0_u8; 1024];
    loop {
        if buffer.len() >= DISPLAY_MAX_HEADER_BYTES {
            return Err(431);
        }
        let size = tokio::time::timeout(DISPLAY_REQUEST_READ_TIMEOUT, stream.read(&mut chunk))
            .await
            .map_err(|_| 408_u16)?
            .map_err(|_| 400_u16)?;
        if size == 0 {
            return Err(400);
        }
        buffer.extend_from_slice(&chunk[..size]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let (mut request, body_start) = parse_display_request(&buffer).ok_or(404_u16)?;
    let expected_body_len = content_length(&request.headers)?;
    if expected_body_len > DISPLAY_MAX_BODY_BYTES {
        return Err(413);
    }
    request.body.extend_from_slice(&buffer[body_start..]);
    if request.body.len() > expected_body_len {
        request.body.truncate(expected_body_len);
    }
    while request.body.len() < expected_body_len {
        let remaining = expected_body_len - request.body.len();
        let read_limit = remaining.min(chunk.len());
        let size = tokio::time::timeout(
            DISPLAY_REQUEST_READ_TIMEOUT,
            stream.read(&mut chunk[..read_limit]),
        )
        .await
        .map_err(|_| 408_u16)?
        .map_err(|_| 400_u16)?;
        if size == 0 {
            return Err(400);
        }
        request.body.extend_from_slice(&chunk[..size]);
    }
    Ok(request)
}

async fn forward_display_http_stream(
    mut stream: TcpStream,
    registry: &Arc<RwLock<RelayRegistry>>,
    daemon_sender: RelaySender,
    request: DisplayHttpRequest,
    stream_id: String,
    tunnel_id: String,
    mut event_rx: mpsc::Receiver<DisplayStreamEvent>,
) -> Result<(), std::io::Error> {
    let websocket_key = request.websocket_key.clone();
    let open = RelayEnvelope::DaemonDisplayTunnelOpen {
        request: RelayDisplayTunnelOpenRequest {
            stream_id: stream_id.clone(),
            tunnel_id,
            method: request.method,
            path: request.path,
            headers: request.headers,
            body_base64: (!request.body.is_empty()).then(|| BASE64_STANDARD.encode(&request.body)),
        },
    };
    if send_envelope(&daemon_sender, &open).is_err() {
        write_response(&mut stream, 502, "display tunnel daemon is busy").await?;
        return Ok(());
    }
    let start = match tokio::time::timeout(DISPLAY_RESPONSE_START_TIMEOUT, event_rx.recv()).await {
        Ok(Some(DisplayStreamEvent::ResponseStart { status, headers })) => (status, headers),
        Ok(Some(DisplayStreamEvent::Close { error })) => {
            let message = display_stream_error_message(error.as_ref());
            write_response(&mut stream, 502, &message).await?;
            return Ok(());
        }
        Ok(Some(DisplayStreamEvent::Chunk { .. })) => {
            write_response(
                &mut stream,
                502,
                "display tunnel sent data before response headers",
            )
            .await?;
            return Ok(());
        }
        Ok(None) => {
            write_response(&mut stream, 502, "display tunnel closed before response").await?;
            return Ok(());
        }
        Err(_) => {
            registry
                .write()
                .await
                .remove_pending_display_stream(&stream_id);
            write_response(&mut stream, 504, "display tunnel response timed out").await?;
            return Ok(());
        }
    };
    if let Some(key) = websocket_key {
        if start.0 != 101 {
            write_response(
                &mut stream,
                502,
                "display tunnel websocket handshake failed",
            )
            .await?;
            return Ok(());
        }
        return forward_display_websocket_stream(stream, daemon_sender, stream_id, key, event_rx)
            .await;
    }
    write_response_start(&mut stream, start.0, &start.1).await?;
    loop {
        match tokio::time::timeout(DISPLAY_STREAM_IDLE_TIMEOUT, event_rx.recv()).await {
            Ok(Some(DisplayStreamEvent::Chunk { data, .. })) => {
                let decoded = BASE64_STANDARD
                    .decode(data.as_bytes())
                    .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
                stream.write_all(&decoded).await?;
            }
            Ok(Some(DisplayStreamEvent::Close { .. })) | Ok(None) => break,
            Ok(Some(DisplayStreamEvent::ResponseStart { .. })) => continue,
            Err(_) => break,
        }
    }
    let _ = stream.shutdown().await;
    Ok(())
}

async fn forward_display_websocket_stream(
    mut stream: TcpStream,
    daemon_sender: RelaySender,
    stream_id: String,
    websocket_key: String,
    mut event_rx: mpsc::Receiver<DisplayStreamEvent>,
) -> Result<(), std::io::Error> {
    write_websocket_handshake(&mut stream, &websocket_key).await?;
    let websocket = WebSocketStream::from_raw_socket(stream, Role::Server, None).await;
    let (mut browser_write, mut browser_read) = websocket.split();
    loop {
        tokio::select! {
            browser_message = browser_read.next() => {
                match browser_message {
                    Some(Ok(Message::Binary(data))) => {
                        send_display_client_chunk(&daemon_sender, &stream_id, data.as_ref(), Some("binary"))?;
                    }
                    Some(Ok(Message::Text(data))) => {
                        send_display_client_chunk(&daemon_sender, &stream_id, data.as_str().as_bytes(), Some("text"))?;
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        let _ = send_envelope(
                            &daemon_sender,
                            &RelayEnvelope::DaemonDisplayTunnelClientClose {
                                stream_id: stream_id.clone(),
                                error: None,
                            },
                        );
                        break;
                    }
                    Some(Ok(Message::Ping(data))) => {
                        let _ = browser_write.send(Message::Pong(data)).await;
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Frame(_))) => {}
                    Some(Err(error)) => {
                        let _ = send_envelope(
                            &daemon_sender,
                            &RelayEnvelope::DaemonDisplayTunnelClientClose {
                                stream_id: stream_id.clone(),
                                error: Some(relay_error("display_browser_websocket_failed", &error.to_string(), true)),
                            },
                        );
                        break;
                    }
                }
            }
            event = receive_display_websocket_event(&mut event_rx) => {
                match event {
                    Some(DisplayStreamEvent::Chunk { data, message_kind }) => {
                        let decoded = BASE64_STANDARD
                            .decode(data.as_bytes())
                            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
                        let message = match message_kind.as_deref() {
                            Some("text") => Message::Text(String::from_utf8_lossy(&decoded).to_string().into()),
                            _ => Message::Binary(decoded.into()),
                        };
                        if browser_write.send(message).await.is_err() {
                            break;
                        }
                    }
                    Some(DisplayStreamEvent::Close { .. }) | None => {
                        let _ = browser_write.close().await;
                        break;
                    }
                    Some(DisplayStreamEvent::ResponseStart { .. }) => {}
                }
            }
        }
    }
    Ok(())
}

async fn receive_display_websocket_event(
    event_rx: &mut mpsc::Receiver<DisplayStreamEvent>,
) -> Option<DisplayStreamEvent> {
    match DISPLAY_WEBSOCKET_STREAM_IDLE_TIMEOUT {
        Some(idle_timeout) => tokio::time::timeout(idle_timeout, event_rx.recv())
            .await
            .ok()
            .flatten(),
        None => event_rx.recv().await,
    }
}

#[cfg(test)]
mod websocket_idle_policy_tests {
    use super::{display_stream_queue_capacity, DISPLAY_WEBSOCKET_STREAM_IDLE_TIMEOUT};

    #[test]
    fn display_websocket_stays_open_until_an_explicit_close() {
        assert_eq!(DISPLAY_WEBSOCKET_STREAM_IDLE_TIMEOUT, None);
    }

    #[test]
    fn encrypted_display_stream_uses_the_end_to_end_queue_bound() {
        assert_eq!(
            display_stream_queue_capacity(&["view".to_string(), "encrypted".to_string()]),
            16
        );
        assert_eq!(display_stream_queue_capacity(&["view".to_string()]), 128);
    }
}

fn send_display_client_chunk(
    sender: &RelaySender,
    stream_id: &str,
    data: &[u8],
    message_kind: Option<&str>,
) -> Result<(), std::io::Error> {
    send_envelope(
        sender,
        &RelayEnvelope::DaemonDisplayTunnelClientChunk {
            chunk: crate::protocol::RelayDisplayTunnelStreamChunk {
                stream_id: stream_id.to_string(),
                data: BASE64_STANDARD.encode(data),
                message_kind: message_kind.map(|value| value.to_string()),
            },
        },
    )
}

fn send_envelope(sender: &RelaySender, envelope: &RelayEnvelope) -> Result<(), std::io::Error> {
    let payload = serde_json::to_string(envelope)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    sender
        .try_send(Message::Text(payload.into()))
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::BrokenPipe, error.to_string()))
}

fn display_stream_error_message(error: Option<&RelayError>) -> String {
    error
        .map(|error| format!("display tunnel failed: {}", error.message))
        .unwrap_or_else(|| "display tunnel closed before response".to_string())
}

fn relay_error(code: &str, message: &str, retryable: bool) -> RelayError {
    RelayError {
        code: code.to_string(),
        message: message.to_string(),
        retryable,
    }
}

fn display_request_path(buffer: &[u8]) -> Option<&str> {
    let end = buffer
        .windows(2)
        .position(|window| window == b"\r\n")
        .unwrap_or(buffer.len());
    let request_line = std::str::from_utf8(&buffer[..end]).ok()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?;
    let path = parts.next()?;
    match method {
        "GET" | "HEAD" | "OPTIONS" | "POST" | "PUT" | "PATCH" | "DELETE" => {}
        _ => return None,
    }
    path.starts_with("/display/").then_some(path)
}

fn parse_display_request(buffer: &[u8]) -> Option<(DisplayHttpRequest, usize)> {
    let end = buffer.windows(4).position(|window| window == b"\r\n\r\n")?;
    let request = std::str::from_utf8(&buffer[..end]).ok()?;
    let mut lines = request.split("\r\n");
    let request_line = lines.next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    display_request_path(request_line.as_bytes())?;
    let headers = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some(RelayDisplayTunnelHeader {
                name: name.trim().to_string(),
                value: value.trim().to_string(),
            })
        })
        .filter(|header| !header.name.is_empty())
        .collect::<Vec<_>>();
    let websocket_key = websocket_request_key(&headers);
    Some((
        DisplayHttpRequest {
            method,
            path,
            headers,
            body: Vec::new(),
            websocket_key,
        },
        end + 4,
    ))
}

fn websocket_request_key(headers: &[RelayDisplayTunnelHeader]) -> Option<String> {
    let upgrade = header_value(headers, "upgrade")?;
    if !upgrade.eq_ignore_ascii_case("websocket") {
        return None;
    }
    let connection = header_value(headers, "connection")?;
    if !connection
        .split(',')
        .any(|value| value.trim().eq_ignore_ascii_case("upgrade"))
    {
        return None;
    }
    header_value(headers, "sec-websocket-key").map(str::to_string)
}

fn header_value<'a>(headers: &'a [RelayDisplayTunnelHeader], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|header| header.name.eq_ignore_ascii_case(name))
        .map(|header| header.value.as_str())
}

fn content_length(headers: &[RelayDisplayTunnelHeader]) -> Result<usize, u16> {
    let Some(value) = header_value(headers, "content-length") else {
        return Ok(0);
    };
    value.trim().parse::<usize>().map_err(|_| 400_u16)
}

fn display_tunnel_id(path: &str) -> Option<&str> {
    let remainder = path.strip_prefix("/display/")?;
    let id = remainder.split(['/', '?', '#']).next()?.trim();
    (!id.is_empty() && id.bytes().all(display_tunnel_id_byte_allowed)).then_some(id)
}

fn display_tunnel_id_byte_allowed(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
}

async fn write_response(
    stream: &mut TcpStream,
    status: u16,
    body: &str,
) -> Result<(), std::io::Error> {
    let reason = status_reason(status);
    let body = format!("{body}\n");
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: text/plain; charset=utf-8\r\ncontent-length: {}\r\ncache-control: no-store\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    let _ = stream.shutdown().await;
    Ok(())
}

async fn write_response_start(
    stream: &mut TcpStream,
    status: u16,
    headers: &[RelayDisplayTunnelHeader],
) -> Result<(), std::io::Error> {
    let reason = status_reason(status);
    let mut response =
        format!("HTTP/1.1 {status} {reason}\r\ncache-control: no-store\r\nconnection: close\r\n");
    for header in headers {
        if relay_response_header_allowed(&header.name, &header.value) {
            response.push_str(header.name.trim());
            response.push_str(": ");
            response.push_str(header.value.trim());
            response.push_str("\r\n");
        }
    }
    response.push_str("\r\n");
    stream.write_all(response.as_bytes()).await
}

async fn write_websocket_handshake(
    stream: &mut TcpStream,
    key: &str,
) -> Result<(), std::io::Error> {
    let mut hasher = Sha1::new();
    hasher.update(key.trim().as_bytes());
    hasher.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    let accept = BASE64_STANDARD.encode(hasher.finalize());
    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\nupgrade: websocket\r\nconnection: Upgrade\r\nsec-websocket-accept: {accept}\r\ncache-control: no-store\r\n\r\n"
    );
    stream.write_all(response.as_bytes()).await
}

fn relay_response_header_allowed(name: &str, value: &str) -> bool {
    !name.trim().is_empty()
        && !name
            .bytes()
            .any(|byte| matches!(byte, b'\r' | b'\n' | b':'))
        && !value.bytes().any(|byte| matches!(byte, b'\r' | b'\n'))
}

fn status_reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        408 => "Request Timeout",
        410 => "Gone",
        413 => "Payload Too Large",
        431 => "Request Header Fields Too Large",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        504 => "Gateway Timeout",
        _ => "Error",
    }
}

fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_request_path_accepts_only_display_http_requests() {
        assert_eq!(
            display_request_path(b"GET /display/tunnel-1/vnc.html HTTP/1.1\r\n"),
            Some("/display/tunnel-1/vnc.html")
        );
        assert_eq!(
            display_request_path(b"POST /display/tunnel/api HTTP/1.1\r\n"),
            Some("/display/tunnel/api")
        );
        assert_eq!(
            display_request_path(b"GET /kernel HTTP/1.1\r\nUpgrade: websocket\r\n"),
            None
        );
    }

    #[test]
    fn display_tunnel_id_parses_first_safe_segment() {
        assert_eq!(
            display_tunnel_id("/display/abc-123_/vnc.html"),
            Some("abc-123_")
        );
        assert_eq!(display_tunnel_id("/display/abc.123?x=1"), Some("abc.123"));
        assert_eq!(display_tunnel_id("/display/"), None);
        assert_eq!(display_tunnel_id("/display/abc%2Fdef"), None);
    }

    #[test]
    fn parse_display_request_preserves_method_path_and_headers() {
        let raw =
            b"GET /display/tunnel-1/vnc.html HTTP/1.1\r\nhost: relay.test\r\naccept: text/html\r\n\r\n";
        let (request, body_start) = parse_display_request(raw).expect("request should parse");

        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/display/tunnel-1/vnc.html");
        assert_eq!(request.headers[0].name, "host");
        assert_eq!(request.headers[1].value, "text/html");
        assert_eq!(request.websocket_key, None);
        assert_eq!(body_start, raw.len());
    }

    #[test]
    fn parse_display_request_preserves_post_request_body_metadata() {
        let headers =
            b"POST /display/tunnel-1/invoke HTTP/1.1\r\nhost: relay.test\r\ncontent-length: 15\r\ncontent-type: application/json\r\n\r\n";
        let raw = [headers.as_slice(), b"{\"prompt\":\"hi\"}".as_slice()].concat();
        let (request, body_start) = parse_display_request(&raw).expect("request should parse");

        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/display/tunnel-1/invoke");
        assert_eq!(content_length(&request.headers), Ok(15));
        assert_eq!(body_start, headers.len());
    }

    #[test]
    fn parse_display_request_detects_websocket_upgrade() {
        let (request, _) = parse_display_request(
            b"GET /display/tunnel-1/websockify HTTP/1.1\r\nhost: relay.test\r\nconnection: keep-alive, Upgrade\r\nupgrade: websocket\r\nsec-websocket-key: abc\r\n\r\n",
        )
        .expect("request should parse");

        assert_eq!(request.websocket_key.as_deref(), Some("abc"));
    }

    #[test]
    fn display_response_start_timeout_allows_synchronous_workflow_calls() {
        assert!(DISPLAY_RESPONSE_START_TIMEOUT >= Duration::from_secs(300));
    }

    #[test]
    fn display_daemon_send_fails_fast_when_queue_is_full() {
        let (sender, _receiver) = mpsc::channel(1);
        sender
            .try_send(Message::Text("occupied".to_string().into()))
            .expect("test queue should accept first message");

        let error = send_envelope(
            &sender,
            &RelayEnvelope::Close {
                reason: "should fail fast".to_string(),
            },
        )
        .expect_err("full daemon queue should reject display envelope");

        assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
    }
}
