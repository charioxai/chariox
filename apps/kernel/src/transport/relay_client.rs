use std::collections::BTreeMap;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, oneshot, watch, Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout, MissedTickBehavior};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use arroba_relay::protocol::{
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
    agent_activity_changed_event, event_is_relevant_to_attachment, provider_run_changed_event,
    runtime_interactions_changed_event, session_metadata_changed_event,
    subscription_event_stream_id, terminal_output_event_batches, waiting_room_rows_changed_event,
    workflow_run_only_changed, workflow_run_updated_events, KernelEvent,
    WAITING_ROOM_INVENTORY_SENTINEL_ID, WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE,
};
use crate::transport::relay_crypto;
use crate::transport::relay_discovery;
use crate::transport::relay_peer::RelayPeerEvent;

mod connection_config;
mod connection_state;
mod connector;
mod daemon_requests;
mod display_tunnel;
mod envelope_io;
mod events;
mod incoming_envelopes;
mod peer_client;
mod peer_events;
mod peer_requests;
mod remote_inventory;
mod request_errors;
mod subscriptions;
use connection_config::{relay_config_continuity, RelayConfigContinuity};
use connection_state::{
    publish_offline_and_set_disconnected, set_connected, spawn_cloud_presence_publish,
};
use daemon_requests::handle_daemon_request;
use display_tunnel::handle_display_tunnel_open;
use envelope_io::{encrypt_json_response, encrypt_peer_payload, send_outgoing_envelope};
use events::{emit_relay_event, replay_recent_relay_events, RelayEventRuntime};
use incoming_envelopes::handle_incoming_envelope;
#[cfg(test)]
pub use peer_client::send_peer_request_via_relay;
use peer_client::{resolve_pending_peer_response, RelayPeerResponseEnvelope};
pub use peer_client::{
    send_peer_request_via_connected_relay, send_peer_request_via_temporary_connection,
    send_peer_request_via_temporary_connection_with_timeout, LEASED_PROMPT_SUBMIT_RESPONSE_TIMEOUT,
};
use peer_events::{handle_daemon_peer_event, pump_leased_projection_events};
use peer_requests::handle_daemon_peer_request;
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
pub(crate) use connection_state::RelayDisplayTunnelTarget;
pub use connector::{run_daemon_relay_connector, run_daemon_relay_connector_with_static_relay};

const RELAY_HEARTBEAT_INTERVAL_TICKS: u64 = 50;
const RELAY_REMOTE_MACHINE_DISCOVERY_INTERVAL_TICKS: u64 = 150;
const RELAY_WAITING_ROOM_INVENTORY_INTERVAL_TICKS: u64 = 100;
const RELAY_OUTGOING_QUEUE_LIMIT: usize = 1024;
const RELAY_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const CLOUD_RELAY_TOKEN_REFRESH_CHECK_INTERVAL: Duration = Duration::from_secs(5);
const CLOUD_RELAY_PRESENCE_REFRESH_INTERVAL: Duration = Duration::from_secs(30);
const RELAY_HEARTBEAT_APP_WORK_TIMEOUT: Duration = Duration::from_millis(500);
const REMOTE_INVENTORY_RELAY_TIMEOUT_MS: u64 = 10_000;
const REMOTE_INVENTORY_KERNEL_PROBE_TIMEOUT_MS: u64 = 5_000;

#[derive(Debug, Clone)]
pub(crate) struct RelayOutgoingSender {
    priority_tx: mpsc::Sender<RelayEnvelope>,
    event_tx: mpsc::Sender<RelayEnvelope>,
}

impl RelayOutgoingSender {
    fn new(
        priority_tx: mpsc::Sender<RelayEnvelope>,
        event_tx: mpsc::Sender<RelayEnvelope>,
    ) -> Self {
        Self {
            priority_tx,
            event_tx,
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
}

fn relay_envelope_uses_event_lane(envelope: &RelayEnvelope) -> bool {
    matches!(
        envelope,
        RelayEnvelope::DaemonEvent { .. }
            | RelayEnvelope::DaemonPeerEvent { .. }
            | RelayEnvelope::DaemonDisplayTunnelChunk { .. }
            | RelayEnvelope::DaemonDisplayTunnelClientChunk { .. }
    )
}

type RelayCommandResultCache = Arc<CommandResultCache>;

#[cfg(test)]
mod tests;
