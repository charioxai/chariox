use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, oneshot, watch, Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout, MissedTickBehavior};
#[cfg(test)]
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use chariox_relay::protocol::{
    ClientTarget, EncryptedRelayPayload, RelayDisplayTunnelHeader, RelayDisplayTunnelOpenRequest,
    RelayDisplayTunnelResponseStart, RelayDisplayTunnelStreamChunk, RelayEnvelope, RelayError,
};

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::runtime::event_log::{EventLog, EventRetentionPolicy, ReplayOutcome};
use crate::runtime::router::{CommandRouter, INTERACTIVE_COMMAND_QUEUE_LIMIT};
use crate::runtime_transport::command_cache::CommandResultCache;
use crate::runtime_transport::{
    WatchResult, RECENT_EVENT_LIMIT, SESSION_SNAPSHOT_RECONCILIATION_INTERVAL_TICKS,
    WAITING_ROOM_ROW_COALESCE_MS, WATCH_INTERVAL_MS,
};
use crate::transport::kernel_protocol::{
    event_is_relevant_to_attachment, subscription_event_stream_id, terminal_output_event_batches,
    KernelEvent, WAITING_ROOM_INVENTORY_SENTINEL_ID, WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE,
};
use crate::transport::relay_crypto;
use crate::transport::relay_discovery;
use crate::transport::relay_peer::RelayPeerEvent;

mod browser_display;
mod browser_input_order;
mod connection_config;
mod connection_state;
mod connector;
mod daemon_requests;
mod display_admission;
mod display_delivery;
mod display_ingress;
mod display_pump;
mod display_tunnel;
mod envelope_io;
mod events;
mod incoming_envelopes;
mod local_browser;
mod peer_client;
mod peer_events;
mod peer_requests;
mod remote_inventory;
mod request_errors;
mod request_timing;
mod sender_identity;
mod subscriptions;
use connection_config::{relay_config_continuity, RelayConfigContinuity};
use connection_state::{
    publish_offline_and_set_disconnected, set_connected, spawn_cloud_presence_publish,
};
use daemon_requests::{handle_prepared_daemon_request, prepare_daemon_request};
use display_tunnel::handle_display_tunnel_open;
use envelope_io::{
    encrypt_json_response, encrypt_peer_payload, send_outgoing_envelope,
    send_outgoing_event_envelope,
};
use events::{emit_relay_event, replay_recent_relay_events, RelayEventRuntime};
use incoming_envelopes::{handle_incoming_envelope, IncomingEnvelopeContext, RelayReconnectGate};
use local_browser::LocalBrowserDirect;
#[cfg(test)]
pub use peer_client::send_peer_request_via_relay;
pub(crate) use peer_client::send_peer_request_via_temporary_connection_with_optional_timeout;
pub(crate) use peer_client::{
    enqueue_peer_request_via_connected_relay_authorized,
    enqueue_peer_request_via_connected_relay_with_timeout,
    send_peer_request_to_known_kernel_via_relay,
    send_peer_request_to_known_kernel_via_relay_with_timeout,
    send_peer_request_via_connected_relay_authenticated,
    send_peer_request_via_connected_relay_authorized,
    send_peer_request_via_temporary_connection_authenticated,
    send_peer_request_via_temporary_connection_authorized, RelayPeerResponseWaiter,
};
use peer_client::{resolve_pending_peer_response, RelayPeerResponseEnvelope};
pub use peer_client::{
    send_peer_request_via_connected_relay, send_peer_request_via_connected_relay_with_timeout,
    send_peer_request_via_temporary_connection,
    send_peer_request_via_temporary_connection_with_timeout, LEASED_PROMPT_SUBMIT_RESPONSE_TIMEOUT,
};
use peer_events::{handle_daemon_peer_event, pump_leased_projection_events};
use peer_requests::handle_daemon_peer_request;
#[cfg(test)]
pub(crate) use peer_requests::send_authenticated_peer_request_for_test;
pub(crate) use remote_inventory::refresh_remote_inventory_projection;
use remote_inventory::{
    abort_inventory_refresh_task, clear_remote_inventory_projection,
    spawn_remote_inventory_projection_refresh,
};
use subscriptions::{
    abort_subscription_tasks, handle_relay_subscribe, handle_relay_unsubscribe,
    RelaySubscriptionTasks,
};

