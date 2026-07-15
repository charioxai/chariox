//! Relay connection state transitions and Cloud presence publication.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot, RwLock};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use crate::runtime::router::CommandRouter;

use arroba_relay::protocol::{
    RelayDisplayTunnelRegistration, RelayDisplayTunnelStreamChunk, RelayEnvelope, RelayError,
};

use super::peer_client::RelayPeerResponseEnvelope;
use super::request_errors::relay_error;
use super::RelayOutgoingSender;

const CLOUD_RELAY_PRESENCE_PUBLISH_TIMEOUT: Duration = Duration::from_millis(750);

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

    pub(super) fn peer_public_key(&self, target_ref: &str) -> Option<String> {
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
