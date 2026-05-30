//! Incoming relay envelope dispatch for daemon, peer, and subscription traffic.

use super::*;

pub(super) async fn handle_incoming_envelope(
    router: &Arc<CommandRouter>,
    command_sequence: &Arc<AtomicU64>,
    state: &Arc<RwLock<RelayClientState>>,
    outgoing_tx: &RelayOutgoingSender,
    subscription_tasks: &RelaySubscriptionTasks,
    event_runtime: &Arc<RelayEventRuntime>,
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
            tokio::spawn(async move {
                let relay_response = handle_daemon_request(
                    &router,
                    &command_sequence,
                    caller_identity,
                    encrypted_request,
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
            from_daemon_id: _,
            caller_identity: _,
            encrypted_request,
        } => {
            let router = Arc::clone(router);
            let outgoing_tx = outgoing_tx.clone();
            tokio::spawn(async move {
                let relay_response =
                    handle_daemon_peer_request(&router, &outgoing_tx, encrypted_request).await;
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
            if let Err(error) = handle_daemon_peer_event(router, encrypted_event).await {
                crate::logging::warn_with_fields(
                    "daemon.relay_client",
                    "failed to handle relay peer event",
                    serde_json::json!({
                        "error": error.to_string(),
                    }),
                );
            }
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