pub use connection_state::RelayClientState;
pub(crate) use connection_state::RelayDisplayTunnelClientEvent;
#[cfg(test)]
pub(crate) use connection_state::TestPeerRequestObservation;
pub(crate) use connection_state::{RelayDisplayTunnelTarget, RelayDisplayTunnelTargetKind};
#[cfg(test)]
pub use connector::{
    run_daemon_relay_connector, run_daemon_relay_connector_with_router_and_static_relay,
};
pub use connector::{
    run_daemon_relay_connector_with_router, run_daemon_relay_connector_with_static_relay,
};

#[cfg(test)]
pub(crate) async fn resolve_pending_peer_response_for_test(
    state: &Arc<RwLock<RelayClientState>>,
    request_id: String,
    from_daemon_id: String,
    encrypted_response: EncryptedRelayPayload,
) {
    resolve_pending_peer_response(
        state,
        request_id,
        RelayPeerResponseEnvelope {
            from_daemon_id,
            encrypted_response: Some(encrypted_response),
            error: None,
        },
    )
    .await;
}

#[cfg(test)]
pub(crate) async fn resolve_pending_peer_error_for_test(
    state: &Arc<RwLock<RelayClientState>>,
    request_id: String,
    from_daemon_id: String,
    error: RelayError,
) {
    resolve_pending_peer_response(
        state,
        request_id,
        RelayPeerResponseEnvelope {
            from_daemon_id,
            encrypted_response: None,
            error: Some(error),
        },
    )
    .await;
}

const RELAY_HEARTBEAT_INTERVAL_TICKS: u64 = 50;
const RELAY_REMOTE_MACHINE_DISCOVERY_INTERVAL_TICKS: u64 = 150;
const RELAY_WAITING_ROOM_INVENTORY_INTERVAL_TICKS: u64 = 100;
const RELAY_OUTGOING_QUEUE_LIMIT: usize = 1024;
const RELAY_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const CLOUD_RELAY_TOKEN_REFRESH_CHECK_INTERVAL: Duration = Duration::from_secs(5);
const CLOUD_RELAY_PRESENCE_REFRESH_INTERVAL: Duration = Duration::from_secs(30);
const RELAY_HEARTBEAT_APP_WORK_SLOW_THRESHOLD: Duration = Duration::from_millis(500);
const REMOTE_INVENTORY_RELAY_TIMEOUT_MS: u64 = 10_000;
const REMOTE_INVENTORY_KERNEL_PROBE_TIMEOUT_MS: u64 = 5_000;

#[derive(Debug, Clone)]
pub(crate) struct RelayOutgoingSender {
    priority_tx: mpsc::Sender<RelayEnvelope>,
    event_tx: mpsc::Sender<RelayEnvelope>,
    closing: Arc<AtomicBool>,
}

