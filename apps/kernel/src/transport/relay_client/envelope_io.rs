//! Relay envelope serialization, encryption, and outgoing send helpers.

use super::*;

pub(super) async fn encrypt_json_response(
    router: &Arc<CommandRouter>,
    client_public_key: &str,
    value: serde_json::Value,
) -> Result<EncryptedRelayPayload, DaemonError> {
    let daemon_private_key = router.relay_private_key();
    let plaintext = serde_json::to_vec(&value).map_err(|error| DaemonError::LocalTransport {
        operation: "serialize relay response",
        message: error.to_string(),
    })?;
    relay_crypto::encrypt_payload_for_peer(&daemon_private_key, client_public_key, &plaintext)
}

pub(super) fn encrypt_peer_payload<T: serde::Serialize>(
    sender_private_key: &str,
    peer_public_key: &str,
    value: &T,
) -> Result<EncryptedRelayPayload, DaemonError> {
    let plaintext = serde_json::to_vec(value).map_err(|error| DaemonError::LocalTransport {
        operation: "serialize relay peer payload",
        message: error.to_string(),
    })?;
    relay_crypto::encrypt_payload_for_peer(sender_private_key, peer_public_key, &plaintext)
}

pub(super) fn send_outgoing_envelope(
    outgoing_tx: &RelayOutgoingSender,
    envelope: RelayEnvelope,
) -> Result<(), DaemonError> {
    outgoing_tx.try_send(envelope).map_err(|error| {
        let message = match error {
            tokio::sync::mpsc::error::TrySendError::Full(_) => {
                "relay outgoing queue overloaded".to_string()
            }
            tokio::sync::mpsc::error::TrySendError::Closed(_) => {
                "relay connection writer is closed".to_string()
            }
        };
        DaemonError::LocalTransport {
            operation: "send relay envelope",
            message,
        }
    })
}

pub(super) async fn send_outgoing_event_envelope(
    outgoing_tx: &RelayOutgoingSender,
    envelope: RelayEnvelope,
) -> Result<(), DaemonError> {
    outgoing_tx
        .send_event(envelope)
        .await
        .map_err(|_| DaemonError::LocalTransport {
            operation: "send relay event envelope",
            message: "relay connection writer is closed".to_string(),
        })
}

