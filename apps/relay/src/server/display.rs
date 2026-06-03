use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::RwLock;

use crate::registry::{DisplayTunnelLookup, RelayRegistry};

const DISPLAY_PEEK_TIMEOUT: Duration = Duration::from_millis(250);
const DISPLAY_REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(5);
const DISPLAY_MAX_HEADER_BYTES: usize = 16 * 1024;

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
    let lookup = {
        let guard = registry.read().await;
        guard.display_tunnel_lookup(tunnel_id, current_unix_ms())
    };
    match lookup {
        DisplayTunnelLookup::Missing => {
            write_response(&mut stream, 404, "display tunnel not found").await?;
        }
        DisplayTunnelLookup::Expired => {
            write_response(&mut stream, 410, "display tunnel expired").await?;
        }
        DisplayTunnelLookup::Active { daemon_sender } => {
            if daemon_sender.is_none() {
                write_response(&mut stream, 502, "display tunnel daemon is disconnected").await?;
            } else {
                write_response(
                    &mut stream,
                    501,
                    "display stream forwarding is not implemented",
                )
                .await?;
            }
        }
    }
    Ok(())
}

struct DisplayHttpRequest {
    path: String,
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
    let path = display_request_path(&buffer).ok_or(404_u16)?.to_string();
    Ok(DisplayHttpRequest { path })
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

fn status_reason(status: u16) -> &'static str {
    match status {
        400 => "Bad Request",
        404 => "Not Found",
        408 => "Request Timeout",
        410 => "Gone",
        431 => "Request Header Fields Too Large",
        501 => "Not Implemented",
        502 => "Bad Gateway",
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
}
