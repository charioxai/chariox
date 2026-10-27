use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::{TcpListener, TcpStream};
use tokio::time::{sleep, Instant};

use crate::runtime::projection::TransportHealthStore;

// MP-08/MP-10: an admission failure must not end kernel authority or leave MCP
// permanently offline. Retry only acceptance; existing connections keep running.
// Callers select this future alongside shutdown/control work, so even sustained
// descriptor/resource pressure never blocks those branches or spins the loop.
pub(crate) async fn accept_with_backoff(
    listener: &TcpListener,
    health: &TransportHealthStore,
    transport: &'static str,
) -> (TcpStream, SocketAddr) {
    accept_with_retry(|| listener.accept(), health, transport).await
}

pub(crate) async fn accept_unix_with_backoff(
    listener: &tokio::net::UnixListener,
    health: &TransportHealthStore,
    transport: &'static str,
) -> (tokio::net::UnixStream, tokio::net::unix::SocketAddr) {
    accept_with_retry(|| listener.accept(), health, transport).await
}

async fn accept_with_retry<T, F, Fut>(
    mut accept: F,
    health: &TransportHealthStore,
    transport: &'static str,
) -> T
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = std::io::Result<T>>,
{
    let mut delay = Duration::from_millis(100);
    let mut failures = 0u64;
    let mut next_diagnostic = Instant::now();
    loop {
        match accept().await {
            Ok(connection) => {
                if failures > 0 {
                    crate::logging::info_with_fields(
                        "daemon.listener_admission",
                        "local transport admission recovered",
                        serde_json::json!({"transport": transport, "failed_attempts": failures}),
                    );
                }
                return connection;
            }
            Err(error) => {
                failures = failures.saturating_add(1);
                health.record_inbound_overload_rejection();
                if Instant::now() >= next_diagnostic {
                    crate::logging::warn_with_fields(
                        "daemon.listener_admission",
                        "local transport admission unavailable; retrying while existing connections remain active",
                        serde_json::json!({
                            "transport": transport,
                            "error": error.to_string(),
                            "retry_delay_ms": delay.as_millis(),
                            "failed_attempts": failures,
                            "action": "release process file descriptors or raise its open-file limit if exhausted",
                        }),
                    );
                    next_diagnostic = Instant::now() + Duration::from_secs(30);
                }
                sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(1));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[tokio::test]
    async fn unix_admission_recovers_after_descriptor_exhaustion() {
        let health = TransportHealthStore::default();
        let attempts = Cell::new(0);
        let accepted = accept_with_retry(
            || {
                let attempt = attempts.get();
                attempts.set(attempt + 1);
                std::future::ready(if attempt < 2 {
                    Err(std::io::Error::from_raw_os_error(libc::EMFILE))
                } else {
                    Ok("accepted")
                })
            },
            &health,
            "unix-test",
        )
        .await;
        assert_eq!(accepted, "accepted");
        assert_eq!(attempts.get(), 3);
        assert_eq!(health.snapshot(0, 0, 0).inbound_overload_rejections, 2);
    }

    #[tokio::test]
    async fn unix_admission_pressure_keeps_shutdown_selectable() {
        let health = TransportHealthStore::default();
        let admission = accept_with_retry(
            || {
                std::future::ready(Err::<(), _>(std::io::Error::from_raw_os_error(
                    libc::EMFILE,
                )))
            },
            &health,
            "unix-test",
        );
        tokio::select! {
            _ = admission => panic!("exhaustion must not terminate the server"),
            _ = sleep(Duration::from_millis(120)) => {},
        }
        assert!(health.snapshot(0, 0, 0).inbound_overload_rejections >= 1);
    }
}
