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
    // MP-08/MP-10/MP-11: skip coalesced and queued stale stream events while
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

// MP-08 / MP-10 / MP-11: socket writer stays below the connector coordinator.
pub(super) async fn run_relay_writer<S>(
    mut writer: S,
    mut priority_outgoing_rx: tokio::sync::mpsc::Receiver<RelayEnvelope>,
    mut event_outgoing_rx: tokio::sync::mpsc::Receiver<RelayEnvelope>,
    mut pong_rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
    mut transport_receipt_rx: tokio::sync::mpsc::Receiver<String>,
    binary_events: bool,
    closing: std::sync::Arc<AtomicBool>,
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
                if writer.send(Message::Text(receipt.into())).await.is_err() { break; }
            }
            Some(payload) = pong_rx.recv() => {
                if writer.send(Message::Pong(payload.into())).await.is_err() { break; }
            }
            envelope = priority_outgoing_rx.recv(), if priority_open => {
                match envelope {
                    Some(envelope) => if !send_relay_envelope_frame(&mut writer, envelope, "priority", binary_events, &closing).await { break; },
                    None => priority_open = false,
                }
            }
            envelope = event_outgoing_rx.recv(), if event_open => {
                match envelope {
                    Some(envelope) => if !send_relay_envelope_frame(&mut writer, envelope, "event", binary_events, &closing).await { break; },
                    None => event_open = false,
                }
            }
        }
    }
}
