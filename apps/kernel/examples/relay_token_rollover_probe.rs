//! MP-08/MP-10/MP-11: inject normal encrypted peer traffic during a live drill.
//! Uses only the lane kernel's product-created profile; never prints credentials.
use chariox_kernel::transport::relay_client::send_peer_request_via_temporary_connection;
use chariox_kernel::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use chariox_kernel::DaemonConfig;
use chariox_relay::protocol::ClientTarget;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let target = args.next().ok_or("target kernel id required")?;
    let bytes: usize = args.next().ok_or("payload size required")?.parse()?;
    if bytes == 0 || bytes > 4 * 1024 * 1024 || args.next().is_some() {
        return Err("payload size must be 1..4194304 and no extra arguments".into());
    }
    // Same loader used by the real kernel. In live drills CHARIOX_HOME and all
    // identity/listener settings must match the owned, already running kernel.
    let config = DaemonConfig::load_from_env();
    let payload = "p".repeat(bytes);
    let started = std::time::Instant::now();
    let response = send_peer_request_via_temporary_connection(
        &config,
        ClientTarget {
            daemon_id: Some(target.clone()),
            daemon_alias: None,
        },
        RelayPeerRequest::Ping {
            value: payload.clone(),
        },
    )
    .await;
    let intact = matches!(response, Ok(RelayPeerResponse::Pong { value, daemon_id })
        if value == payload && daemon_id == target);
    println!(
        "{}",
        serde_json::json!({
            "mpItems": ["MP-08", "MP-10", "MP-11"],
            "intact": intact,
            "bytes": bytes,
            "elapsedMs": started.elapsed().as_millis(),
        })
    );
    Ok(())
}
