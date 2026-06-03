use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, RwLock};
use tokio_tungstenite::tungstenite::Message;

use crate::protocol::{
    RelayDisplayTunnelHeader, RelayDisplayTunnelOpenRequest, RelayEnvelope, RelayError,
};
use crate::registry::{DisplayStreamEvent, DisplayTunnelLookup, RelayRegistry, RelaySender};

const DISPLAY_PEEK_TIMEOUT: Duration = Duration::from_millis(250);
const DISPLAY_REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(5);
const DISPLAY_RESPONSE_START_TIMEOUT: Duration = Duration::from_secs(10);
const DISPLAY_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
const DISPLAY_MAX_HEADER_BYTES: usize = 16 * 1024;
const DISPLAY_STREAM_QUEUE_CAPACITY: usize = 128;

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
        } => {
            let Some(daemon_sender) = daemon_sender else {
                write_response(&mut stream, 502, "display tunnel daemon is disconnected").await?;
                return Ok(());
            };
            let stream_id = format!(
                "display-stream-{}",
                relay_request_counter.fetch_add(1, Ordering::Relaxed) + 1
            );
            let (event_tx, event_rx) = mpsc::channel(DISPLAY_STREAM_QUEUE_CAPACITY);
            {
                let mut guard = registry.write().await;
                guard.insert_pending_display_stream(stream_id.clone(), daemon_key, event_tx);
            }
            let result = forward_display_http_stream(
                &mut stream,
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

struct DisplayHttpRequest {
    method: String,
    path: String,
    headers: Vec<RelayDisplayTunnelHeader>,
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
    parse_display_request(&buffer).ok_or(404_u16)
}

async fn forward_display_http_stream(
    stream: &mut TcpStream,
    registry: &Arc<RwLock<RelayRegistry>>,
    daemon_sender: RelaySender,
    request: DisplayHttpRequest,
    stream_id: String,
    tunnel_id: String,
    mut event_rx: mpsc::Receiver<DisplayStreamEvent>,
) -> Result<(), std::io::Error> {
    let open = RelayEnvelope::DaemonDisplayTunnelOpen {
        request: RelayDisplayTunnelOpenRequest {
            stream_id: stream_id.clone(),
            tunnel_id,
            method: request.method,
            path: request.path,
            headers: request.headers,
        },
    };
    send_envelope(&daemon_sender, &open).await?;
    let start = match tokio::time::timeout(DISPLAY_RESPONSE_START_TIMEOUT, event_rx.recv()).await {
        Ok(Some(DisplayStreamEvent::ResponseStart { status, headers })) => (status, headers),
        Ok(Some(DisplayStreamEvent::Close { error })) => {
            let message = display_stream_error_message(error.as_ref());
            write_response(stream, 502, &message).await?;
            return Ok(());
        }
        Ok(Some(DisplayStreamEvent::Chunk { .. })) => {
            write_response(
                stream,
                502,
                "display tunnel sent data before response headers",
            )
            .await?;
            return Ok(());
        }
        Ok(None) => {
            write_response(stream, 502, "display tunnel closed before response").await?;
            return Ok(());
        }
        Err(_) => {
            registry
                .write()
                .await
                .remove_pending_display_stream(&stream_id);
            write_response(stream, 504, "display tunnel response timed out").await?;
            return Ok(());
        }
    };
    write_response_start(stream, start.0, &start.1).await?;
    loop {
        match tokio::time::timeout(DISPLAY_STREAM_IDLE_TIMEOUT, event_rx.recv()).await {
            Ok(Some(DisplayStreamEvent::Chunk { data })) => {
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

async fn send_envelope(
    sender: &RelaySender,
    envelope: &RelayEnvelope,
) -> Result<(), std::io::Error> {
    let payload = serde_json::to_string(envelope)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    sender
        .send(Message::Text(payload.into()))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "daemon disconnected"))
}

fn display_stream_error_message(error: Option<&RelayError>) -> String {
    error
        .map(|error| format!("display tunnel failed: {}", error.message))
        .unwrap_or_else(|| "display tunnel closed before response".to_string())
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
        "GET" | "HEAD" | "OPTIONS" => {}
        _ => return None,
    }
    path.starts_with("/display/").then_some(path)
}

fn parse_display_request(buffer: &[u8]) -> Option<DisplayHttpRequest> {
    let end = buffer.windows(4).position(|window| window == b"\r\n\r\n")?;
    let request = std::str::from_utf8(&buffer[..end]).ok()?;
    let mut lines = request.split("\r\n");
    let request_line = lines.next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    if display_request_path(request_line.as_bytes()).is_none() {
        return None;
    }
    let headers = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some(RelayDisplayTunnelHeader {
                name: name.trim().to_string(),
                value: value.trim().to_string(),
            })
        })
        .filter(|header| !header.name.is_empty())
        .collect();
    Some(DisplayHttpRequest {
        method,
        path,
        headers,
    })
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
            display_request_path(b"GET /kernel HTTP/1.1\r\nUpgrade: websocket\r\n"),
            None
        );
        assert_eq!(
            display_request_path(b"POST /display/tunnel HTTP/1.1\r\n"),
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
        let request = parse_display_request(
            b"GET /display/tunnel-1/vnc.html HTTP/1.1\r\nhost: relay.test\r\naccept: text/html\r\n\r\n",
        )
        .expect("request should parse");

        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/display/tunnel-1/vnc.html");
        assert_eq!(request.headers[0].name, "host");
        assert_eq!(request.headers[1].value, "text/html");
    }
}
