//! Relay peer event ingestion and leased runtime projection emission.

use super::*;

pub(super) async fn pump_leased_projection_events(
    router: &Arc<CommandRouter>,
    outgoing_tx: &mpsc::UnboundedSender<RelayEnvelope>,
) {
    let events = match router.relay_pump_leased_runtime_projections().await {
        Ok(events) => events,
        Err(error) => {
            crate::logging::warn_with_fields(
                "daemon.relay",
                "failed to pump leased runtime projections",
                serde_json::json!({
                    "error": error.to_string(),
                }),
            );
            return;
        }
    };
    for (target_daemon_id, event) in events {
        let config = router.relay_config_snapshot();
        let target_kernel = match tokio::time::timeout(
            std::time::Duration::from_secs(2),
            relay_discovery::get_live_kernel(&config, &target_daemon_id),
        )
        .await
        {
            Ok(Ok(kernel)) => kernel,
            Ok(Err(error)) => {
                crate::logging::warn_with_fields(
                    "daemon.relay",
                    "failed to resolve leased runtime projection target",
                    serde_json::json!({
                        "target_daemon_id": target_daemon_id,
                        "error": error.to_string(),
                    }),
                );
                continue;
            }
            Err(_) => {
                crate::logging::warn_with_fields(
                    "daemon.relay",
                    "timed out resolving leased runtime projection target",
                    serde_json::json!({
                        "target_daemon_id": target_daemon_id,
                    }),
                );
                continue;
            }
        };
        let encrypted_event = match encrypt_peer_payload(
            &config.relay_private_key,
            &target_kernel.public_key,
            &event,
        ) {
            Ok(payload) => payload,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.relay",
                    "failed to encrypt leased runtime projection event",
                    serde_json::json!({
                        "target_daemon_id": target_daemon_id,
                        "error": error.to_string(),
                    }),
                );
                continue;
            }
        };
        if let Err(error) = send_outgoing_envelope(
            outgoing_tx,
            RelayEnvelope::DaemonPeerEvent {
                target: ClientTarget {
                    daemon_id: Some(target_daemon_id.clone()),
                    daemon_alias: None,
                },
                encrypted_event,
            },
        ) {
            crate::logging::warn_with_fields(
                "daemon.relay",
                "failed to send leased runtime projection event",
                serde_json::json!({
                    "target_daemon_id": target_daemon_id,
                    "error": error.to_string(),
                }),
            );
        }
    }
}

pub(super) async fn handle_daemon_peer_event(
    router: &Arc<CommandRouter>,
    encrypted_event: EncryptedRelayPayload,
) -> Result<(), DaemonError> {
    let daemon_private_key = router.relay_private_key();
    let decrypted =
        relay_crypto::decrypt_payload_for_private_key(&daemon_private_key, &encrypted_event)?;
    let event =
        serde_json::from_slice::<RelayPeerEvent>(&decrypted.plaintext).map_err(|error| {
            DaemonError::LocalTransport {
                operation: "decode relay peer event",
                message: error.to_string(),
            }
        })?;
    match event {
        RelayPeerEvent::LeasedRuntimeProjection {
            home_session_id,
            home_agent_id,
            provider_run_id,
            prompts,
            output_chunks,
            notices,
            completions,
        } => {
            router
                .relay_project_remote_runtime_projection(
                    &home_session_id,
                    &home_agent_id,
                    &provider_run_id,
                    prompts,
                    output_chunks,
                    notices,
                    completions,
                )
                .await?;
        }
    }
    Ok(())
}

pub(super) async fn emit_leased_projection_event(
    router: &Arc<CommandRouter>,
    outgoing_tx: &mpsc::UnboundedSender<RelayEnvelope>,
    leased_agent_id: &str,
    provider_run_id: &str,
    pump_output: bool,
) -> Result<(), DaemonError> {
    let config = router.relay_config_snapshot();
    let Some((target_daemon_id, event)) = router
        .relay_drain_leased_runtime_projection(leased_agent_id, provider_run_id, pump_output)
        .await?
    else {
        return Ok(());
    };
    let target_kernel = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        relay_discovery::get_live_kernel(&config, &target_daemon_id),
    )
    .await
    .map_err(|_| DaemonError::LocalTransport {
        operation: "resolve relay peer event target",
        message: format!("timed out resolving relay target kernel `{target_daemon_id}`"),
    })??;
    let encrypted_event =
        encrypt_peer_payload(&config.relay_private_key, &target_kernel.public_key, &event)?;
    send_outgoing_envelope(
        outgoing_tx,
        RelayEnvelope::DaemonPeerEvent {
            target: ClientTarget {
                daemon_id: Some(target_daemon_id),
                daemon_alias: None,
            },
            encrypted_event,
        },
    )
}
