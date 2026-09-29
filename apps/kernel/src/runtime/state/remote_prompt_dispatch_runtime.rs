//! Remote prompt dispatch coordinator wiring.

use super::remote_prompt_claim_runtime::RemotePromptAgentClaim;
use super::*;

impl KernelRuntimeState {
    /// Send a Room request to its slice worker the way browser actions do: over the
    /// connected relay when there is one. A temporary connection first needs relay
    /// metadata access, which a kernel-scoped Cloud relay token does not carry.
    pub(super) async fn send_room_slice_peer_request(
        &self,
        config: &crate::config::DaemonConfig,
        target: chariox_relay::protocol::ClientTarget,
        request: RelayPeerRequest,
        timeout: std::time::Duration,
    ) -> Result<RelayPeerResponse, DaemonError> {
        match self.connected_relay_state_for_config(config).await {
            Some(relay_state) => {
                crate::transport::relay_client::send_peer_request_via_connected_relay_with_timeout(
                    config,
                    &relay_state,
                    target,
                    request,
                    timeout,
                )
                .await
            }
            None => {
                crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
                    config, target, request, timeout,
                )
                .await
            }
        }
    }

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
