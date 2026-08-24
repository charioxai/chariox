//! Incoming relay envelope dispatch for daemon, peer, and subscription traffic.

use super::*;

pub(super) async fn handle_incoming_envelope(
    router: &Arc<CommandRouter>,
    command_sequence: &Arc<AtomicU64>,
    state: &Arc<RwLock<RelayClientState>>,
    outgoing_tx: &RelayOutgoingSender,
    subscription_tasks: &RelaySubscriptionTasks,
    event_runtime: &Arc<RelayEventRuntime>,
    command_result_cache: &RelayCommandResultCache,
    active_dynamic_relay: Option<(&str, &str)>,
    payload: &str,
) -> Result<(), DaemonError> {
    let envelope = serde_json::from_str::<RelayEnvelope>(payload).map_err(|error| {
        DaemonError::LocalTransport {
            operation: "parse relay envelope",
            message: error.to_string(),
        }
    })?;
    match envelope {
        RelayEnvelope::DaemonRequest {
            relay_request_id,
            caller_identity,
            encrypted_request,
        } => {
            let router = Arc::clone(router);
            let command_sequence = Arc::clone(command_sequence);
            let outgoing_tx = outgoing_tx.clone();
            let command_result_cache = Arc::clone(command_result_cache);
            tokio::spawn(async move {
                let relay_response = handle_daemon_request(
                    &router,
                    &command_sequence,
                    caller_identity,
                    encrypted_request,
                    &command_result_cache,
                )
                .await;
                if let Err(error) = send_outgoing_envelope(
                    &outgoing_tx,
                    RelayEnvelope::DaemonResponse {
                        relay_request_id,
                        encrypted_response: relay_response.encrypted_response,
                        error: relay_response.error,
                    },
                ) {
                    crate::logging::warn_with_fields(
                        "daemon.relay_client",
                        "failed to send async daemon response",
                        serde_json::json!({
                            "error": error.to_string(),
                        }),
                    );
                }
            });
        }
        RelayEnvelope::DaemonIncomingPeerRequest {
            relay_request_id,
            from_daemon_id,
            caller_identity,
            encrypted_request,
        } => {
            let router = Arc::clone(router);
            let state = Arc::clone(state);
            let outgoing_tx = outgoing_tx.clone();
            let active_dynamic_relay = active_dynamic_relay
                .map(|(relay_url, relay_token)| (relay_url.to_string(), relay_token.to_string()));
            tokio::spawn(async move {
                let relay_response = handle_daemon_peer_request(
                    &router,
                    &state,
                    &outgoing_tx,
                    &from_daemon_id,
                    caller_identity,
                    encrypted_request,
                )
                .await;
                if let Err(error) = send_outgoing_envelope(
                    &outgoing_tx,
                    RelayEnvelope::DaemonIncomingPeerResponse {
                        relay_request_id,
                        encrypted_response: relay_response.encrypted_response,
                        error: relay_response.error,
                    },
                ) {
                    crate::logging::warn_with_fields(
                        "daemon.relay_client",
                        "failed to send async daemon peer response",
                        serde_json::json!({
                            "error": error.to_string(),
                        }),
                    );
                    return;
                }
                if let Some((active_relay_url, active_relay_token)) = active_dynamic_relay.as_ref()
                {
                    match enqueue_dynamic_relay_reconnect_if_changed(
                        &outgoing_tx,
                        active_relay_url,
                        active_relay_token,
                        &router.relay_config_snapshot(),
                    ) {
                        Ok(Some(reason)) => crate::logging::info_with_fields(
                            "daemon.relay_client",
                            "relay reconnect queued after peer response",
                            serde_json::json!({
                                "reason": reason,
                            }),
                        ),
                        Ok(None) => {}
                        Err(error) => crate::logging::warn_with_fields(
                            "daemon.relay_client",
                            "failed to queue relay reconnect after peer response",
                            serde_json::json!({
                                "error": error.to_string(),
                            }),
                        ),
                    }
                }
            });
        }
        RelayEnvelope::DaemonPeerResponse {
            request_id,
            from_daemon_id,
            encrypted_response,
            error,
        } => {
            resolve_pending_peer_response(
                state,
                request_id,
                RelayPeerResponseEnvelope {
                    from_daemon_id,
                    encrypted_response,
                    error,
                },
            )
            .await;
        }
        RelayEnvelope::DaemonIncomingPeerEvent {
            from_daemon_id: _,
            caller_identity: _,
            encrypted_event,
        } => {
            let router = Arc::clone(router);
            tokio::spawn(async move {
                if let Err(error) = handle_daemon_peer_event(&router, encrypted_event).await {
                    crate::logging::warn_with_fields(
                        "daemon.relay_client",
                        "failed to handle relay peer event",
                        serde_json::json!({
                            "error": error.to_string(),
                        }),
                    );
                }
            });
        }
        RelayEnvelope::DaemonSubscribe {
            relay_request_id,
            relay_subscription_id,
            caller_identity,
            session_id,
            attachment_id,
            client_public_key,
            subscription_scope,
            resume_from_event_id,
        } => {
            handle_relay_subscribe(
                router,
                outgoing_tx,
                subscription_tasks,
                event_runtime,
                relay_request_id,
                relay_subscription_id,
                session_id,
                attachment_id,
                caller_identity,
                client_public_key,
                subscription_scope,
                resume_from_event_id,
            )
            .await?;
        }
        RelayEnvelope::DaemonUnsubscribe {
            relay_request_id,
            relay_subscription_id,
            caller_identity: _,
            client_public_key,
        } => {
            handle_relay_unsubscribe(
                router,
                outgoing_tx,
                subscription_tasks,
                relay_request_id,
                relay_subscription_id,
                client_public_key,
            )
            .await?;
        }
        RelayEnvelope::DaemonDisplayTunnelOpen { request } => {
            let state = Arc::clone(state);
            let outgoing_tx = outgoing_tx.clone();
            tokio::spawn(async move {
                handle_display_tunnel_open(state, outgoing_tx, request).await;
            });
        }
        RelayEnvelope::DaemonDisplayTunnelRegistered {
            tunnel_id, error, ..
        } => {
            state
                .write()
                .await
                .resolve_display_tunnel_registration(&tunnel_id, error);
        }
        RelayEnvelope::DaemonDisplayTunnelClientChunk { chunk } => {
            let sender = {
                let guard = state.read().await;
                guard.display_stream_sender(&chunk.stream_id)
            };
            if let Some(sender) = sender {
                let _ = sender.try_send(RelayDisplayTunnelClientEvent::Chunk(chunk));
            }
        }
        RelayEnvelope::DaemonDisplayTunnelClientClose { stream_id, .. } => {
            let sender = {
                let guard = state.read().await;
                guard.display_stream_sender(&stream_id)
            };
            if let Some(sender) = sender {
                let _ = sender.try_send(RelayDisplayTunnelClientEvent::Close);
            }
        }
        RelayEnvelope::ClientMetadataResponse { .. } => {}
        RelayEnvelope::Close { reason } => {
            return Err(DaemonError::LocalTransport {
                operation: "relay closed connection",
                message: reason,
            });
        }
        _ => {}
    }
    Ok(())
}

