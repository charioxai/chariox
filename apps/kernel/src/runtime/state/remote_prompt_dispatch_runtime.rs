//! Remote prompt dispatch coordinator wiring.

use super::remote_prompt_claim_runtime::RemotePromptAgentClaim;
use super::*;

impl KernelRuntimeState {
    pub(super) fn connected_relays(&self) -> crate::transport::relay_client::ConnectedRelays {
        crate::transport::relay_client::ConnectedRelays {
            primary: Arc::clone(&self.owned.relay_state),
            slice_private: Arc::clone(&self.owned.slice_private_relay_connectors),
        }
    }

    pub(super) async fn connected_relay_state_for_config(
        &self,
        relay_config: &crate::config::DaemonConfig,
    ) -> Option<Arc<tokio::sync::RwLock<crate::transport::relay_client::RelayClientState>>> {
        self.connected_relays().state_for_config(relay_config).await
    }

    /// The one route of every home-to-worker request for a leased agent; see
    /// [`ConnectedRelays::enqueue_leased_worker_request_authorized`].
    pub(super) async fn enqueue_leased_worker_request(
        &self,
        config: &crate::config::DaemonConfig,
        target: chariox_relay::protocol::ClientTarget,
        request: crate::transport::relay_peer::RelayPeerRequest,
        response_timeout: std::time::Duration,
    ) -> Result<crate::transport::relay_client::RelayPeerResponseWaiter, DaemonError> {
        self.enqueue_leased_worker_request_authorized(
            config,
            target,
            request,
            response_timeout,
            || Ok(()),
        )
        .await
    }

    pub(super) async fn enqueue_leased_worker_request_authorized(
        &self,
        config: &crate::config::DaemonConfig,
        target: chariox_relay::protocol::ClientTarget,
        request: crate::transport::relay_peer::RelayPeerRequest,
        response_timeout: std::time::Duration,
        authorize: impl Fn() -> Result<(), DaemonError> + Send + Sync,
    ) -> Result<crate::transport::relay_client::RelayPeerResponseWaiter, DaemonError> {
        self.connected_relays()
            .enqueue_leased_worker_request_authorized(
                config,
                target,
                request,
                response_timeout,
                authorize,
            )
            .await
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
