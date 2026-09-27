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

    pub(crate) fn spawn_remote_prompt_dispatch(
        &self,
        dispatch: crate::app::KernelRemotePromptDispatch,
    ) {
        // A stale projection drain can discover a dead lease while the initial
        // dispatch is already refreshing that same binding. Both paths submit
        // the active prompt, so serialize them per agent to prevent one browser
        // prompt from starting on two freshly-created worker agents.
        let Some(claim) = RemotePromptAgentClaim::try_acquire(
            Arc::clone(&self.owned.remote_prompt_recoveries),
            &dispatch.session_id,
            &dispatch.agent_id,
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