impl RelayOutgoingSender {
    // MD-DISPLAY-02/04: only the admitted display producer selects this path.
    // Small input acknowledgements avoid event batching; large repair/video
    // envelopes stay on the bounded event lane so controls are not starved.
    async fn send_display_event(
        &self,
        envelope: RelayEnvelope,
    ) -> Result<(), mpsc::error::SendError<RelayEnvelope>> {
        debug_assert!(matches!(envelope, RelayEnvelope::DaemonEvent { .. }));
        let small = matches!(&envelope, RelayEnvelope::DaemonEvent { encrypted_event, .. } if encrypted_event.ciphertext.len() <= 12 * 1024);
        if small {
            self.priority_tx.send(envelope).await
        } else {
            self.send_event(envelope).await
        }
    }
    fn new(
        priority_tx: mpsc::Sender<RelayEnvelope>,
        event_tx: mpsc::Sender<RelayEnvelope>,
    ) -> Self {
        Self {
            priority_tx,
            event_tx,
            closing: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) fn channel(
        capacity: usize,
    ) -> (
        Self,
        mpsc::Receiver<RelayEnvelope>,
        mpsc::Receiver<RelayEnvelope>,
    ) {
        let (priority_tx, priority_rx) = mpsc::channel(capacity);
        let (event_tx, event_rx) = mpsc::channel(capacity);
        (Self::new(priority_tx, event_tx), priority_rx, event_rx)
    }

    #[allow(
        clippy::result_large_err,
        reason = "Preserve the existing try_send typed error contract"
    )]
    pub(crate) fn try_send(
        &self,
        envelope: RelayEnvelope,
    ) -> Result<(), mpsc::error::TrySendError<RelayEnvelope>> {
        if relay_envelope_uses_event_lane(&envelope) {
            self.event_tx.try_send(envelope)
        } else {
            self.priority_tx.try_send(envelope)
        }
    }

    async fn send_event(
        &self,
        envelope: RelayEnvelope,
    ) -> Result<(), mpsc::error::SendError<RelayEnvelope>> {
        debug_assert!(relay_envelope_uses_event_lane(&envelope));
        self.event_tx.send(envelope).await
    }

    /// MP-08/MP-10/MP-11: replies wait for lane capacity; they are never dropped.
    /// Returns false only when the writer is closed.
    async fn send(&self, envelope: RelayEnvelope) -> bool {
        if relay_envelope_uses_event_lane(&envelope) {
            self.event_tx.send(envelope).await.is_ok()
        } else {
            self.priority_tx.send(envelope).await.is_ok()
        }
    }

    // MP-08/MP-10/MP-11: discard stale stream events during reconnect,
    // including coalesced events, but retain every queued reply and Close.
    #[cfg(test)]
    fn should_write(&self, envelope: &RelayEnvelope) -> bool {
        relay_envelope_should_write(envelope, self.closing.load(Ordering::Acquire))
    }

    /// MP-08/MP-10/MP-11: a deferred reconnect Close follows every queued bulk
    /// reply through the FIFO event lane instead of overtaking it.
    /// Returns false only when the writer is closed.
    fn send_after_queued_events(&self, envelope: RelayEnvelope) -> bool {
        self.closing.store(true, Ordering::Release);
        match self.event_tx.try_send(envelope) {
            Err(mpsc::error::TrySendError::Full(envelope)) => {
                let event_tx = self.event_tx.clone();
                tokio::spawn(async move { event_tx.send(envelope).await });
                true
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
            Ok(()) => true,
        }
    }
}

fn relay_envelope_should_write(envelope: &RelayEnvelope, closing: bool) -> bool {
    // Leased peer events carry projections whose cursor already advanced.
    !closing || !matches!(envelope, RelayEnvelope::DaemonEvent { .. })
}

fn relay_envelope_uses_event_lane(envelope: &RelayEnvelope) -> bool {
    // MP-08 / MP-10 / MP-11: bulk encrypted replies (catalogues, history,
    // captures) must not fill the priority queue ahead of Computer control.
    // The relay remains opaque; classify size without inspecting the payload.
    if matches!(envelope, RelayEnvelope::DaemonResponse { encrypted_response: Some(payload), .. }
        | RelayEnvelope::DaemonIncomingPeerResponse { encrypted_response: Some(payload), .. }
        if payload.ciphertext.len() > 64 * 1024)
    {
        return true;
    }
    matches!(
        envelope,
        RelayEnvelope::DaemonEvent { .. }
            | RelayEnvelope::DaemonPeerEvent { .. }
            | RelayEnvelope::DaemonDisplayTunnelChunk { .. }
            | RelayEnvelope::DaemonDisplayTunnelClose { .. }
            | RelayEnvelope::DaemonDisplayTunnelClientChunk { .. }
    )
}

type RelayCommandResultCache = Arc<CommandResultCache>;

#[cfg(test)]
mod admission_drill;
#[cfg(test)]
mod tests;
