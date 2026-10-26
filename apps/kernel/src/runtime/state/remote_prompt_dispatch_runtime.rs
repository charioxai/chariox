//! Remote prompt dispatch coordinator wiring.

use super::remote_prompt_claim_runtime::RemotePromptAgentClaim;
use super::*;

impl KernelRuntimeState {
    pub(super) async fn connected_relay_state_for_config(
        &self,
        relay_config: &crate::config::DaemonConfig,
    ) -> Option<Arc<tokio::sync::RwLock<crate::transport::relay_client::RelayClientState>>> {
        let relay_url = relay_config.relay_url.as_deref()?;
        if self
            .owned
            .relay_state
            .read()
            .await
            .connected_relay_url()
            .as_deref()
            == Some(relay_url)
        {
            return Some(Arc::clone(&self.owned.relay_state));
        }
        let slice_states = {
            let connectors = self.owned.slice_private_relay_connectors.lock().await;
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

    /// The one route of every home-to-worker request for a leased agent: the
    /// relay connection that already reaches the worker (its identity and its
    /// cached peer key), else a temporary connection on the binding's relay.
    /// A temporary connection discovers the worker through relay metadata,
    /// which hosted and slice workers are not visible to.
    pub(super) async fn enqueue_leased_worker_request(
        &self,
        config: &crate::config::DaemonConfig,
        target: chariox_relay::protocol::ClientTarget,
        request: crate::transport::relay_peer::RelayPeerRequest,
        response_timeout: std::time::Duration,
    ) -> Result<crate::transport::relay_client::RelayPeerResponseWaiter, DaemonError> {
        match self.connected_relay_state_for_config(config).await {
            Some(relay_state) => {
                crate::transport::relay_client::enqueue_peer_request_via_connected_relay_with_timeout(
                    config,
                    &relay_state,
                    target,
                    request,
                    response_timeout,
                )
                .await
            }
            None => {
                // Temporary sockets have no shared FIFO sender with manifest
                // updates, so keep this fallback serialized through its response.
                let response = crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
                    config,
                    target,
                    request,
                    response_timeout,
                )
                .await?;
                Ok(crate::transport::relay_client::RelayPeerResponseWaiter::ready(response))
            }
        }
    }

    pub(crate) fn spawn_remote_prompt_dispatch(
        &self,
        dispatch: crate::app::KernelRemotePromptDispatch,
    ) {
        // A stale projection drain can discover a dead lease while the initial
        // dispatch is already refreshing that same binding. Both paths submit
        // the active prompt, so serialize them per agent to prevent one browser
        // prompt from starting on two freshly-created worker agents.
        let Some(claim) = RemotePromptAgentClaim::try_acquire_or_defer_dispatch(
            Arc::clone(&self.owned.remote_prompt_recoveries),
            &dispatch,
        ) else {
            return;
        };
        let state = self.clone();
        tokio::spawn(async move {
            state
                .run_remote_prompt_dispatch_with_claim(claim, Some(dispatch))
                .await;
        });
    }
}

#[cfg(test)]
#[path = "remote_prompt_dispatch_runtime/tests.rs"]
mod tests;
