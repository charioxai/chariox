use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::RwLock;

use crate::registry::RelayRegistry;

const HEALTH_PEEK_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(250);
const HEALTH_REQUEST_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
const HEALTH_MAX_REQUEST_BYTES: usize = 8 * 1024;

pub(crate) async fn is_health_http_request(stream: &TcpStream) -> bool {
    let mut buffer = [0_u8; 512];
    let Ok(Ok(size)) = tokio::time::timeout(HEALTH_PEEK_TIMEOUT, stream.peek(&mut buffer)).await
    else {
        return false;
    };
    health_request_path(&buffer[..size]).is_some()
}

pub(crate) async fn handle_health_connection(
    mut stream: TcpStream,
    registry: Arc<RwLock<RelayRegistry>>,
    draining: Arc<AtomicBool>,
) -> Result<(), std::io::Error> {
    let Some(path) = read_health_request(&mut stream).await else {
        write_json_response(
            &mut stream,
            400,
            json!({
                "status": "invalid",
                "draining": draining.load(Ordering::Relaxed),
                "unix_ms": current_unix_ms(),
            }),
        )
        .await?;
        return Ok(());
    };

    let is_draining = draining.load(Ordering::Relaxed);
    let status = if is_draining { "draining" } else { "healthy" };
    let http_status = if is_draining && path == HealthRequestPath::Ready {
        503
    } else {
        200
    };
    let guard = registry.read().await;
    let body = json!({
        "status": status,
        "draining": is_draining,
        "unix_ms": current_unix_ms(),
        "peer_count": guard.peer_count(),
        "daemon_count": guard.daemon_count(),
        "pending_request_count": guard.pending_request_count(),
        "subscription_count": guard.subscription_count(),
        "display_tunnel_count": guard.display_tunnel_count(),
    });
    drop(guard);

    write_json_response(&mut stream, http_status, body).await
}

async fn read_health_request(stream: &mut TcpStream) -> Option<HealthRequestPath> {
    let mut buffer = Vec::with_capacity(256);
    let mut chunk = [0_u8; 512];
    loop {
        if buffer.len() >= HEALTH_MAX_REQUEST_BYTES {
            return None;
        }
        let size = tokio::time::timeout(HEALTH_REQUEST_READ_TIMEOUT, stream.read(&mut chunk))
            .await
            .ok()?
            .ok()?;
        if size == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..size]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            return health_request_path(&buffer);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HealthRequestPath {
    Health,
    Ready,
}

fn health_request_path(buffer: &[u8]) -> Option<HealthRequestPath> {
    let end = buffer
        .windows(2)
        .position(|window| window == b"\r\n")
        .unwrap_or(buffer.len());
    let request_line = std::str::from_utf8(&buffer[..end]).ok()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?;
    let path = parts.next()?;
    if method != "GET" {
        return None;
    }
    match path {
        "/healthz" | "/health" => Some(HealthRequestPath::Health),
        "/readyz" => Some(HealthRequestPath::Ready),
        _ => None,
    }
}

async fn write_json_response(
    stream: &mut TcpStream,
    status: u16,
    body: serde_json::Value,
) -> Result<(), std::io::Error> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        503 => "Service Unavailable",
        _ => "Error",
    };
    let body =
        serde_json::to_string(&body).map_err(|error| std::io::Error::other(error.to_string()))?;
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\ncache-control: no-store\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    let _ = stream.shutdown().await;
    Ok(())
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
    fn health_request_path_accepts_only_health_gets() {
        assert_eq!(
            health_request_path(b"GET /healthz HTTP/1.1\r\n"),
            Some(HealthRequestPath::Health)
        );
        assert_eq!(
            health_request_path(b"GET /readyz HTTP/1.1\r\n"),
            Some(HealthRequestPath::Ready)
        );
        assert_eq!(health_request_path(b"POST /healthz HTTP/1.1\r\n"), None);
        assert_eq!(
            health_request_path(b"GET /display/tunnel HTTP/1.1\r\n"),
            None
        );
    }
}
