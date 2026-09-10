//! Incoming relay envelope dispatch for daemon, peer, and subscription traffic.

use super::*;

pub(super) struct IncomingEnvelopeContext<'a> {
    pub router: &'a Arc<CommandRouter>,
    pub command_sequence: &'a Arc<AtomicU64>,
    pub state: &'a Arc<RwLock<RelayClientState>>,
    pub outgoing_tx: &'a RelayOutgoingSender,
    pub subscription_tasks: &'a RelaySubscriptionTasks,
    pub event_runtime: &'a Arc<RelayEventRuntime>,
    pub command_result_cache: &'a RelayCommandResultCache,
    pub reconnect_gate: &'a Arc<RelayReconnectGate>,
}

#[derive(Debug, Default)]
pub(super) struct RelayReconnectGate {
    state: std::sync::Mutex<RelayReconnectGateState>,
}

#[derive(Debug, Default)]
struct RelayReconnectGateState {
    active_daemon_requests: usize,
    pending_reconnect: Option<&'static str>,
    closing: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum RelayReconnectDecision {
    Unchanged,
    Deferred(&'static str),
    Queued(&'static str),
}

impl RelayReconnectGate {
    fn begin_daemon_request(&self) -> bool {
        let mut state = self.state.lock().expect("relay reconnect gate poisoned");
        if state.closing {
            return false;
        }
        state.active_daemon_requests = state.active_daemon_requests.saturating_add(1);
        true
    }

    fn finish_daemon_request(
        &self,
        outgoing_tx: &RelayOutgoingSender,
    ) -> Result<Option<&'static str>, DaemonError> {
        let mut state = self.state.lock().expect("relay reconnect gate poisoned");
        debug_assert!(state.active_daemon_requests > 0);
        state.active_daemon_requests = state.active_daemon_requests.saturating_sub(1);
        if state.active_daemon_requests != 0 {
            return Ok(None);
        }
        let Some(reason) = state.pending_reconnect else {
            return Ok(None);
        };
        enqueue_relay_close(outgoing_tx)?;
        state.pending_reconnect = None;
        state.closing = true;
        Ok(Some(reason))
    }

    fn request_reconnect(
        &self,
        outgoing_tx: &RelayOutgoingSender,
        reason: &'static str,
    ) -> Result<RelayReconnectDecision, DaemonError> {
        let mut state = self.state.lock().expect("relay reconnect gate poisoned");
        if state.closing {
            return Ok(RelayReconnectDecision::Queued(reason));
        }
        if state.active_daemon_requests == 0 {
            enqueue_relay_close(outgoing_tx)?;
            state.closing = true;
            Ok(RelayReconnectDecision::Queued(reason))
        } else {
            state.pending_reconnect.get_or_insert(reason);
            Ok(RelayReconnectDecision::Deferred(reason))
        }
    }
}

pub(super) async fn handle_incoming_envelope(
    context: IncomingEnvelopeContext<'_>,
    active_dynamic_relay: Option<(&str, &str)>,
    payload: &str,
) -> Result<(), DaemonError> {
    let IncomingEnvelopeContext {
        router,
        command_sequence,
        state,
        outgoing_tx,
        subscription_tasks,
        event_runtime,
        command_result_cache,
        reconnect_gate,
    } = context;
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
            if !reconnect_gate.begin_daemon_request() {
                crate::logging::info_with_fields(
                    "daemon.relay_client",
                    "ignored daemon request while relay reconnect is queued",
                    serde_json::json!({
                        "relay_request_id": relay_request_id,
                    }),
                );
                return Ok(());
            }
            let router = Arc::clone(router);
            let command_sequence = Arc::clone(command_sequence);
            let outgoing_tx = outgoing_tx.clone();
            let command_result_cache = Arc::clone(command_result_cache);
            let reconnect_gate = Arc::clone(reconnect_gate);
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
                match reconnect_gate.finish_daemon_request(&outgoing_tx) {
                    Ok(Some(reason)) => crate::logging::info_with_fields(
                        "daemon.relay_client",
                        "relay reconnect queued after daemon response",
                        serde_json::json!({
                            "reason": reason,
                        }),
                    ),
                    Ok(None) => {}
                    Err(error) => crate::logging::warn_with_fields(
                        "daemon.relay_client",
                        "failed to queue relay reconnect after daemon response",
                        serde_json::json!({
                            "error": error.to_string(),
                        }),
                    ),
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
            let reconnect_gate = Arc::clone(reconnect_gate);
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
                    return;
                }
                if let Some((active_relay_url, active_relay_token)) = active_dynamic_relay.as_ref()
                {
                    match enqueue_dynamic_relay_reconnect_if_changed(
                        &outgoing_tx,
                        &reconnect_gate,
                        active_relay_url,
                        active_relay_token,
                        &router.relay_config_snapshot(),
                    ) {
                        Ok(RelayReconnectDecision::Queued(reason)) => {
                            crate::logging::info_with_fields(
                                "daemon.relay_client",
                                "relay reconnect queued after peer response",
                                serde_json::json!({
                                    "reason": reason,
                                }),
                            )
                        }
                        Ok(RelayReconnectDecision::Deferred(reason)) => {
                            crate::logging::info_with_fields(
                                "daemon.relay_client",
                                "relay reconnect deferred until daemon responses complete",
                                serde_json::json!({
                                    "reason": reason,
                                }),
                            )
                        }
                        Ok(RelayReconnectDecision::Unchanged) => {}
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

fn enqueue_dynamic_relay_reconnect_if_changed(
    outgoing_tx: &RelayOutgoingSender,
    reconnect_gate: &RelayReconnectGate,
    active_relay_url: &str,
    active_relay_token: &str,
    config: &crate::config::DaemonConfig,
) -> Result<RelayReconnectDecision, DaemonError> {
    match relay_config_continuity(active_relay_url, active_relay_token, config) {
        RelayConfigContinuity::Continue => Ok(RelayReconnectDecision::Unchanged),
        RelayConfigContinuity::Reauthenticate => {
            defer_or_enqueue_relay_close(outgoing_tx, reconnect_gate, "relay token changed")
        }
        RelayConfigContinuity::Reconnect(reason) => {
            defer_or_enqueue_relay_close(outgoing_tx, reconnect_gate, reason)
        }
    }
}

fn defer_or_enqueue_relay_close(
    outgoing_tx: &RelayOutgoingSender,
    reconnect_gate: &RelayReconnectGate,
    reason: &'static str,
) -> Result<RelayReconnectDecision, DaemonError> {
    reconnect_gate.request_reconnect(outgoing_tx, reason)
}

fn enqueue_relay_close(outgoing_tx: &RelayOutgoingSender) -> Result<(), DaemonError> {
    send_outgoing_envelope(
        outgoing_tx,
        RelayEnvelope::Close {
            reason: "relay configuration changed after peer response".to_string(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dynamic_relay_reconnect_waits_for_in_flight_daemon_response() {
        let (outgoing_tx, mut priority_rx, _event_rx) = RelayOutgoingSender::channel(2);
        let reconnect_gate = RelayReconnectGate::default();
        assert!(reconnect_gate.begin_daemon_request());
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
            &reconnect_gate,
            "wss://relay.example.test",
            "bootstrap-token",
            &config,
        )
        .expect("reconnect should enqueue");

        assert_eq!(
            reason,
            RelayReconnectDecision::Deferred("relay token changed")
        );
        assert!(matches!(
            priority_rx.try_recv(),
            Ok(RelayEnvelope::DaemonIncomingPeerResponse { .. })
        ));
        assert!(priority_rx.try_recv().is_err());

        send_outgoing_envelope(
            &outgoing_tx,
            RelayEnvelope::DaemonResponse {
                relay_request_id: "start-slice".to_string(),
                encrypted_response: None,
                error: None,
            },
        )
        .expect("daemon response should enqueue");
        let deferred_reason = reconnect_gate
            .finish_daemon_request(&outgoing_tx)
            .expect("deferred reconnect should enqueue")
            .expect("last daemon response should release deferred reconnect");

        assert_eq!(deferred_reason, "relay token changed");
        assert!(matches!(
            priority_rx.try_recv(),
            Ok(RelayEnvelope::DaemonResponse { .. })
        ));
        assert!(matches!(
            priority_rx.try_recv(),
            Ok(RelayEnvelope::Close { .. })
        ));
    }

    #[test]
    fn dynamic_relay_reconnect_is_immediate_without_active_daemon_requests() {
        let (outgoing_tx, mut priority_rx, _event_rx) = RelayOutgoingSender::channel(1);
        let reconnect_gate = RelayReconnectGate::default();
        let mut config = crate::config::DaemonConfig::for_tests();
        config.relay_url = Some("wss://relay.example.test".to_string());
        config.relay_token = Some("new-token".to_string());

        let decision = enqueue_dynamic_relay_reconnect_if_changed(
            &outgoing_tx,
            &reconnect_gate,
            "wss://relay.example.test",
            "bootstrap-token",
            &config,
        )
        .expect("idle reconnect should enqueue");

        assert_eq!(
            decision,
            RelayReconnectDecision::Queued("relay token changed")
        );
        assert!(matches!(
            priority_rx.try_recv(),
            Ok(RelayEnvelope::Close { .. })
        ));
    }

    #[test]
    fn dynamic_relay_reconnect_waits_for_every_active_daemon_request() {
        let reconnect_gate = RelayReconnectGate::default();
        let (outgoing_tx, mut priority_rx, _event_rx) = RelayOutgoingSender::channel(1);
        assert!(reconnect_gate.begin_daemon_request());
        assert!(reconnect_gate.begin_daemon_request());

        assert_eq!(
            reconnect_gate
                .request_reconnect(&outgoing_tx, "relay token changed")
                .expect("reconnect should defer"),
            RelayReconnectDecision::Deferred("relay token changed"),
        );
        assert_eq!(
            reconnect_gate
                .finish_daemon_request(&outgoing_tx)
                .expect("first request should finish"),
            None
        );
        assert_eq!(
            reconnect_gate
                .finish_daemon_request(&outgoing_tx)
                .expect("last request should queue reconnect"),
            Some("relay token changed")
        );
        assert!(matches!(
            priority_rx.try_recv(),
            Ok(RelayEnvelope::Close { .. })
        ));
    }

    #[test]
    fn daemon_request_cannot_enter_after_deferred_reconnect_is_released() {
        let (outgoing_tx, mut priority_rx, _event_rx) = RelayOutgoingSender::channel(2);
        let reconnect_gate = RelayReconnectGate::default();
        assert!(reconnect_gate.begin_daemon_request());
        assert_eq!(
            reconnect_gate
                .request_reconnect(&outgoing_tx, "relay token changed")
                .expect("reconnect should defer"),
            RelayReconnectDecision::Deferred("relay token changed"),
        );

        send_outgoing_envelope(
            &outgoing_tx,
            RelayEnvelope::DaemonResponse {
                relay_request_id: "first-request".to_string(),
                encrypted_response: None,
                error: None,
            },
        )
        .expect("daemon response should enqueue");
        assert_eq!(
            reconnect_gate
                .finish_daemon_request(&outgoing_tx)
                .expect("deferred reconnect should enqueue"),
            Some("relay token changed")
        );
        assert!(!reconnect_gate.begin_daemon_request());
        assert!(matches!(
            priority_rx.try_recv(),
            Ok(RelayEnvelope::DaemonResponse { .. })
        ));
        assert!(matches!(
            priority_rx.try_recv(),
            Ok(RelayEnvelope::Close { .. })
        ));
    }
}
