use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use chariox_relay::protocol::RelayEnvelope;
use futures_util::{Sink, SinkExt};
use tokio_tungstenite::tungstenite::Message;

use super::super::reply_notes::ReplyNotes;

const RELAY_LARGE_FRAME_LOG_BYTES: usize = 256 * 1024;
const RELAY_SLOW_WRITE_LOG_MS: u128 = 500;
const RELAY_WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

async fn send_relay_frame<S>(writer: &mut S, frame: Message) -> bool
where
    S: Sink<Message> + Unpin,
{
    // MP-08 / MP-09 / MP-10 / MP-11: a queued renewal cannot heal a stuck
    // writer. Ending it wakes the connector and closes the old socket.
    match tokio::time::timeout(RELAY_WRITE_TIMEOUT, writer.send(frame)).await {
        Ok(result) => result.is_ok(),
        Err(_) => {
            crate::logging::warn_with_fields(
                "daemon.relay_client",
                "relay socket write timed out; reconnecting",
                serde_json::json!({ "timeout_ms": RELAY_WRITE_TIMEOUT.as_millis() }),
            );
            false
        }
    }
}

pub(super) async fn send_relay_envelope_frame<S>(
    writer: &mut S,
    envelope: RelayEnvelope,
    lane: &'static str,
    binary_events: bool,
    closing: &AtomicBool,
    notes: &ReplyNotes,
) -> bool
where
    S: Sink<Message> + Unpin,
{
    // MP-08/MP-10/MP-11: skip coalesced and queued stale stream events while
    // preserving every bulk reply. Keep no sender inside its own writer task.
    if !super::super::relay_envelope_should_write(&envelope, closing.load(Ordering::Acquire)) {
        return true;
    }
    let kind = relay_envelope_kind(&envelope);
    let frame = if binary_events {
        match chariox_relay::binary_response::from_envelope(&envelope).and_then(|response| {
            match response {
                Some(bytes) => Ok(Some(bytes)),
                None => chariox_relay::binary_event::from_envelope(&envelope),
            }
        }) {
            Ok(Some(bytes)) => Message::Binary(bytes.into()),
            Ok(None) => match serde_json::to_string(&envelope) {
                Ok(text) => Message::Text(text.into()),
                Err(_) => return false,
            },
            Err(_) => match serde_json::to_string(&envelope) {
                Ok(text) => Message::Text(text.into()),
                Err(_) => return false,
            },
        }
    } else {
        match serde_json::to_string(&envelope) {
            Ok(text) => Message::Text(text.into()),
            Err(_) => return false,
        }
    };
    let payload_len = frame.len();
    let reply = notes.take(&envelope);
    let packet = chariox_relay::transport_timing::envelope_signature(&envelope);
    chariox_relay::transport_timing::record("kernel_write_start", packet);
    let started = Instant::now();
    let sent = send_relay_frame(writer, frame).await;
    chariox_relay::transport_timing::record("kernel_write_end", packet);
    if kind == "daemon_event" {
        crate::transport::kernel_browser_display::timing("event_socket_write", started);
    }
    let elapsed_ms = started.elapsed().as_millis();
    notes.record_written(lane, reply.as_ref(), payload_len);
    if payload_len >= RELAY_LARGE_FRAME_LOG_BYTES || elapsed_ms >= RELAY_SLOW_WRITE_LOG_MS {
        crate::logging::warn_with_fields(
            "daemon.relay_client",
            "relay envelope write exceeded diagnostic threshold",
            serde_json::json!({
                "lane": lane,
                "kind": kind,
                "payload_bytes": payload_len,
                "write_ms": elapsed_ms,
                "sent": sent,
                "request_kind": reply.as_ref().map(|reply| reply.origin.request_kind),
                "client": reply.as_ref().map(|reply| reply.origin.client.as_str()),
            }),
        );
    }
    sent
}

fn relay_envelope_kind(envelope: &RelayEnvelope) -> &'static str {
    match envelope {
        RelayEnvelope::DaemonRegister { .. } => "daemon_register",
        RelayEnvelope::DaemonHeartbeat { .. } => "daemon_heartbeat",
        RelayEnvelope::DaemonResponse { .. } => "daemon_response",
        RelayEnvelope::DaemonEvent { .. } => "daemon_event",
        RelayEnvelope::DaemonPeerRequest { .. } => "daemon_peer_request",
        RelayEnvelope::DaemonPeerResponse { .. } => "daemon_peer_response",
        RelayEnvelope::DaemonPeerEvent { .. } => "daemon_peer_event",
        RelayEnvelope::DaemonDisplayTunnelResponseStart { .. } => {
            "daemon_display_tunnel_response_start"
        }
        RelayEnvelope::DaemonDisplayTunnelChunk { .. } => "daemon_display_tunnel_chunk",
        RelayEnvelope::DaemonDisplayTunnelClientChunk { .. } => {
            "daemon_display_tunnel_client_chunk"
        }
        RelayEnvelope::Close { .. } => "close",
        _ => "other",
    }
}

