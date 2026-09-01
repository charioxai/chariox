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
            caller_identity: _,
            encrypted_request,
        } => {
            let router = Arc::clone(router);
            let state = Arc::clone(state);
            let outgoing_tx = outgoing_tx.clone();
            tokio::spawn(async move {
                let relay_response = handle_daemon_peer_request(
                    &router,
                    &state,
                    &outgoing_tx,
                    &from_daemon_id,
                    encrypted_request,
                )
                .await;
                #[cfg(test)]
                let relay_response = {
                    let mut relay_response = relay_response;
                    if let Some(forget_receipts) =
                        state.write().await.test_take_lost_peer_response_payload()
                    {
                        if forget_receipts {
                            router
                                .runtime_state()
                                .test_forget_completed_browser_action_receipts();
                        }
                        relay_response.encrypted_response = None;
                        relay_response.error = None;
                    }
                    relay_response
                };
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
            let daemon_private_key = router.relay_private_key();
            tokio::spawn(async move {
                handle_display_tunnel_open(state, outgoing_tx, request, daemon_private_key).await;
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
            let stream_id = chunk.stream_id.clone();
            state.write().await.try_send_display_stream_event(
                &stream_id,
                RelayDisplayTunnelClientEvent::Chunk(chunk),
            );
        }
        RelayEnvelope::DaemonDisplayTunnelClientClose { stream_id, .. } => {
            state
                .write()
                .await
                .try_send_display_stream_event(&stream_id, RelayDisplayTunnelClientEvent::Close);
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
