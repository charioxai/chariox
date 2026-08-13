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
) -> bool
where
    S: Sink<Message> + Unpin,
{
    let kind = relay_envelope_kind(&envelope);
    let payload = match serde_json::to_string(&envelope) {
        Ok(payload) => payload,
        Err(_) => return false,
    };
    let payload_len = payload.len();
    let started = Instant::now();
    let sent = writer.send(Message::Text(payload.into())).await.is_ok();
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
