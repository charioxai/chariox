//! Relay connection state transitions and Cloud presence publication.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot, RwLock};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use crate::runtime::router::CommandRouter;

use chariox_relay::protocol::{
    RelayDisplayTunnelRegistration, RelayDisplayTunnelStreamChunk, RelayEnvelope, RelayError,
};

use super::peer_client::RelayPeerResponseEnvelope;
use super::request_errors::relay_error;
use super::RelayOutgoingSender;

const CLOUD_RELAY_PRESENCE_PUBLISH_TIMEOUT: Duration = Duration::from_millis(750);
const MANAGED_SLICE_ACTIVATION_EXPECTATION_TTL: Duration = Duration::from_secs(30);
const MANAGED_SLICE_ACTIVATION_CONFIRMATION_TTL: Duration = Duration::from_secs(30);
const MANAGED_SLICE_ACTIVATION_CONFIRMATION_MAX_ATTEMPTS: u8 = 3;

#[allow(dead_code)]
#[derive(Debug)]
pub struct RelayClientState {
    pub(super) connected: bool,
    pub(super) connected_relay_url: Option<String>,
    pub(super) outgoing_tx: Option<RelayOutgoingSender>,
    pub(super) pending_peer_requests: BTreeMap<String, oneshot::Sender<RelayPeerResponseEnvelope>>,
    pub(super) next_peer_request_id: u64,
    peer_public_keys: BTreeMap<String, String>,
    pub(super) display_tunnels: BTreeMap<String, RelayDisplayTunnelTarget>,
    pub(super) pending_display_tunnel_registrations:
        BTreeMap<String, oneshot::Sender<Option<RelayError>>>,
    pub(super) display_streams: BTreeMap<String, mpsc::Sender<RelayDisplayTunnelClientEvent>>,
    managed_slice_activation_expectations: BTreeMap<String, ManagedSliceRelayActivationExpectation>,
    pending_managed_slice_activation_confirmation:
        Option<PendingManagedSliceActivationConfirmation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ManagedSliceRelayActivationExpectation {
    worker_kernel_id: String,
    worker_relay_subject: String,
    worker_public_key: String,
    activation_nonce: String,
    confirmed: bool,
    expires_at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingManagedSliceActivationConfirmation {
    pub(crate) slice_id: String,
    pub(crate) owner_kernel_id: String,
    pub(crate) owner_public_key: String,
    pub(crate) worker_kernel_id: String,
    pub(crate) activation_nonce: String,
    expires_at: Instant,
    attempts: u8,
}

impl PendingManagedSliceActivationConfirmation {
    pub(crate) fn new(
        slice_id: String,
        owner_kernel_id: String,
        owner_public_key: String,
        worker_kernel_id: String,
        activation_nonce: String,
    ) -> Self {
        Self {
            slice_id,
            owner_kernel_id,
            owner_public_key,
            worker_kernel_id,
            activation_nonce,
            expires_at: Instant::now() + MANAGED_SLICE_ACTIVATION_CONFIRMATION_TTL,
            attempts: 0,
        }
    }
}

impl RelayClientState {
    pub fn connected(&self) -> bool {
        self.connected
    }

    pub(crate) fn outgoing_sender(&self) -> Option<RelayOutgoingSender> {
        self.outgoing_tx.clone()
    }

    pub(crate) fn connected_relay_url(&self) -> Option<String> {
        self.connected_relay_url.clone()
    }

    pub(crate) fn peer_public_key(&self, target_ref: &str) -> Option<String> {
        self.peer_public_keys.get(target_ref).cloned()
    }

    pub(crate) fn remember_peer_public_key(
        &mut self,
        target_ref: impl Into<String>,
        public_key: impl Into<String>,
    ) {
        self.peer_public_keys
            .insert(target_ref.into(), public_key.into());
    }

    pub(crate) fn begin_managed_slice_relay_activation(
        &mut self,
        slice_id: String,
        worker_kernel_id: String,
        worker_relay_subject: String,
        worker_public_key: String,
        activation_nonce: String,
    ) {
        self.prune_managed_slice_relay_activations(Instant::now());
        self.managed_slice_activation_expectations.insert(
            slice_id,
            ManagedSliceRelayActivationExpectation {
                worker_kernel_id,
                worker_relay_subject,
                worker_public_key,
                activation_nonce,
                confirmed: false,
                expires_at: Instant::now() + MANAGED_SLICE_ACTIVATION_EXPECTATION_TTL,
            },
        );
    }

    pub(crate) fn confirm_managed_slice_relay_activation(
        &mut self,
        slice_id: &str,
        worker_kernel_id: &str,
        worker_relay_subject: &str,
        worker_public_key: &str,
        activation_nonce: &str,
    ) -> bool {
        self.prune_managed_slice_relay_activations(Instant::now());
        let Some(expectation) = self.managed_slice_activation_expectations.get_mut(slice_id) else {
            return false;
        };
        if expectation.worker_kernel_id != worker_kernel_id
            || expectation.worker_relay_subject != worker_relay_subject
            || expectation.worker_public_key != worker_public_key
            || expectation.activation_nonce != activation_nonce
        {
            return false;
        }
        expectation.confirmed = true;
        true
    }

    pub(crate) fn managed_slice_relay_activation_confirmed(
        &self,
        slice_id: &str,
        activation_nonce: &str,
    ) -> bool {
        self.managed_slice_activation_expectations
            .get(slice_id)
            .is_some_and(|expectation| {
                expectation.activation_nonce == activation_nonce
                    && expectation.confirmed
                    && expectation.expires_at > Instant::now()
            })
    }

    pub(crate) fn retain_completed_managed_slice_relay_activation(
        &mut self,
        slice_id: &str,
        activation_nonce: &str,
    ) {
        if let Some(expectation) = self.managed_slice_activation_expectations.get_mut(slice_id) {
            if expectation.activation_nonce == activation_nonce && expectation.confirmed {
                expectation.expires_at = Instant::now() + MANAGED_SLICE_ACTIVATION_EXPECTATION_TTL;
            }
        }
    }

    pub(crate) fn discard_managed_slice_relay_activation(
        &mut self,
        slice_id: &str,
        activation_nonce: &str,
    ) {
        if self
            .managed_slice_activation_expectations
            .get(slice_id)
            .is_some_and(|expectation| expectation.activation_nonce == activation_nonce)
        {
            self.managed_slice_activation_expectations.remove(slice_id);
        }
    }

    pub(crate) fn stage_managed_slice_activation_confirmation(
        &mut self,
        confirmation: PendingManagedSliceActivationConfirmation,
    ) {
        self.pending_managed_slice_activation_confirmation = Some(confirmation);
    }

    pub(crate) fn claim_pending_managed_slice_activation_confirmation(
        &mut self,
    ) -> Option<PendingManagedSliceActivationConfirmation> {
        let expired_or_exhausted = self
            .pending_managed_slice_activation_confirmation
            .as_ref()
            .is_some_and(|pending| {
                pending.expires_at <= Instant::now()
                    || pending.attempts >= MANAGED_SLICE_ACTIVATION_CONFIRMATION_MAX_ATTEMPTS
            });
        if expired_or_exhausted {
            self.pending_managed_slice_activation_confirmation = None;
            return None;
        }
        let pending = self
            .pending_managed_slice_activation_confirmation
            .as_mut()?;
        pending.attempts += 1;
        Some(pending.clone())
    }

    pub(crate) fn pending_managed_slice_activation_confirmation(
        &self,
    ) -> Option<PendingManagedSliceActivationConfirmation> {
        self.pending_managed_slice_activation_confirmation
            .as_ref()
            .filter(|pending| {
                pending.expires_at > Instant::now()
                    && pending.attempts < MANAGED_SLICE_ACTIVATION_CONFIRMATION_MAX_ATTEMPTS
            })
            .cloned()
    }

    pub(crate) fn finish_managed_slice_activation_confirmation(
        &mut self,
        slice_id: &str,
        activation_nonce: &str,
    ) {
        if self
            .pending_managed_slice_activation_confirmation
            .as_ref()
            .is_some_and(|pending| {
                pending.slice_id == slice_id && pending.activation_nonce == activation_nonce
            })
        {
            self.pending_managed_slice_activation_confirmation = None;
        }
    }

    fn prune_managed_slice_relay_activations(&mut self, now: Instant) {
        self.managed_slice_activation_expectations
            .retain(|_, expectation| expectation.expires_at > now);
    }

    pub(super) fn forget_peer_public_key(&mut self, target_ref: &str) {
        self.peer_public_keys.remove(target_ref);
    }

    pub(crate) fn upsert_display_tunnel(&mut self, target: RelayDisplayTunnelTarget) {
        self.display_tunnels
            .insert(target.tunnel_id.clone(), target);
    }

    pub(crate) fn remove_display_tunnel(&mut self, tunnel_id: &str) {
        self.display_tunnels.remove(tunnel_id);
    }

    pub(crate) fn insert_pending_display_tunnel_registration(
        &mut self,
        tunnel_id: String,
        sender: oneshot::Sender<Option<RelayError>>,
    ) {
        self.pending_display_tunnel_registrations
            .insert(tunnel_id, sender);
    }

    pub(crate) fn resolve_display_tunnel_registration(
        &mut self,
        tunnel_id: &str,
        error: Option<RelayError>,
    ) {
        if let Some(sender) = self.pending_display_tunnel_registrations.remove(tunnel_id) {
            let _ = sender.send(error);
        }
    }

    pub(crate) fn cancel_display_tunnel_registration(&mut self, tunnel_id: &str) {
        self.pending_display_tunnel_registrations.remove(tunnel_id);
    }

    pub(crate) fn remove_display_tunnels_for_slice(&mut self, slice_id: &str) -> Vec<String> {
        let mut removed = Vec::new();
        self.display_tunnels.retain(|tunnel_id, target| {
            if target.slice_id == slice_id {
                removed.push(tunnel_id.clone());
                false
            } else {
                true
            }
        });
        removed
    }

    pub(crate) fn display_tunnel(
        &self,
        tunnel_id: &str,
        now_ms: u64,
    ) -> Option<RelayDisplayTunnelTarget> {
        self.display_tunnels
            .get(tunnel_id)
            .filter(|target| target.expires_at_ms > now_ms)
            .cloned()
    }

    pub(crate) fn display_tunnel_for_slice(
        &self,
        slice_id: &str,
        local_base_url: &str,
    ) -> Option<RelayDisplayTunnelTarget> {
        self.display_tunnels
            .values()
            .find(|target| target.slice_id == slice_id && target.local_base_url == local_base_url)
            .cloned()
    }

    pub(crate) fn display_tunnel_registration_pending(&self, tunnel_id: &str) -> bool {
        self.pending_display_tunnel_registrations
            .contains_key(tunnel_id)
    }

    pub(crate) fn prune_expired_display_tunnels(&mut self, now_ms: u64) {
        self.display_tunnels
            .retain(|_, target| target.expires_at_ms > now_ms);
    }

    pub(crate) fn insert_display_stream(
        &mut self,
        stream_id: String,
        sender: mpsc::Sender<RelayDisplayTunnelClientEvent>,
    ) {
        self.display_streams.insert(stream_id, sender);
    }

    pub(crate) fn remove_display_stream(&mut self, stream_id: &str) {
        self.display_streams.remove(stream_id);
    }

    pub(crate) fn display_stream_sender(
        &self,
        stream_id: &str,
    ) -> Option<mpsc::Sender<RelayDisplayTunnelClientEvent>> {
        self.display_streams.get(stream_id).cloned()
    }

    #[cfg(test)]
    pub(crate) fn test_set_connected_sender(
        &mut self,
        outgoing_tx: RelayOutgoingSender,
        relay_url: impl Into<String>,
    ) {
        self.connected = true;
        self.connected_relay_url = Some(relay_url.into());
        self.outgoing_tx = Some(outgoing_tx);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RelayDisplayTunnelTarget {
    pub(crate) tunnel_id: String,
    pub(crate) slice_id: String,
    pub(crate) local_base_url: String,
    pub(crate) expires_at_ms: u64,
    pub(crate) capabilities: Vec<String>,
}

impl RelayDisplayTunnelTarget {
    fn registration(&self) -> RelayDisplayTunnelRegistration {
        RelayDisplayTunnelRegistration {
            tunnel_id: self.tunnel_id.clone(),
            expires_at_ms: self.expires_at_ms,
            capabilities: self.capabilities.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum RelayDisplayTunnelClientEvent {
    Chunk(RelayDisplayTunnelStreamChunk),
    Close,
}

impl Default for RelayClientState {
    fn default() -> Self {
        Self {
            connected: false,
            connected_relay_url: None,
            outgoing_tx: None,
            pending_peer_requests: BTreeMap::new(),
            next_peer_request_id: 0,
            peer_public_keys: BTreeMap::new(),
            display_tunnels: BTreeMap::new(),
            pending_display_tunnel_registrations: BTreeMap::new(),
            display_streams: BTreeMap::new(),
            managed_slice_activation_expectations: BTreeMap::new(),
            pending_managed_slice_activation_confirmation: None,
        }
    }
}

pub(super) async fn set_connected(
    state: &Arc<RwLock<RelayClientState>>,
    outgoing_tx: RelayOutgoingSender,
    relay_url: String,
) -> Result<usize, String> {
    let registrations = {
        let mut guard = state.write().await;
        guard.connected = true;
        guard.connected_relay_url = Some(relay_url);
        guard.outgoing_tx = Some(outgoing_tx.clone());
        guard.prune_managed_slice_relay_activations(Instant::now());
        guard.prune_expired_display_tunnels(crate::session::unix_epoch_ms());
        guard
            .display_tunnels
            .values()
            .map(RelayDisplayTunnelTarget::registration)
            .collect::<Vec<_>>()
    };
    for registration in &registrations {
        outgoing_tx
            .try_send(RelayEnvelope::DaemonDisplayTunnelRegister {
                registration: registration.clone(),
            })
            .map_err(|error| {
                format!(
                    "failed to replay display tunnel {} after relay reconnect: {error}",
                    registration.tunnel_id
                )
            })?;
    }
    Ok(registrations.len())
}

pub(super) async fn publish_cloud_presence(
    router: &Arc<CommandRouter>,
    online: bool,
    reason: &str,
) {
    let publish_started = Instant::now();
    match router.publish_cloud_kernel_presence(online).await {
        Ok(()) => {
            crate::logging::info_with_fields(
                "daemon.startup",
                if online {
                    "kernel cloud presence visible"
                } else {
                    "kernel cloud presence offline"
                },
                serde_json::json!({
                    "online": online,
                    "reason": reason,
                    "publish_ms": publish_started.elapsed().as_millis(),
                }),
            );
        }
        Err(error) => {
            crate::logging::warn_with_fields(
                "daemon.relay_client",
                "failed to publish cloud relay presence",
                serde_json::json!({
                    "online": online,
                    "reason": reason,
                    "publish_ms": publish_started.elapsed().as_millis(),
                    "error": error.to_string(),
                }),
            );
        }
    }
}

pub(super) fn spawn_cloud_presence_publish(
    router: Arc<CommandRouter>,
    online: bool,
    reason: impl Into<String>,
) -> JoinHandle<()> {
    let reason = reason.into();
    tokio::spawn(async move {
        if timeout(
            CLOUD_RELAY_PRESENCE_PUBLISH_TIMEOUT,
            publish_cloud_presence(&router, online, &reason),
        )
        .await
        .is_err()
        {
            crate::logging::warn_with_fields(
                "daemon.relay_client",
                "cloud relay presence publish timed out",
                serde_json::json!({
                    "online": online,
                    "reason": reason,
                    "timeout_ms": CLOUD_RELAY_PRESENCE_PUBLISH_TIMEOUT.as_millis(),
                }),
            );
        }
    })
}

pub(super) async fn publish_offline_and_set_disconnected(
    router: &Arc<CommandRouter>,
    state: &Arc<RwLock<RelayClientState>>,
    reason: &str,
) {
    crate::logging::warn_with_fields(
        "daemon.relay_client",
        "relay socket disconnected",
        serde_json::json!({
            "reason": reason,
        }),
    );
    set_disconnected(state).await;
    spawn_cloud_presence_publish(Arc::clone(router), false, reason.to_string());
}

pub(super) async fn set_disconnected(state: &Arc<RwLock<RelayClientState>>) {
    let (pending_peer, pending_display_tunnels) = {
        let mut guard = state.write().await;
        guard.connected = false;
        guard.connected_relay_url = None;
        guard.outgoing_tx = None;
        guard.peer_public_keys.clear();
        guard.managed_slice_activation_expectations.clear();
        guard.display_streams.clear();
        (
            std::mem::take(&mut guard.pending_peer_requests),
            std::mem::take(&mut guard.pending_display_tunnel_registrations),
        )
    };
    for (_, sender) in pending_peer {
        let _ = sender.send(RelayPeerResponseEnvelope {
            from_daemon_id: String::new(),
            encrypted_response: None,
            error: Some(relay_error(
                "relay_disconnected",
                "relay connection closed before peer response arrived",
                true,
            )),
        });
    }
    drop(pending_display_tunnels);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn disconnect_forgets_cached_peer_keys() {
        let state = Arc::new(RwLock::new(RelayClientState::default()));
        state
            .write()
            .await
            .remember_peer_public_key("worker-1", "public-key-1");

        assert_eq!(
            state.read().await.peer_public_key("worker-1").as_deref(),
            Some("public-key-1")
        );

        set_disconnected(&state).await;

        assert!(state.read().await.peer_public_key("worker-1").is_none());
    }

    #[test]
    fn managed_slice_activation_requires_exact_worker_subject_key_and_nonce() {
        let mut state = RelayClientState::default();
        state.begin_managed_slice_relay_activation(
            "slice-1".to_string(),
            "worker-1".to_string(),
            "slice:dev".to_string(),
            "worker-key".to_string(),
            "activation-1".to_string(),
        );

        for (worker_id, subject, key, nonce) in [
            ("worker-2", "slice:dev", "worker-key", "activation-1"),
            ("worker-1", "slice:other", "worker-key", "activation-1"),
            ("worker-1", "slice:dev", "replacement-key", "activation-1"),
            ("worker-1", "slice:dev", "worker-key", "activation-2"),
        ] {
            assert!(
                !state.confirm_managed_slice_relay_activation(
                    "slice-1", worker_id, subject, key, nonce,
                )
            );
            assert!(!state.managed_slice_relay_activation_confirmed("slice-1", "activation-1"));
        }

        assert!(state.confirm_managed_slice_relay_activation(
            "slice-1",
            "worker-1",
            "slice:dev",
            "worker-key",
            "activation-1",
        ));
        assert!(state.managed_slice_relay_activation_confirmed("slice-1", "activation-1"));
        assert!(!state.managed_slice_relay_activation_confirmed("slice-1", "activation-2"));
    }

    #[test]
    fn cancelled_owner_activation_expires_and_cannot_be_confirmed() {
        let mut state = RelayClientState::default();
        state.begin_managed_slice_relay_activation(
            "slice-1".to_string(),
            "worker-1".to_string(),
            "slice:dev".to_string(),
            "worker-key".to_string(),
            "activation-1".to_string(),
        );
        state
            .managed_slice_activation_expectations
            .get_mut("slice-1")
            .expect("expectation should exist")
            .expires_at = Instant::now() - Duration::from_millis(1);

        assert!(!state.confirm_managed_slice_relay_activation(
            "slice-1",
            "worker-1",
            "slice:dev",
            "worker-key",
            "activation-1",
        ));
        assert!(!state
            .managed_slice_activation_expectations
            .contains_key("slice-1"));
    }

    #[test]
    fn worker_confirmation_is_bounded_by_attempts_and_expiry() {
        let pending = || {
            PendingManagedSliceActivationConfirmation::new(
                "slice-1".to_string(),
                "owner-1".to_string(),
                "owner-key".to_string(),
                "worker-1".to_string(),
                "activation-1".to_string(),
            )
        };
        let mut state = RelayClientState::default();
        state.stage_managed_slice_activation_confirmation(pending());
        for _ in 0..MANAGED_SLICE_ACTIVATION_CONFIRMATION_MAX_ATTEMPTS {
            assert!(state
                .claim_pending_managed_slice_activation_confirmation()
                .is_some());
        }
        assert!(state
            .claim_pending_managed_slice_activation_confirmation()
            .is_none());
        assert!(state
            .pending_managed_slice_activation_confirmation
            .is_none());

        state.stage_managed_slice_activation_confirmation(pending());
        state
            .pending_managed_slice_activation_confirmation
            .as_mut()
            .expect("pending confirmation should exist")
            .expires_at = Instant::now() - Duration::from_millis(1);
        assert!(state
            .claim_pending_managed_slice_activation_confirmation()
            .is_none());
        assert!(state
            .pending_managed_slice_activation_confirmation
            .is_none());
    }

    #[tokio::test]
    async fn disconnect_invalidates_owner_expectation_but_preserves_worker_confirmation() {
        let state = Arc::new(RwLock::new(RelayClientState::default()));
        {
            let mut guard = state.write().await;
            guard.begin_managed_slice_relay_activation(
                "slice-1".to_string(),
                "worker-1".to_string(),
                "slice:dev".to_string(),
                "worker-key".to_string(),
                "activation-1".to_string(),
            );
            guard.stage_managed_slice_activation_confirmation(
                PendingManagedSliceActivationConfirmation::new(
                    "slice-1".to_string(),
                    "owner-1".to_string(),
                    "owner-key".to_string(),
                    "worker-1".to_string(),
                    "activation-1".to_string(),
                ),
            );
        }

        set_disconnected(&state).await;

        let guard = state.read().await;
        assert!(!guard.managed_slice_relay_activation_confirmed("slice-1", "activation-1"));
        assert_eq!(
            guard
                .pending_managed_slice_activation_confirmation()
                .expect("worker must confirm after reconnect")
                .activation_nonce,
            "activation-1"
        );
    }

    #[tokio::test]
    async fn disconnect_preserves_live_display_tunnels_for_reconnect_replay() {
        let state = Arc::new(RwLock::new(RelayClientState::default()));
        state
            .write()
            .await
            .upsert_display_tunnel(RelayDisplayTunnelTarget {
                tunnel_id: "publication-live".to_string(),
                slice_id: "publication:session-1:public-api".to_string(),
                local_base_url: "http://127.0.0.1:43100/".to_string(),
                expires_at_ms: u64::MAX,
                capabilities: vec!["http".to_string(), "publication".to_string()],
            });

        set_disconnected(&state).await;

        assert_eq!(
            state
                .read()
                .await
                .display_tunnel("publication-live", 1)
                .map(|target| target.local_base_url),
            Some("http://127.0.0.1:43100/".to_string())
        );
    }

    #[tokio::test]
    async fn reconnect_replays_live_display_tunnels_with_original_capabilities() {
        let state = Arc::new(RwLock::new(RelayClientState::default()));
        state
            .write()
            .await
            .upsert_display_tunnel(RelayDisplayTunnelTarget {
                tunnel_id: "publication-live".to_string(),
                slice_id: "publication:session-1:public-api".to_string(),
                local_base_url: "http://127.0.0.1:43100/".to_string(),
                expires_at_ms: u64::MAX,
                capabilities: vec!["http".to_string(), "publication".to_string()],
            });
        state
            .write()
            .await
            .upsert_display_tunnel(RelayDisplayTunnelTarget {
                tunnel_id: "publication-expired".to_string(),
                slice_id: "publication:session-1:expired".to_string(),
                local_base_url: "http://127.0.0.1:43101/".to_string(),
                expires_at_ms: 1,
                capabilities: vec!["http".to_string(), "publication".to_string()],
            });
        let (outgoing_tx, mut priority_rx, _event_rx) = RelayOutgoingSender::channel(4);

        assert_eq!(
            set_connected(&state, outgoing_tx, "wss://relay.example.test".to_string())
                .await
                .expect("display tunnel replay should queue"),
            1
        );
        assert_eq!(
            priority_rx.recv().await,
            Some(RelayEnvelope::DaemonDisplayTunnelRegister {
                registration: RelayDisplayTunnelRegistration {
                    tunnel_id: "publication-live".to_string(),
                    expires_at_ms: u64::MAX,
                    capabilities: vec!["http".to_string(), "publication".to_string()],
                },
            })
        );
        assert!(state
            .read()
            .await
            .display_tunnel("publication-expired", 0)
            .is_none());
    }
}
