//! MP-08/MP-10/MP-11: supplementary live fault traffic, not owner Copy proof.
//! Load an owned running kernel's product-created identity; never print it.
use chariox_kernel::DaemonConfig;
use chariox_relay::protocol::{ClientTarget, DaemonRegistration, RelayEnvelope};
use futures_util::SinkExt;
use tokio_tungstenite::{connect_async, tungstenite::Message};

// Use the product encryption implementation without widening the kernel API.
pub use chariox_kernel::error;
#[allow(dead_code)]
#[path = "../src/transport/relay_crypto.rs"]
mod relay_crypto;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let target = args.next().ok_or("target kernel id required")?;
    let bytes: usize = args.next().ok_or("payload size required")?.parse()?;
    if bytes == 0 || bytes > 4 * 1024 * 1024 || args.next().is_some() {
        return Err("payload size must be 1..4194304 and no extra arguments".into());
    }
    let config = DaemonConfig::load_from_env();
    // Self-target only: never probe another lane's identity or running kernel.
    if target != config.daemon_id {
        return Err("target must match the owned kernel's product identity".into());
    }
    let url = config
        .relay_url
        .as_ref()
        .ok_or("owned kernel needs a relay")?;
    let payload = "p".repeat(bytes);
    let request = serde_json::to_vec(&serde_json::json!({"kind":"ping", "value":payload}))?;
    let encrypted_request = relay_crypto::encrypt_payload_for_peer(
        &config.relay_private_key,
        &config.relay_public_key,
        &request,
    )?;
    let started = std::time::Instant::now();
    let request_id = format!(
        "daemon-peer-tmp-{}-{}-1",
        std::process::id(),
        chariox_kernel::session::unix_epoch_ms()
    );
    let stage = std::cell::Cell::new("connect");
    let operation = async {
        let (mut socket, _) =
            connect_async(chariox_relay::frame_transport::transport_url(url)).await?;
        // Same temporary peer registration namespace used by the kernel's peer
        // client. Identity, signing scope and keys come from the product loader.
        stage.set("register");
        let registration = DaemonRegistration {
            auth_token: config
                .relay_token
                .clone()
                .ok_or("owned kernel needs authentication")?,
            daemon_id: format!("{}:peer-tmp:{}", config.daemon_id, request_id),
            machine_id: config.host_machine_id.clone(),
            machine_alias: config.host_machine_alias.clone(),
            os_name: Some(config.os_name.clone()),
            kernel_started_at_ms: chariox_kernel::session::unix_epoch_ms(),
            daemon_alias: None,
            kernel_alias: None,
            public_key: config.relay_public_key.clone(),
            capabilities: vec!["relay_peer_transport".into()],
            available_providers: Vec::new(),
            provider_accounts: Vec::new(),
            accepting_remote_leases: false,
            leased_agent_count: 0,
            local_session_count: 0,
        };
        if chariox_relay::protocol::canonical_peer_daemon_id(&registration.daemon_id)
            != Some(config.daemon_id.as_str())
        {
            return Err("probe must use the product temporary peer namespace".into());
        }
        socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonRegister { registration })?.into(),
            ))
            .await?;
        stage.set("send_peer_request");
        socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonPeerRequest {
                    request_id: request_id.clone(),
                    target: ClientTarget {
                        daemon_id: Some(target.clone()),
                        daemon_alias: None,
                    },
                    encrypted_request,
                })?
                .into(),
            ))
            .await?;
        stage.set("read_response");
        let mut frames = chariox_relay::frame_transport::FrameReceiver::default();
        while let Some(message) =
            chariox_relay::frame_transport::read_message(&mut socket, &mut frames).await
        {
            let Message::Text(text) = message? else {
                continue;
            };
            match serde_json::from_str::<RelayEnvelope>(&text)? {
                RelayEnvelope::DaemonPeerResponse {
                    request_id: returned,
                    from_daemon_id,
                    encrypted_response: Some(response),
                    error: None,
                } if returned == request_id && from_daemon_id == target => {
                    stage.set("verify_binding");
                    if response.sender_public_key != config.relay_public_key {
                        return Ok(false);
                    }
                    stage.set("decrypt_response");
                    let decrypted = relay_crypto::decrypt_payload_for_private_key(
                        &config.relay_private_key,
                        &response,
                    )?;
                    let response: serde_json::Value = serde_json::from_slice(&decrypted.plaintext)?;
                    let intact = response["kind"] == "pong"
                        && response["daemon_id"] == target
                        && response["value"] == payload;
                    socket.close(None).await?;
                    stage.set(if intact {
                        "verified"
                    } else {
                        "payload_mismatch"
                    });
                    return Ok(intact);
                }
                RelayEnvelope::Close { .. } => {
                    stage.set("relay_close");
                    return Ok(false);
                }
                RelayEnvelope::DaemonPeerResponse { .. } => {
                    stage.set("peer_error_or_wrong_response");
                    return Ok(false);
                }
                _ => {}
            }
        }
        Ok::<bool, Box<dyn std::error::Error>>(false)
    };
    let intact = matches!(
        tokio::time::timeout(std::time::Duration::from_secs(10), operation).await,
        Ok(Ok(true))
    );
    println!(
        "{}",
        serde_json::json!({"mpItems":["MP-08","MP-10","MP-11"], "intact":intact, "stage":stage.get(), "bytes":bytes, "elapsedMs":started.elapsed().as_millis()})
    );
    Ok(())
}
