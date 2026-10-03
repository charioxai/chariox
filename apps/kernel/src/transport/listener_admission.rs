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
    let mut delay = Duration::from_millis(100);
    let mut failures = 0u64;
    let mut next_diagnostic = Instant::now();
    loop {
        match listener.accept().await {
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
