//! Relay connection state transitions and Cloud presence publication.

use std::collections::BTreeMap;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, RwLock};

use arroba_relay::protocol::RelayEnvelope;

use crate::runtime::router::CommandRouter;

use super::peer_client::RelayPeerResponseEnvelope;
use super::request_errors::relay_error;

#[allow(dead_code)]
#[derive(Debug)]
pub struct RelayClientState {
    pub(super) connected: bool,
    pub(super) outgoing_tx: Option<mpsc::UnboundedSender<RelayEnvelope>>,
    pub(super) pending_peer_requests: BTreeMap<String, oneshot::Sender<RelayPeerResponseEnvelope>>,
    pub(super) next_peer_request_id: u64,
}

impl RelayClientState {
    pub fn connected(&self) -> bool {
        self.connected
    }
}

impl Default for RelayClientState {
    fn default() -> Self {
        Self {
            connected: false,
            outgoing_tx: None,
            pending_peer_requests: BTreeMap::new(),
            next_peer_request_id: 0,
        }
    }
}

pub(super) async fn set_connected(
    state: &Arc<RwLock<RelayClientState>>,
    outgoing_tx: mpsc::UnboundedSender<RelayEnvelope>,
) {
    let mut guard = state.write().await;
    guard.connected = true;
    guard.outgoing_tx = Some(outgoing_tx);
}

pub(super) async fn publish_cloud_presence(
    router: &Arc<CommandRouter>,
    online: bool,
    reason: &str,
) {
    if let Err(error) = router.publish_cloud_kernel_presence(online).await {
        crate::logging::warn_with_fields(
            "daemon.relay_client",
            "failed to publish cloud relay presence",
            serde_json::json!({
                "online": online,
                "reason": reason,
                "error": error.to_string(),
            }),
        );
    }
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
    publish_cloud_presence(router, false, reason).await;
    set_disconnected(state).await;
}

async fn set_disconnected(state: &Arc<RwLock<RelayClientState>>) {
    let pending_peer = {
        let mut guard = state.write().await;
        guard.connected = false;
        guard.outgoing_tx = None;
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
