use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use chariox_relay::protocol::RelayEnvelope;
use futures_util::{Sink, SinkExt};
use tokio_tungstenite::tungstenite::Message;

const RELAY_LARGE_FRAME_LOG_BYTES: usize = 256 * 1024;
const RELAY_SLOW_WRITE_LOG_MS: u128 = 500;

pub(super) async fn send_relay_envelope_frame<S>(
    writer: &mut S,
    envelope: RelayEnvelope,
    lane: &'static str,
    binary_events: bool,
    closing: &AtomicBool,
) -> bool
where
    S: Sink<Message> + Unpin,
{
    // MP-08/MP-10/MP-11: skip queued stale stream events while
    // preserving every bulk reply. Keep no sender inside its own writer task.
    if !super::super::relay_envelope_should_write(&envelope, closing.load(Ordering::Acquire)) {
        return true;
    }
    let kind = relay_envelope_kind(&envelope);
    let frame = if binary_events {
        match chariox_relay::binary_event::from_envelope(&envelope) {
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
    let packet = chariox_relay::transport_timing::envelope_signature(&envelope);
    chariox_relay::transport_timing::record("kernel_write_start", packet);
    let started = Instant::now();
    let sent = writer.send(frame).await.is_ok();
    chariox_relay::transport_timing::record("kernel_write_end", packet);
    if kind == "daemon_event" {
        crate::transport::kernel_browser_display::timing("event_socket_write", started);
    }
    let elapsed_ms = started.elapsed().as_millis();
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

/// MP-08/MP-10/MP-11: one bounded writer for every runtime surface.
pub(super) async fn write_relay_envelopes<S>(
    writer: &mut S,
    mut priority_outgoing_rx: tokio::sync::mpsc::Receiver<RelayEnvelope>,
    mut event_outgoing_rx: tokio::sync::mpsc::Receiver<RelayEnvelope>,
    mut transport_receipt_rx: tokio::sync::mpsc::Receiver<String>,
    mut pong_rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
    binary_events: bool,
    writer_closing: &AtomicBool,
) where
    S: Sink<Message> + Unpin,
{
    let mut priority_open = true;
    let mut event_open = true;
    // MP-08/MP-10/MP-11: Room displays also use this socket.
    // The bounded event lane supplies backpressure; delaying each
    // packet by a frame adds latency without combining writes.
    while priority_open || event_open {
        tokio::select! {
            biased;
            Some(receipt) = transport_receipt_rx.recv() => {
                if writer.send(Message::Text(receipt.into())).await.is_err() { break; }
            }
            Some(payload) = pong_rx.recv() => {
                if writer.send(Message::Pong(payload.into())).await.is_err() { break; }
            }
            envelope = priority_outgoing_rx.recv(), if priority_open => {
                match envelope {
                    Some(envelope) => {
                        if !send_relay_envelope_frame(writer, envelope, "priority", binary_events, writer_closing).await { break; }
                    }
                    None => priority_open = false,
                }
            }
            envelope = event_outgoing_rx.recv(), if event_open => {
                match envelope {
                    Some(envelope) => {
                        if !send_relay_envelope_frame(writer, envelope, "event", binary_events, writer_closing).await { break; }
                    }
                    None => event_open = false,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(event_id: u64) -> RelayEnvelope {
        RelayEnvelope::DaemonEvent {
            subscription_id: "room-display-test".into(),
            event_id,
            encrypted_event: chariox_relay::protocol::EncryptedRelayPayload {
                sender_public_key: "sender".into(),
                nonce: "nonce".into(),
                ciphertext: "payload".into(),
            },
        }
    }
    #[tokio::test(start_paused = true)]
    async fn mp10_room_event_writer_has_no_frame_timer_or_browser_flag() {
        let (_priority_tx, priority) = tokio::sync::mpsc::channel(1);
        let (event_tx, events) = tokio::sync::mpsc::channel(1);
        let (_receipt_tx, receipts) = tokio::sync::mpsc::channel(1);
        let (_pong_tx, pongs) = tokio::sync::mpsc::channel(1);
        let (written_tx, mut written_rx) = tokio::sync::mpsc::channel(1);
        let task = tokio::spawn(async move {
            let mut sink = Box::pin(futures_util::sink::unfold(
                written_tx,
                |tx, packet| async move {
                    tx.send(packet).await.map_err(|_| ())?;
                    Ok::<_, ()>(tx)
                },
            ));
            write_relay_envelopes(
                &mut sink,
                priority,
                events,
                receipts,
                pongs,
                false,
                &AtomicBool::new(false),
            )
            .await;
        });
        event_tx.send(event(1)).await.unwrap();
        let packet =
            tokio::time::timeout(std::time::Duration::from_millis(20), written_rx.recv()).await;
        task.abort();
        assert!(
            packet.unwrap().is_some(),
            "an event must be written before a 33ms frame timer"
        );
    }
    #[tokio::test]
    async fn mp10_slow_room_writer_preserves_fifo_backpressure_and_control_priority() {
        let (priority_tx, priority) = tokio::sync::mpsc::channel(1);
        let (event_tx, events) = tokio::sync::mpsc::channel(2);
        let (_receipt_tx, receipts) = tokio::sync::mpsc::channel(1);
        let (_pong_tx, pongs) = tokio::sync::mpsc::channel(1);
        let (written_tx, mut written_rx) = tokio::sync::mpsc::channel(1);
        let writer = tokio::spawn(async move {
            let mut sink = Box::pin(futures_util::sink::unfold(
                written_tx,
                |tx, packet| async move {
                    tx.send(packet).await.map_err(|_| ())?;
                    Ok::<_, ()>(tx)
                },
            ));
            write_relay_envelopes(
                &mut sink,
                priority,
                events,
                receipts,
                pongs,
                false,
                &AtomicBool::new(false),
            )
            .await;
        });
        let producer = tokio::spawn(async move {
            for id in 0..200 {
                event_tx.send(event(id)).await.unwrap();
            }
        });
        // Wait for the first write; with the sink blocked, the bounded event
        // queue cannot absorb the rest of the stream.
        while written_rx.is_empty() {
            tokio::task::yield_now().await;
        }
        assert!(!producer.is_finished());
        priority_tx.send(event(999)).await.unwrap();
        let mut ids = Vec::new();
        for _ in 0..201 {
            let packet = tokio::time::timeout(std::time::Duration::from_secs(2), written_rx.recv())
                .await
                .unwrap()
                .unwrap();
            let envelope: RelayEnvelope = serde_json::from_str(packet.to_text().unwrap()).unwrap();
            let RelayEnvelope::DaemonEvent { event_id, .. } = envelope else {
                panic!("event expected")
            };
            ids.push(event_id);
        }
        producer.await.unwrap();
        assert!(
            ids.iter().position(|id| *id == 999).unwrap() <= 2,
            "control must precede queued bulk events"
        );
        assert_eq!(
            ids.into_iter().filter(|id| *id != 999).collect::<Vec<_>>(),
            (0..200).collect::<Vec<_>>()
        );
        drop(priority_tx);
        writer.await.unwrap();
    }
}
