use std::collections::VecDeque;
use std::time::{Duration, Instant};

use chariox_relay::protocol::RelayEnvelope;
use futures_util::{Sink, SinkExt};
use tokio_tungstenite::tungstenite::Message;

pub(super) const RELAY_EVENT_WRITE_COALESCE_MS: u64 = 33;
const RELAY_LARGE_FRAME_LOG_BYTES: usize = 256 * 1024;
const RELAY_SLOW_WRITE_LOG_MS: u128 = 500;

pub(super) async fn send_relay_envelope_frame<S>(
    writer: &mut S,
    envelope: RelayEnvelope,
    lane: &'static str,
    binary_events: bool,
) -> bool
where
    S: Sink<Message> + Unpin,
{
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

#[derive(Debug)]
pub(super) struct RelayEventWriteCoalescer<T> {
    delay_ms: u64,
    envelopes: VecDeque<T>,
    ready_at: Option<tokio::time::Instant>,
}

impl<T> RelayEventWriteCoalescer<T> {
    const MAX_PENDING_EVENTS: usize = 32;

    pub(super) fn new(delay_ms: u64) -> Self {
        Self {
            delay_ms,
            envelopes: VecDeque::new(),
            ready_at: None,
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.envelopes.is_empty()
    }

    pub(super) fn ready_at(&self) -> Option<tokio::time::Instant> {
        self.ready_at
    }

    pub(super) fn push_event(&mut self, envelope: T, now: tokio::time::Instant) -> Option<T> {
        if self.delay_ms == 0 {
            return Some(envelope);
        }
        self.envelopes.push_back(envelope);
        if self.ready_at.is_none() {
            self.ready_at = Some(now + Duration::from_millis(self.delay_ms));
        }
        // A continuously ready receiver can win over the write timer. Flush
        // the oldest event at capacity so batching cannot bypass backpressure.
        if self.envelopes.len() > Self::MAX_PENDING_EVENTS {
            return self.envelopes.pop_front();
        }
        None
    }

    pub(super) fn pop_ready(&mut self, now: tokio::time::Instant) -> Option<T> {
        self.ready_at = None;
        let envelope = self.envelopes.pop_front();
        if !self.envelopes.is_empty() {
            self.ready_at = Some(now);
        }
        envelope
    }
}
