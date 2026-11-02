//! The relay connections a home kernel holds, and the one route every
//! home-to-worker request for a leased agent takes over them.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, RwLock};

use super::{
    enqueue_peer_request_via_connected_relay_authorized,
    send_peer_request_via_temporary_connection_authorized, RelayClientState,
    RelayPeerResponseWaiter,
};
use crate::transport::relay_peer::RelayPeerRequest;
use crate::DaemonError;

pub(crate) type SharedRelayClientState = Arc<RwLock<RelayClientState>>;

/// The home kernel's connection to a slice's private relay.
pub(crate) struct SlicePrivateRelayConnector {
    pub(crate) relay_url: String,
    pub(crate) state: SharedRelayClientState,
    pub(crate) shutdown_tx: tokio::sync::watch::Sender<bool>,
    pub(crate) task: std::thread::JoinHandle<()>,
}

pub(crate) type SlicePrivateRelayConnectors =
    Arc<Mutex<BTreeMap<String, SlicePrivateRelayConnector>>>;

#[derive(Clone)]
pub(crate) struct ConnectedRelays {
    pub(crate) primary: SharedRelayClientState,
    pub(crate) slice_private: SlicePrivateRelayConnectors,
}

impl ConnectedRelays {
    pub(crate) async fn state_for_config(
        &self,
        relay_config: &crate::config::DaemonConfig,
    ) -> Option<SharedRelayClientState> {
        let relay_url = relay_config.relay_url.as_deref()?;
        if self.primary.read().await.connected_relay_url().as_deref() == Some(relay_url) {
            return Some(Arc::clone(&self.primary));
        }
        let slice_states = {
            let connectors = self.slice_private.lock().await;
            connectors
                .values()
                .filter(|connector| connector.relay_url == relay_url)
                .map(|connector| Arc::clone(&connector.state))
                .collect::<Vec<_>>()
        };
        for state in slice_states {
            if state.read().await.connected_relay_url().as_deref() == Some(relay_url) {
                return Some(state);
            }
        }
        None
    }

    /// The relay connection that already reaches the worker (its identity and
    /// its cached peer key), else a temporary connection on the binding's
    /// relay. A temporary connection discovers the worker through relay
    /// metadata, which hosted and slice workers are not visible to.
    pub(crate) async fn enqueue_leased_worker_request_authorized(
        &self,
        config: &crate::config::DaemonConfig,
        target: chariox_relay::protocol::ClientTarget,
        request: RelayPeerRequest,
        response_timeout: Duration,
        authorize: impl Fn() -> Result<(), DaemonError> + Send + Sync,
    ) -> Result<RelayPeerResponseWaiter, DaemonError> {
        match self.state_for_config(config).await {
            Some(relay_state) => {
                enqueue_peer_request_via_connected_relay_authorized(
                    config,
                    &relay_state,
                    target,
                    request,
                    response_timeout,
                    authorize,
                )
                .await
            }
            None => {
                // Temporary sockets have no shared FIFO sender with manifest
                // updates, so keep this fallback serialized through its response.
                let response = send_peer_request_via_temporary_connection_authorized(
                    config,
                    target,
                    request,
                    response_timeout,
                    authorize,
                )
                .await?;
                Ok(RelayPeerResponseWaiter::ready(response))
            }
        }
    }
}

/// A worker on the other end of a home's connected relay connection: the
/// home holds its peer key, and requests leave over the encrypted wire.
#[cfg(test)]
pub(crate) struct ConnectedWorkerRig {
    worker: crate::config::DaemonConfig,
    home_public_key: String,
    state: SharedRelayClientState,
    wire: tokio::sync::mpsc::Receiver<chariox_relay::protocol::RelayEnvelope>,
}

#[cfg(test)]
impl ConnectedWorkerRig {
    pub(crate) async fn connect(
        state: &SharedRelayClientState,
        home: &crate::config::DaemonConfig,
        relay_url: &str,
    ) -> Self {
        let worker = crate::config::DaemonConfig::for_tests();
        let (sender, wire, _events) = super::RelayOutgoingSender::channel(4);
        {
            let mut relay = state.write().await;
            relay.test_set_connected_sender(sender, relay_url);
            relay.remember_peer_public_key(&worker.daemon_id, worker.relay_public_key.clone());
        }
        Self {
            home_public_key: home.relay_public_key.clone(),
            state: Arc::clone(state),
            worker,
            wire,
        }
    }

    pub(crate) fn worker_kernel_id(&self) -> &str {
        &self.worker.daemon_id
    }

    /// Receive the next request over the connected wire, decrypt it as the
    /// worker, and answer it with `respond`. Panics when no request arrives.
    pub(crate) async fn answer(
        &mut self,
        respond: impl FnOnce(&RelayPeerRequest) -> crate::transport::relay_peer::RelayPeerResponse,
    ) -> RelayPeerRequest {
        use crate::transport::relay_crypto;
        let envelope = tokio::time::timeout(Duration::from_secs(5), self.wire.recv())
            .await
            .expect("the request must use the connected encrypted peer transport")
            .expect("the connected wire stays open");
        let chariox_relay::protocol::RelayEnvelope::DaemonPeerRequest {
            request_id,
            target,
            encrypted_request,
        } = envelope
        else {
            panic!("expected a peer request")
        };
        assert_eq!(
            target.daemon_id.as_deref(),
            Some(self.worker.daemon_id.as_str())
        );
        let decoded = relay_crypto::decrypt_payload_for_private_key(
            &self.worker.relay_private_key,
            &encrypted_request,
        )
        .unwrap();
        let request: RelayPeerRequest = serde_json::from_slice(&decoded.plaintext).unwrap();
        let encrypted = relay_crypto::encrypt_payload_for_peer(
            &self.worker.relay_private_key,
            &self.home_public_key,
            &serde_json::to_vec(&respond(&request)).unwrap(),
        )
        .unwrap();
        super::resolve_pending_peer_response_for_test(
            &self.state,
            request_id,
            self.worker.daemon_id.clone(),
            encrypted,
        )
        .await;
        request
    }
}
