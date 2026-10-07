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
        let session_id = prepared.leased_agent.backing_session_id.clone();
        let agent_id = prepared.leased_agent.backing_agent_id.clone();
        let claim = if prepared.changed {
            let session = self.owned.session_store.get_session(&session_id)?;
            let claim = self
                .owned
                .prompt_state_owner
                .claim_idle_agent_profile_transition(&session, &agent_id)?;
            let agent = self.owned.agent_store.get_agent(&agent_id)?;
            self.compact_before_window_downshift(
                &session_id,
                &agent,
                Some(&prepared.profile.provider),
                Some(&prepared.profile.account_profile),
                prepared.profile.model.as_deref(),
            )
            .await;
            Some(claim)
        } else {
            None
        };
        let resume_queue = claim.is_some();
        let result = self
            .with_app_side_effect(move |app| {
                crate::app::RemoteLeaseRuntime::new(app).commit_leased_agent_profile(prepared)
            })
            .await;
        drop(claim);
        if resume_queue {
            // Commit/rejection precedes promotion. Project/provider preparation
            // runs off the acknowledgement path, as it does for local updates.
            let promotion =
                self.spawn_project_queued_prompt_after_profile_transition(&session_id, &agent_id);
            if result.is_ok() {
                promotion?;
            }
        }
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