fn enqueue_dynamic_relay_reconnect_if_changed(
    outgoing_tx: &RelayOutgoingSender,
    active_relay_url: &str,
    active_relay_token: &str,
    config: &crate::config::DaemonConfig,
) -> Result<Option<&'static str>, DaemonError> {
    match relay_config_continuity(active_relay_url, active_relay_token, config) {
        RelayConfigContinuity::Continue => Ok(None),
        RelayConfigContinuity::Reauthenticate => {
            send_outgoing_envelope(
                outgoing_tx,
                RelayEnvelope::Close {
                    reason: "relay configuration changed after peer response".to_string(),
                },
            )?;
            Ok(Some("relay token changed"))
        }
        RelayConfigContinuity::Reconnect(reason) => {
            send_outgoing_envelope(
                outgoing_tx,
                RelayEnvelope::Close {
                    reason: "relay configuration changed after peer response".to_string(),
                },
            )?;
            Ok(Some(reason))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_response_precedes_dynamic_relay_reconnect() {
        let (outgoing_tx, mut priority_rx, _event_rx) = RelayOutgoingSender::channel(2);
        send_outgoing_envelope(
            &outgoing_tx,
            RelayEnvelope::DaemonIncomingPeerResponse {
                relay_request_id: "install-token".to_string(),
                encrypted_response: None,
                error: None,
            },
        )
        .expect("peer response should enqueue");
        let mut config = crate::config::DaemonConfig::for_tests();
        config.relay_url = Some("wss://relay.example.test".to_string());
        config.relay_token = Some("new-token".to_string());

        let reason = enqueue_dynamic_relay_reconnect_if_changed(
            &outgoing_tx,
            "wss://relay.example.test",
            "bootstrap-token",
            &config,
        )
        .expect("reconnect should enqueue");

        assert_eq!(reason, Some("relay token changed"));
        assert!(matches!(
            priority_rx.try_recv(),
            Ok(RelayEnvelope::DaemonIncomingPeerResponse { .. })
        ));
        assert!(matches!(
            priority_rx.try_recv(),
            Ok(RelayEnvelope::Close { .. })
        ));
    }
}
