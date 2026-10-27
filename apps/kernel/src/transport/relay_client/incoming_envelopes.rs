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
    pub local_browser: &'a Arc<LocalBrowserDirect>,
}

#[derive(Debug, Default)]
pub(super) struct RelayReconnectGate {
    state: std::sync::Mutex<RelayReconnectGateState>,
    input_order: super::browser_input_order::BrowserInputOrder,
}

#[derive(Debug, Default)]
struct RelayReconnectGateState {
    active_daemon_requests: usize,
    pending_reconnect: Option<&'static str>,
    closing: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum RelayReconnectDecision {
    Unchanged,
    Deferred(&'static str),
    Queued(&'static str),
}

impl RelayReconnectGate {
    fn begin_daemon_request(&self) -> bool {
        let mut state = self.state.lock().expect("relay reconnect gate poisoned");
        // A pending reconnect admits no new work, so in-flight requests drain.
        if state.closing || state.pending_reconnect.is_some() {
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

    pub(super) fn request_reconnect(
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
        local_browser,
    } = context;
    let envelope = serde_json::from_str::<RelayEnvelope>(payload).map_err(|error| {
        DaemonError::LocalTransport {
            operation: "parse relay envelope",
            message: error.to_string(),
        }
    })?;
    let timing_packet = chariox_relay::transport_timing::envelope_signature(&envelope);
    chariox_relay::transport_timing::record("kernel_receive", timing_packet);
    match envelope {
        RelayEnvelope::DaemonRequest {
            relay_request_id,
            caller_identity,
            encrypted_request,
        } => {
            let timing_packet = super::request_timing::packet(&encrypted_request);
            super::request_timing::record("kernel_receive", timing_packet);
            if !reconnect_gate.begin_daemon_request() {
                // MP-08/MP-10/MP-11: never leave a new client request hanging
                // behind the retained bulk reply. Reuse the relay's own
                // not-admitted code, which clients already retry.
                outgoing_tx
                    .send(RelayEnvelope::DaemonResponse {
                        relay_request_id,
                        encrypted_response: None,
                        error: Some(super::request_errors::relay_error(
                            "target_not_connected",
                            "kernel relay connection is reconnecting",
                            true,
                        )),
                    })
                    .await;
                return Ok(());
            }
            let router = Arc::clone(router);
            let command_sequence = Arc::clone(command_sequence);
            let outgoing_tx = outgoing_tx.clone();
            let command_result_cache = Arc::clone(command_result_cache);
            let reconnect_gate = Arc::clone(reconnect_gate);
            let display_subscriptions = Arc::clone(subscription_tasks);
            // MP-08/MP-10/MP-11: decrypt once in socket order, then reserve
            // input before spawned tasks can overtake one another. ACK/capture
            // requests keep the existing independent concurrent path.
            let display_sender = encrypted_request.sender_public_key.clone();
            let mut prepared = prepare_daemon_request(&router, caller_identity, encrypted_request);
            let (mut turn, mut read) = (None, false);
            if let Some((key, mirror_read)) =
                prepared.as_ref().ok().and_then(|p| p.browser_input_key())
            {
                read = mirror_read;
                turn = reconnect_gate.input_order.reserve(key);
                if turn.is_none() {
                    prepared = Err(Box::new(super::daemon_requests::RelayRequestOutcome {
                        display_event: None,
                        encrypted_response: None,
                        error: Some(super::request_errors::relay_error(
                            "request_busy",
                            "MP-08: browser input queue full",
                            false,
                        )),
                    }));
                }
            }
            let timing_input = turn.is_some();
            if timing_input {
                chariox_relay::transport_timing::record("kernel_input_queued", timing_packet);
            }
            let local_browser = Arc::clone(local_browser);
            tokio::spawn(async move {
                let turn = match turn {
                    Some(turn) => turn.enter(read).await,
                    None => None,
                };
                let mut relay_response = super::browser_input_order::serve_holding(turn, async {
                    match prepared {
                        Ok(prepared) => {
                            handle_prepared_daemon_request(
                                &router,
                                &command_sequence,
                                prepared,
                                &command_result_cache,
                                &display_subscriptions,
                                Some(&local_browser),
                            )
                            .await
                        }
                        Err(outcome) => *outcome,
                    }
                })
                .await;
                chariox_relay::transport_timing::record("kernel_request_end", timing_packet);
                if timing_input {
                    chariox_relay::transport_timing::record("kernel_input_end", timing_packet);
                }
                if let Some((display_id, event_id, encrypted_event)) = relay_response.display_event
                {
                    let display_at = std::time::Instant::now();
                    if let Some(subscription_id) =
                        super::browser_display::browser_display_delivery_id(
                            &display_subscriptions,
                            &display_id,
                            &display_sender,
                        )
                        .await
                    {
                        if outgoing_tx
                            .send_display_event(RelayEnvelope::DaemonEvent {
                                subscription_id,
                                event_id,
                                encrypted_event,
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                        crate::transport::kernel_browser_display::timing(
                            "event_queue_credit",
                            display_at,
                        );
                    } else {
                        relay_response.encrypted_response = None;
                        relay_response.error = Some(super::request_errors::relay_error("display_subscription_required", "MD-DISPLAY: register a fresh display subscription with the same sender identity", false));
                    }
                }
                if let Err(error) = super::envelope_io::send_outgoing_reply(
                    &outgoing_tx,
                    RelayEnvelope::DaemonResponse {
                        relay_request_id,
                        encrypted_response: relay_response.encrypted_response,
                        error: relay_response.error,
                    },
                )
                .await
                {
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
            // MP-08/MP-10/MP-11: peer launches and captures share the reconnect fence.
            if !reconnect_gate.begin_daemon_request() {
                outgoing_tx
                    .send(RelayEnvelope::DaemonIncomingPeerResponse {
                        relay_request_id,
                        encrypted_response: None,
                        error: Some(super::request_errors::relay_error(
                            "target_not_connected",
                            "kernel relay connection is reconnecting",
                            true,
                        )),
                    })
                    .await;
                return Ok(());
            }
            let router = Arc::clone(router);
            let state = Arc::clone(state);
            let outgoing_tx = outgoing_tx.clone();
            let reconnect_gate = Arc::clone(reconnect_gate);
            let active_dynamic_relay = active_dynamic_relay
                .map(|(relay_url, relay_token)| (relay_url.to_string(), relay_token.to_string()));
            tokio::spawn(async move {
                #[cfg(test)]
                let response_kind = test_peer_response_kind(&router, &encrypted_request);
                // Keep the large peer request future off the worker stack.
                let relay_response = Box::pin(handle_daemon_peer_request(
                    &router,
                    &state,
                    &outgoing_tx,
                    &from_daemon_id,
                    caller_identity,
                    encrypted_request,
                ))
                .await;
                #[cfg(test)]
                let relay_response = {
                    let mut relay_response = relay_response;
                    if let Some(forget_receipts) = state
                        .write()
                        .await
                        .test_take_lost_peer_response_payload(response_kind)
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
                if let Err(error) = super::envelope_io::send_outgoing_reply(
                    &outgoing_tx,
                    RelayEnvelope::DaemonIncomingPeerResponse {
                        relay_request_id,
                        encrypted_response: relay_response.encrypted_response,
                        error: relay_response.error,
                    },
                )
                .await
                {
                    crate::logging::warn_with_fields(
                        "daemon.relay_client",
                        "failed to send async daemon peer response",
                        serde_json::json!({
                            "error": error.to_string(),
                        }),
                    );
                    return;
                }
                if let Err(error) = reconnect_gate.finish_daemon_request(&outgoing_tx) {
                    crate::logging::warn_with_fields(
                        "daemon.relay_client",
                        "failed to finish peer reply reconnect fence",
                        serde_json::json!({"error": error.to_string()}),
                    );
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
            from_daemon_id,
            caller_identity,
            encrypted_event,
        } => {
            let router = Arc::clone(router);
            let state = Arc::clone(state);
            tokio::spawn(async move {
                if let Err(error) = handle_daemon_peer_event(
                    &router,
                    &state,
                    &from_daemon_id,
                    caller_identity,
                    encrypted_event,
                )
                .await
                {
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
            heartbeat_interval_ms,
        } => {
            if chariox_relay::frame_transport::is_display_scope(subscription_scope.as_deref()) {
                return super::browser_display::enqueue_subscribe(
                    router,
                    outgoing_tx,
                    subscription_tasks,
                    event_runtime,
                    super::browser_display::DisplaySubscription {
                        request_id: relay_request_id,
                        relay_id: relay_subscription_id,
                        display_id: session_id,
                        generation: attachment_id,
                        identity: caller_identity,
                        public_key: client_public_key,
                        resume: resume_from_event_id,
                        recover: subscription_scope.as_deref()
                            == Some(chariox_relay::frame_transport::DISPLAY_RESUME_SCOPE),
                    },
                )
                .await;
            }
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
                heartbeat_interval_ms,
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
            super::display_ingress::forward_display_client_event(
                state,
                &stream_id,
                RelayDisplayTunnelClientEvent::Chunk(chunk),
            )
            .await;
        }
        RelayEnvelope::DaemonDisplayTunnelClientClose { stream_id, error } => {
            if let Some(error) = error {
                crate::logging::warn_with_fields(
                    "display.proxy",
                    "relay closed display client",
                    serde_json::json!({"stream_id": stream_id, "code": error.code}),
                );
            }
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
        // MP-08/MP-10/MP-11: the connector renews a rotated token in-band with
        // DaemonHeartbeat. Closing here dropped every client subscription.
        RelayConfigContinuity::Continue | RelayConfigContinuity::Reauthenticate => {
            Ok(RelayReconnectDecision::Unchanged)
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
    if outgoing_tx.send_after_queued_events(RelayEnvelope::Close {
        reason: "relay configuration changed after peer response".to_string(),
    }) {
        return Ok(());
    }
    Err(DaemonError::LocalTransport {
        operation: "send relay envelope",
        message: "relay connection writer is closed".to_string(),
    })
}

#[cfg(test)]
fn test_peer_response_kind(
    router: &CommandRouter,
    request: &chariox_relay::protocol::EncryptedRelayPayload,
) -> super::connection_state::TestPeerResponseKind {
    use super::connection_state::TestPeerResponseKind;
    use crate::transport::relay_peer::RelayPeerRequest;
    use crate::transport::room_browser_controller::RoomBrowserControllerCommand;
    crate::transport::relay_crypto::decrypt_payload_for_private_key(
        &router.relay_private_key(),
        request,
    )
    .ok()
    .and_then(|payload| serde_json::from_slice::<RelayPeerRequest>(&payload.plaintext).ok())
    .map(|request| match request {
        RelayPeerRequest::RoomBrowserController {
            command:
                RoomBrowserControllerCommand::Tab { .. }
                | RoomBrowserControllerCommand::History { .. }
                | RoomBrowserControllerCommand::Navigate { .. }
                | RoomBrowserControllerCommand::Dialog { .. }
                | RoomBrowserControllerCommand::Action { .. }
                | RoomBrowserControllerCommand::Upload { .. }
                | RoomBrowserControllerCommand::Permission { .. }
                | RoomBrowserControllerCommand::ConfigureDownloads { .. },
            ..
        } => TestPeerResponseKind::BrowserMutation,
        RelayPeerRequest::SubmitLeasedPrompt { .. } => TestPeerResponseKind::LeasedPrompt,
        RelayPeerRequest::SteerLeasedPrompt { .. } => TestPeerResponseKind::LeasedSteer,
        _ => TestPeerResponseKind::Other,
    })
    .unwrap_or(TestPeerResponseKind::Other)
}

#[cfg(test)]
mod tests {
    use super::*;
    // #958 review @a81084519 P2: the writer drains priority before events, so
    // a deferred reconnect Close must follow a queued bulk reply.
    #[test]
    fn deferred_reconnect_close_follows_a_queued_bulk_reply() {
        let (outgoing_tx, mut priority_rx, mut event_rx) = RelayOutgoingSender::channel(4);
        let gate = RelayReconnectGate::default();
        assert!(gate.begin_daemon_request());
        assert_eq!(
            gate.request_reconnect(&outgoing_tx, "relay token changed")
                .unwrap(),
            RelayReconnectDecision::Deferred("relay token changed")
        );
        let payload = chariox_relay::protocol::EncryptedRelayPayload {
            sender_public_key: "sender".into(),
            nonce: "nonce".into(),
            ciphertext: "x".repeat(64 * 1024 + 1),
        };
        send_outgoing_envelope(
            &outgoing_tx,
            RelayEnvelope::DaemonResponse {
                relay_request_id: "history".into(),
                encrypted_response: Some(payload),
                error: None,
            },
        )
        .unwrap();
        assert_eq!(
            gate.finish_daemon_request(&outgoing_tx).unwrap(),
            Some("relay token changed")
        );
        // Biased writer order: every queued priority envelope, then events.
        let mut written = Vec::new();
        while let Ok(envelope) = priority_rx.try_recv() {
            written.push(envelope);
        }
        while let Ok(envelope) = event_rx.try_recv() {
            written.push(envelope);
        }
        assert!(
            matches!(written.first(), Some(RelayEnvelope::DaemonResponse { .. })),
            "the bulk reply must be written before Close: {written:?}"
        );
        assert!(matches!(written.get(1), Some(RelayEnvelope::Close { .. })));
    }

    // MP-08/MP-10/MP-11: close must not drain a display slideshow first, but
    // leased projections already committed by the pump must still be written.
    #[test]
    fn reconnect_discards_queued_display_events_but_preserves_peer_events_and_bulk_replies() {
        let (outgoing_tx, _priority_rx, mut event_rx) = RelayOutgoingSender::channel(4);
        let payload = chariox_relay::protocol::EncryptedRelayPayload {
            sender_public_key: "sender".into(),
            nonce: "nonce".into(),
            ciphertext: "x".repeat(64 * 1024 + 1),
        };
        outgoing_tx
            .try_send(RelayEnvelope::DaemonEvent {
                subscription_id: "display".into(),
                event_id: 1,
                encrypted_event: payload.clone(),
            })
            .unwrap();
        outgoing_tx
            .try_send(RelayEnvelope::DaemonPeerEvent {
                target: ClientTarget {
                    daemon_id: Some("home".into()),
                    daemon_alias: None,
                },
                encrypted_event: payload.clone(),
            })
            .unwrap();
        outgoing_tx
            .try_send(RelayEnvelope::DaemonResponse {
                relay_request_id: "history".into(),
                encrypted_response: Some(payload),
                error: None,
            })
            .unwrap();
        RelayReconnectGate::default()
            .request_reconnect(&outgoing_tx, "config changed")
            .unwrap();
        let written: Vec<_> = std::iter::from_fn(|| event_rx.try_recv().ok())
            .filter(|envelope| outgoing_tx.should_write(envelope))
            .collect();
        assert!(
            matches!(
                written.as_slice(),
                [
                    RelayEnvelope::DaemonPeerEvent { .. },
                    RelayEnvelope::DaemonResponse { .. },
                    RelayEnvelope::Close { .. },
                ]
            ),
            "only display events may be discarded: {written:?}"
        );
    }

    // MP-08/MP-10/MP-11: a retained bulk reply must not silence new controls,
    // and a pending reconnect admits no new work so in-flight requests drain.
    // The refusal reuses the relay's own not-admitted code.
    #[tokio::test]
    async fn closing_or_draining_connection_refuses_daemon_and_peer_requests() {
        for draining in [false, true] {
            refuses_new_requests_while_reconnecting(draining).await;
        }
    }

    async fn refuses_new_requests_while_reconnecting(draining: bool) {
        let app = Arc::new(tokio::sync::Mutex::new(
            crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap(),
        ));
        let router = Arc::new(CommandRouter::with_interactive_capacity(app, 2));
        let state = Arc::new(RwLock::new(RelayClientState::default()));
        let sequence = Arc::new(AtomicU64::new(0));
        let tasks = Arc::new(Mutex::new(BTreeMap::new()));
        let events = Arc::new(RelayEventRuntime::for_tests(1));
        let cache = Arc::new(CommandResultCache::default());
        let gate = Arc::new(RelayReconnectGate::default());
        let (_shutdown, shutdown) = watch::channel(false);
        let local_browser = LocalBrowserDirect::new(
            router.clone(),
            sequence.clone(),
            events.clone(),
            cache.clone(),
            shutdown,
        );
        let (outgoing, mut priority, mut event_lane) = RelayOutgoingSender::channel(4);
        if draining {
            assert!(gate.begin_daemon_request());
        }
        gate.request_reconnect(&outgoing, "config changed").unwrap();
        let payload = chariox_relay::protocol::EncryptedRelayPayload {
            sender_public_key: "unused".into(),
            nonce: "unused".into(),
            ciphertext: "unused".into(),
        };
        for request in [
            RelayEnvelope::DaemonRequest {
                relay_request_id: "control".into(),
                caller_identity: None,
                encrypted_request: payload.clone(),
            },
            RelayEnvelope::DaemonIncomingPeerRequest {
                relay_request_id: "peer".into(),
                from_daemon_id: "home".into(),
                caller_identity: None,
                encrypted_request: payload,
            },
        ] {
            handle_incoming_envelope(
                IncomingEnvelopeContext {
                    router: &router,
                    command_sequence: &sequence,
                    state: &state,
                    outgoing_tx: &outgoing,
                    subscription_tasks: &tasks,
                    event_runtime: &events,
                    command_result_cache: &cache,
                    reconnect_gate: &gate,
                    local_browser: &local_browser,
                },
                None,
                &serde_json::to_string(&request).unwrap(),
            )
            .await
            .unwrap();
            let reply = priority
                .try_recv()
                .expect("closing requests get an immediate reply");
            let error = match reply {
                RelayEnvelope::DaemonResponse {
                    error: Some(error), ..
                }
                | RelayEnvelope::DaemonIncomingPeerResponse {
                    error: Some(error), ..
                } => error,
                other => panic!("expected reconnect refusal (draining={draining}): {other:?}"),
            };
            assert_eq!(error.code, "target_not_connected", "draining={draining}");
            assert!(error.retryable);
        }
        if draining {
            assert!(
                event_lane.try_recv().is_err(),
                "Close waits for in-flight work"
            );
            assert_eq!(
                gate.finish_daemon_request(&outgoing).unwrap(),
                Some("config changed")
            );
            assert!(matches!(
                event_lane.try_recv(),
                Ok(RelayEnvelope::Close { .. })
            ));
        }
    }

    #[test]
    fn token_rotation_does_not_close_after_peer_or_daemon_response() {
        // MP-08 / MP-10 / MP-11: Computer tools and grants use this response path.
        // The connector renews credentials by heartbeat on the existing socket.
        for active in [false, true] {
            let (outgoing_tx, mut priority_rx, _event_rx) = RelayOutgoingSender::channel(2);
            let gate = RelayReconnectGate::default();
            if active {
                assert!(gate.begin_daemon_request());
            }
            let mut config = crate::config::DaemonConfig::for_tests();
            config.relay_url = Some("wss://relay.example.test".into());
            config.relay_token = Some("renewed-token".into());
            assert_eq!(
                enqueue_dynamic_relay_reconnect_if_changed(
                    &outgoing_tx,
                    &gate,
                    "wss://relay.example.test",
                    "old-token",
                    &config,
                )
                .unwrap(),
                RelayReconnectDecision::Unchanged
            );
            if active {
                assert_eq!(gate.finish_daemon_request(&outgoing_tx).unwrap(), None);
            }
            assert!(
                priority_rx.try_recv().is_err(),
                "renewal must retain display and peer routes"
            );
        }
    }

    // MP-08/MP-10/MP-11: hosted soaks lost every browser subscription when a
    // response finished after a Cloud token renewal. Rotation is renewed in-band.
    #[test]
    fn token_rotation_after_response_keeps_the_socket_for_in_band_renewal() {
        let (outgoing_tx, mut priority_rx, _event_rx) = RelayOutgoingSender::channel(1);
        let reconnect_gate = RelayReconnectGate::default();
        let mut config = crate::config::DaemonConfig::for_tests();
        config.relay_url = Some("wss://relay.example.test".to_string());
        config.relay_token = Some("renewed-token".to_string());

        let decision = enqueue_dynamic_relay_reconnect_if_changed(
            &outgoing_tx,
            &reconnect_gate,
            "wss://relay.example.test",
            "previous-token",
            &config,
        )
        .expect("renewal decision should succeed");

        assert_eq!(decision, RelayReconnectDecision::Unchanged);
        assert!(priority_rx.try_recv().is_err());
        assert!(reconnect_gate.begin_daemon_request());
    }

    // MP-08 / MP-10 / MP-11: token renewal must reuse existing heartbeat reauthentication.
    #[test]
    fn token_rollover_after_peer_response_preserves_live_socket_and_admission() {
        // MP-08/MP-10/MP-11: a peer reply may finish with a token snapshot
        // captured before renewal. Neither idle nor busy clients may be closed.
        for active_requests in 0..=2 {
            let (outgoing_tx, mut priority_rx, _event_rx) = RelayOutgoingSender::channel(2);
            let gate = RelayReconnectGate::default();
            for _ in 0..active_requests {
                assert!(gate.begin_daemon_request());
            }
            let mut config = crate::config::DaemonConfig::for_tests();
            config.relay_url = Some("wss://relay.example.test".into());
            config.relay_token = Some("renewed-token".into());
            assert_eq!(
                enqueue_dynamic_relay_reconnect_if_changed(
                    &outgoing_tx,
                    &gate,
                    "wss://relay.example.test",
                    "old-token",
                    &config,
                )
                .unwrap(),
                RelayReconnectDecision::Unchanged,
                "token renewal must use the existing heartbeat reauthentication path",
            );
            for _ in 0..active_requests {
                assert_eq!(gate.finish_daemon_request(&outgoing_tx).unwrap(), None);
            }
            assert!(
                priority_rx.try_recv().is_err(),
                "renewal must not queue Close"
            );
            assert!(
                gate.begin_daemon_request(),
                "new requests must remain admitted"
            );
            assert_eq!(gate.finish_daemon_request(&outgoing_tx).unwrap(), None);
        }
    }

    #[test]
    fn dynamic_relay_reconnect_waits_for_in_flight_daemon_response() {
        let (outgoing_tx, mut priority_rx, mut event_rx) = RelayOutgoingSender::channel(2);
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
        config.relay_url = Some("wss://moved-relay.example.test".to_string());
        config.relay_token = Some("bootstrap-token".to_string());

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
            RelayReconnectDecision::Deferred("relay url or token missing changed")
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

        assert_eq!(deferred_reason, "relay url or token missing changed");
        assert!(matches!(
            priority_rx.try_recv(),
            Ok(RelayEnvelope::DaemonResponse { .. })
        ));
        assert!(matches!(
            event_rx.try_recv(),
            Ok(RelayEnvelope::Close { .. })
        ));
    }

    #[test]
    fn dynamic_relay_reconnect_is_immediate_without_active_daemon_requests() {
        let (outgoing_tx, _priority_rx, mut event_rx) = RelayOutgoingSender::channel(1);
        let reconnect_gate = RelayReconnectGate::default();
        let mut config = crate::config::DaemonConfig::for_tests();
        config.relay_url = Some("wss://moved-relay.example.test".to_string());
        config.relay_token = Some("bootstrap-token".to_string());

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
            RelayReconnectDecision::Queued("relay url or token missing changed")
        );
        assert!(matches!(
            event_rx.try_recv(),
            Ok(RelayEnvelope::Close { .. })
        ));
    }

    #[test]
    fn dynamic_relay_reconnect_waits_for_every_active_daemon_request() {
        let reconnect_gate = RelayReconnectGate::default();
        let (outgoing_tx, _priority_rx, mut event_rx) = RelayOutgoingSender::channel(1);
        assert!(reconnect_gate.begin_daemon_request());
        assert!(reconnect_gate.begin_daemon_request());

        assert_eq!(
            reconnect_gate
                .request_reconnect(&outgoing_tx, "relay url changed")
                .expect("reconnect should defer"),
            RelayReconnectDecision::Deferred("relay url changed"),
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
            Some("relay url changed")
        );
        assert!(matches!(
            event_rx.try_recv(),
            Ok(RelayEnvelope::Close { .. })
        ));
    }

    #[test]
    fn daemon_request_cannot_enter_after_deferred_reconnect_is_released() {
        let (outgoing_tx, mut priority_rx, mut event_rx) = RelayOutgoingSender::channel(2);
        let reconnect_gate = RelayReconnectGate::default();
        assert!(reconnect_gate.begin_daemon_request());
        assert_eq!(
            reconnect_gate
                .request_reconnect(&outgoing_tx, "relay url changed")
                .expect("reconnect should defer"),
            RelayReconnectDecision::Deferred("relay url changed"),
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
            Some("relay url changed")
        );
        assert!(!reconnect_gate.begin_daemon_request());
        assert!(matches!(
            priority_rx.try_recv(),
            Ok(RelayEnvelope::DaemonResponse { .. })
        ));
        assert!(matches!(
            event_rx.try_recv(),
            Ok(RelayEnvelope::Close { .. })
        ));
    }
}