/// MP-08/MP-10/MP-11: an async daemon or peer reply applies backpressure
/// instead of being dropped when its lane is full.
pub(super) async fn send_outgoing_reply(
    outgoing_tx: &RelayOutgoingSender,
    envelope: RelayEnvelope,
) -> Result<(), DaemonError> {
    if outgoing_tx.send(envelope).await {
        return Ok(());
    }
    Err(DaemonError::LocalTransport {
        operation: "send relay envelope",
        message: "relay connection writer is closed".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bulk_payload() -> chariox_relay::protocol::EncryptedRelayPayload {
        let mut payload = encrypted_payload_for_test();
        payload.ciphertext = "x".repeat(64 * 1024 + 1);
        payload
    }

    // #958 review @a81084519 P2: a saturated event lane must delay a bulk
    // reply (catalogue, history, capture), never reject and lose it.
    #[tokio::test]
    async fn saturated_event_lane_delays_bulk_reply_instead_of_dropping_it() {
        let (sender, _priority, mut events) = RelayOutgoingSender::channel(1);
        send_outgoing_envelope(
            &sender,
            RelayEnvelope::DaemonEvent {
                subscription_id: "subscription".into(),
                event_id: 1,
                encrypted_event: encrypted_payload_for_test(),
            },
        )
        .unwrap();
        let reply = tokio::spawn({
            let sender = sender.clone();
            async move {
                send_outgoing_reply(
                    &sender,
                    RelayEnvelope::DaemonResponse {
                        relay_request_id: "catalogue".into(),
                        encrypted_response: Some(bulk_payload()),
                        error: None,
                    },
                )
                .await
            }
        });
        tokio::task::yield_now().await;
        assert!(matches!(
            events.recv().await,
            Some(RelayEnvelope::DaemonEvent { .. })
        ));
        reply
            .await
            .unwrap()
            .expect("bulk reply must wait for capacity");
        assert!(matches!(
            events.recv().await,
            Some(RelayEnvelope::DaemonResponse { relay_request_id, .. }) if relay_request_id == "catalogue"
        ));
    }

    // #958 review @a81084519 P3: kernels answer peer requests with
    // DaemonIncomingPeerResponse; a large one is bulk too.
    #[test]
    fn large_kernel_peer_reply_uses_the_bulk_lane() {
        let (sender, mut priority, mut events) = RelayOutgoingSender::channel(2);
        send_outgoing_envelope(
            &sender,
            RelayEnvelope::DaemonIncomingPeerResponse {
                relay_request_id: "capture".into(),
                encrypted_response: Some(bulk_payload()),
                error: None,
            },
        )
        .unwrap();
        assert!(priority.try_recv().is_err());
        assert!(matches!(
            events.try_recv(),
            Ok(RelayEnvelope::DaemonIncomingPeerResponse { .. })
        ));
    }

    #[tokio::test]
    async fn md_display_small_events_bypass_batching_but_large_frames_keep_event_credit() {
        let (sender, mut priority, mut events) = RelayOutgoingSender::channel(1);
        let event = |large| {
            let mut payload = encrypted_payload_for_test();
            if large {
                payload.ciphertext = "x".repeat(12 * 1024 + 1);
            }
            RelayEnvelope::DaemonEvent {
                subscription_id: "not-an-authority-prefix".into(),
                event_id: 1,
                encrypted_event: payload,
            }
        };
        sender.send_display_event(event(false)).await.unwrap();
        assert!(priority.try_recv().is_ok());
        assert!(events.try_recv().is_err());
        sender.send_display_event(event(true)).await.unwrap();
        assert!(events.try_recv().is_ok());
        assert!(priority.try_recv().is_err());
        sender.send_display_event(event(false)).await.unwrap();
        assert!(tokio::time::timeout(
            Duration::from_millis(10),
            sender.send_display_event(event(false))
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn bulk_replies_cannot_fill_the_control_priority_queue() {
        let (sender, mut priority, mut events) = RelayOutgoingSender::channel(1);
        let mut payload = encrypted_payload_for_test();
        payload.ciphertext = "x".repeat(64 * 1024 + 1);
        let bulk = RelayEnvelope::DaemonResponse {
            relay_request_id: "catalogue".into(),
            encrypted_response: Some(payload),
            error: None,
        };
        send_outgoing_envelope(&sender, bulk).unwrap();
        assert!(
            priority.try_recv().is_err(),
            "bulk reply occupied control lane"
        );
        let small = RelayEnvelope::DaemonResponse {
            relay_request_id: "computer-start".into(),
            encrypted_response: Some(encrypted_payload_for_test()),
            error: None,
        };
        send_outgoing_envelope(&sender, small).unwrap();
        assert!(priority.try_recv().is_ok());
        assert!(events.try_recv().is_ok());
    }

    #[tokio::test]
    async fn saturated_bulk_queue_preserves_control_and_reports_backpressure() {
        let (sender, mut priority, _events) = RelayOutgoingSender::channel(1);
        let bulk = || {
            let mut payload = encrypted_payload_for_test();
            payload.ciphertext = "x".repeat(64 * 1024 + 1);
            RelayEnvelope::DaemonResponse {
                relay_request_id: "catalogue".into(),
                encrypted_response: Some(payload),
                error: None,
            }
        };
        send_outgoing_envelope(&sender, bulk()).unwrap();
        assert!(send_outgoing_envelope(&sender, bulk()).is_err());
        send_outgoing_envelope(
            &sender,
            RelayEnvelope::DaemonResponse {
                relay_request_id: "computer-start".into(),
                encrypted_response: Some(encrypted_payload_for_test()),
                error: None,
            },
        )
        .unwrap();
        assert!(priority.try_recv().is_ok());
    }

    #[test]
    fn send_outgoing_envelope_fails_when_relay_queue_is_full() {
        let (outgoing_tx, _priority_rx, _event_rx) = RelayOutgoingSender::channel(1);
        send_outgoing_envelope(
            &outgoing_tx,
            RelayEnvelope::Close {
                reason: "first".to_string(),
            },
        )
        .expect("first envelope should fit");

        let error = send_outgoing_envelope(
            &outgoing_tx,
            RelayEnvelope::Close {
                reason: "second".to_string(),
            },
        )
        .expect_err("second envelope should overflow bounded queue");

        assert!(error
            .to_string()
            .contains("relay outgoing queue overloaded"));
    }

    #[test]
    fn send_outgoing_envelope_routes_responses_to_priority_lane() {
        let (outgoing_tx, mut priority_rx, mut event_rx) = RelayOutgoingSender::channel(1);

        send_outgoing_envelope(
            &outgoing_tx,
            RelayEnvelope::DaemonResponse {
                relay_request_id: "request-1".to_string(),
                encrypted_response: None,
                error: None,
            },
        )
        .expect("response should send");

        assert!(matches!(
            priority_rx.try_recv(),
            Ok(RelayEnvelope::DaemonResponse { .. })
        ));
        assert!(event_rx.try_recv().is_err());
    }

    #[test]
    fn send_outgoing_envelope_routes_subscription_events_to_event_lane() {
        let (outgoing_tx, mut priority_rx, mut event_rx) = RelayOutgoingSender::channel(1);

        send_outgoing_envelope(
            &outgoing_tx,
            RelayEnvelope::DaemonEvent {
                subscription_id: "subscription-1".to_string(),
                event_id: 7,
                encrypted_event: encrypted_payload_for_test(),
            },
        )
        .expect("event should send");

        assert!(priority_rx.try_recv().is_err());
        assert!(matches!(
            event_rx.try_recv(),
            Ok(RelayEnvelope::DaemonEvent { event_id: 7, .. })
        ));
    }

    #[tokio::test]
    async fn subscription_event_send_waits_for_event_lane_capacity() {
        let (outgoing_tx, _priority_rx, mut event_rx) = RelayOutgoingSender::channel(1);
        send_outgoing_envelope(
            &outgoing_tx,
            RelayEnvelope::DaemonEvent {
                subscription_id: "subscription-1".to_string(),
                event_id: 7,
                encrypted_event: encrypted_payload_for_test(),
            },
        )
        .expect("first event should fill the event lane");

        let second_send = send_outgoing_event_envelope(
            &outgoing_tx,
            RelayEnvelope::DaemonEvent {
                subscription_id: "subscription-1".to_string(),
                event_id: 8,
                encrypted_event: encrypted_payload_for_test(),
            },
        );
        tokio::pin!(second_send);
        assert!(
            tokio::time::timeout(Duration::from_millis(10), &mut second_send)
                .await
                .is_err()
        );

        assert!(matches!(
            event_rx.recv().await,
            Some(RelayEnvelope::DaemonEvent { event_id: 7, .. })
        ));
        second_send
            .await
            .expect("second event should enqueue after capacity frees");
        assert!(matches!(
            event_rx.recv().await,
            Some(RelayEnvelope::DaemonEvent { event_id: 8, .. })
        ));
    }

    #[test]
    fn display_tunnel_chunks_and_close_share_ordered_event_lane() {
        let (outgoing_tx, mut priority_rx, mut event_rx) = RelayOutgoingSender::channel(2);

        send_outgoing_envelope(
            &outgoing_tx,
            RelayEnvelope::DaemonDisplayTunnelChunk {
                chunk: RelayDisplayTunnelStreamChunk {
                    stream_id: "stream-1".to_string(),
                    data: "aGVsbG8=".to_string(),
                    message_kind: None,
                },
            },
        )
        .expect("display chunk should send");
        send_outgoing_envelope(
            &outgoing_tx,
            RelayEnvelope::DaemonDisplayTunnelClose {
                stream_id: "stream-1".to_string(),
                error: None,
            },
        )
        .expect("display close should send");

        assert!(priority_rx.try_recv().is_err());
        assert!(matches!(
            event_rx.try_recv(),
            Ok(RelayEnvelope::DaemonDisplayTunnelChunk { .. })
        ));
        assert!(matches!(
            event_rx.try_recv(),
            Ok(RelayEnvelope::DaemonDisplayTunnelClose { .. })
        ));
    }

    fn encrypted_payload_for_test() -> EncryptedRelayPayload {
        EncryptedRelayPayload {
            sender_public_key: "sender".to_string(),
            nonce: "nonce".to_string(),
            ciphertext: "ciphertext".to_string(),
        }
    }
}
