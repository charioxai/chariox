//! Worker-local part of the existing home-authoritative leased profile update.
use super::*;

impl KernelRuntimeState {
    /// Caller holds the existing leased-agent operation guard and has consumed
    /// relay authorization. Standard workers and slice workers use this seam.
    pub(super) async fn apply_leased_profile_transition(
        &self,
        leased_agent_id: &str,
        profile: crate::transport::relay_peer::RelayAgentExecutionProfile,
    ) -> Result<crate::execution_lease::LeasedAgent, DaemonError> {
        let id = leased_agent_id.to_string();
        let prepared = self
            .with_app_side_effect(move |app| {
                crate::app::RemoteLeaseRuntime::new(app).prepare_leased_agent_profile(
                    &id,
                    profile.provider,
                    profile.account_profile,
                    profile.model,
                    profile.effort,
                )
            })
            .await?;
        let claim = if prepared.changed {
            let lease = &prepared.leased_agent;
            let session = self
                .owned
                .session_store
                .get_session(&lease.backing_session_id)?;
            Some(
                self.owned
                    .prompt_state_owner
                    .claim_idle_agent_profile_transition(&session, &lease.backing_agent_id)?,
            )
        } else {
            None
        };
        if prepared.changed {
            let lease = &prepared.leased_agent;
            let agent = self.owned.agent_store.get_agent(&lease.backing_agent_id)?;
            self.compact_before_window_downshift(
                &lease.backing_session_id,
                &agent,
                Some(&prepared.profile.provider),
                Some(&prepared.profile.account_profile),
                prepared.profile.model.as_deref(),
            )
            .await;
        }
        let result = self
            .with_app_side_effect(move |app| {
                crate::app::RemoteLeaseRuntime::new(app).commit_leased_agent_profile(prepared)
            })
            .await;
        drop(claim);
        result
    }
}

/// A profile reconciliation can invoke the worker's native session command.
/// Keep the original transport allowance after that bounded command finishes.
pub(super) fn worker_profile_response_timeout(
    normal: std::time::Duration,
    provider: &str,
) -> std::time::Duration {
    if crate::provider::canonical_provider_family(provider) == Some("claude") {
        super::agent_profile_compaction::COMPACT_TIMEOUT.saturating_add(normal)
    } else {
        normal
    }
}