// MP-08 / MP-10 / MP-11: socket writer stays below the connector coordinator.
#[allow(
    clippy::too_many_arguments,
    reason = "each lane, the closing flag and the reply notes are separate inputs of the one writer task"
)]
pub(super) async fn run_relay_writer<S>(
    mut writer: S,
    mut priority_outgoing_rx: tokio::sync::mpsc::Receiver<RelayEnvelope>,
    mut event_outgoing_rx: tokio::sync::mpsc::Receiver<RelayEnvelope>,
    mut pong_rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
    mut transport_receipt_rx: tokio::sync::mpsc::Receiver<String>,
    binary_events: bool,
    closing: std::sync::Arc<AtomicBool>,
    notes: std::sync::Arc<ReplyNotes>,
) where
    S: Sink<Message> + Unpin,
{
    // The producer already uses bounded priority and event channels. Holding
    // envelopes for a frame did not combine their writes; it only delayed the
    // first accepted turn and state. Preserve lane priority and event FIFO.
    let mut priority_open = true;
    let mut event_open = true;
    while priority_open || event_open {
        tokio::select! {
            biased;
            Some(receipt) = transport_receipt_rx.recv() => {
                if !send_relay_frame(&mut writer, Message::Text(receipt.into())).await { break; }
            }
            Some(payload) = pong_rx.recv() => {
                if !send_relay_frame(&mut writer, Message::Pong(payload.into())).await { break; }
            }
            envelope = priority_outgoing_rx.recv(), if priority_open => {
                match envelope {
                    Some(envelope) => if !send_relay_envelope_frame(&mut writer, envelope, "priority", binary_events, &closing, &notes).await { break; },
                    None => priority_open = false,
                }
            }
            envelope = event_outgoing_rx.recv(), if event_open => {
                match envelope {
                    Some(envelope) => if !send_relay_envelope_frame(&mut writer, envelope, "event", binary_events, &closing, &notes).await { break; },
                    None => event_open = false,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::task::{Context, Poll};
    use std::time::Duration;
    use tokio::sync::mpsc;

    struct StalledSink;
    impl Sink<Message> for StalledSink {
        type Error = std::io::Error;
        fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
        fn start_send(self: Pin<&mut Self>, _: Message) -> Result<(), Self::Error> {
            Ok(())
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Pending
        }
        fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Pending
        }
    }

    #[tokio::test(start_paused = true)]
    async fn stalled_writes_end_the_writer_so_the_connector_can_reconnect() {
        // MP-08 / MP-09 / MP-10 / MP-11: token renewal queued behind a stuck
        // socket must not leave the connector believing it is still online.
        for lane in ["priority", "event", "receipt", "pong"] {
            let (priority, priority_rx) = mpsc::channel(2);
            let (event, event_rx) = mpsc::channel(2);
            let (pong, pong_rx) = mpsc::channel(2);
            let (receipt, receipt_rx) = mpsc::channel(2);
            match lane {
                "priority" => priority
                    .send(RelayEnvelope::DaemonHeartbeat {
                        daemon_id: "kernel".into(),
                        registration: None,
                    })
                    .await
                    .unwrap(),
                "event" => event
                    .send(RelayEnvelope::Close {
                        reason: "test".into(),
                        kernel_reload: false,
                    })
                    .await
                    .unwrap(),
                "receipt" => receipt.send("receipt".into()).await.unwrap(),
                _ => pong.send(vec![]).await.unwrap(),
            }
            let writer = run_relay_writer(
                StalledSink,
                priority_rx,
                event_rx,
                pong_rx,
                receipt_rx,
                false,
                Arc::new(AtomicBool::new(false)),
                Arc::default(),
            );
            assert!(
                tokio::time::timeout(Duration::from_secs(31), writer)
                    .await
                    .is_ok(),
                "stalled {lane} write never releases connector"
            );
        }
    }

    // MP-08 / MP-09 / MP-10 / MP-11 (relayflood): the 11-09 flood had to be traced to its
    // requester from frame sizes; the diagnostic now names the request kind and the client.
    #[tokio::test]
    async fn large_reply_log_names_its_request() {
        let logs = crate::logging::capture::start();
        let notes = ReplyNotes::default();
        let reply = RelayEnvelope::DaemonResponse {
            relay_request_id: "chunk-1".into(),
            encrypted_response: Some(chariox_relay::protocol::EncryptedRelayPayload {
                sender_public_key: "daemon".into(),
                nonce: "nonce".into(),
                ciphertext: "x".repeat(300 * 1024),
            }),
            error: None,
        };
        let origin = super::super::super::reply_notes::ReplyOrigin::new(
            "machine_file.read",
            "key-of-the-file-reader",
        );
        let client = origin.client.clone();
        notes.note(&reply, origin, true).await;
        let mut sink = futures_util::sink::drain::<Message>();
        assert!(
            send_relay_envelope_frame(
                &mut sink,
                reply,
                "event",
                false,
                &AtomicBool::new(false),
                &notes
            )
            .await
        );
        let record = logs
            .records()
            .into_iter()
            .find(|record| record.contains("relay envelope write exceeded diagnostic threshold"))
            .expect("a 300 KB reply is logged");
        assert!(
            record.contains("\"request_kind\":\"machine_file.read\""),
            "{record}"
        );
        assert!(
            record.contains(&format!("\"client\":\"{client}\"")),
            "{record}"
        );
        assert_eq!(client.len(), 8);
    }
}
