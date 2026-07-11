//! Relay connection state transitions and Cloud presence publication.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot, RwLock};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use crate::runtime::router::CommandRouter;

use arroba_relay::protocol::RelayDisplayTunnelStreamChunk;

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
            display_streams: BTreeMap::new(),
        }
    }
}

pub(super) async fn set_connected(
    state: &Arc<RwLock<RelayClientState>>,
    outgoing_tx: RelayOutgoingSender,
    relay_url: String,
) {
    let mut guard = state.write().await;
    guard.connected = true;
    guard.connected_relay_url = Some(relay_url);
    guard.outgoing_tx = Some(outgoing_tx);
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
    let pending_peer = {
        let mut guard = state.write().await;
        guard.connected = false;
        guard.connected_relay_url = None;
        guard.outgoing_tx = None;
        guard.peer_public_keys.clear();
        guard.display_tunnels.clear();
        guard.display_streams.clear();
        std::mem::take(&mut guard.pending_peer_requests)
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
}
