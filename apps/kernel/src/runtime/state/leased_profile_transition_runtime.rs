//! Worker-local part of the existing home-authoritative leased profile update.
use super::*;

impl KernelRuntimeState {
    /// Caller holds the existing leased-agent operation guard and has consumed
    /// relay authorization. Standard workers and slice workers use this seam.
    pub(super) async fn apply_leased_profile_transition(
        &self,
        leased_agent_id: &str,
        profile: crate::transport::relay_peer::RelayAgentExecutionProfile,
        while_turn_runs: bool,
        provider_resume_state: Option<crate::provider::ProviderResumeState>,
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
                    while_turn_runs,
                )
            })
            .await?;
        let session_id = prepared.leased_agent.backing_session_id.clone();
        let agent_id = prepared.leased_agent.backing_agent_id.clone();
        // A running turn is neither compacted nor held: its process takes the profile at its next
        // step boundary and the window stays the provider's to manage.
        let claim = if prepared.changed && !prepared.while_turn_runs {
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
            .await?;
            Some(claim)
        } else {
            None
        };
        let result = self
            .with_app_side_effect(move |app| {
                // Seed the home's state while it still names the previous physical account.
                // The profile commit must be the last authority on which sessions survive.
                if let Some(resume) = provider_resume_state {
                    crate::app::RemoteLeaseRuntime::new(app)
                        .seed_leased_agent_resume_state(&prepared.leased_agent.id, resume)?;
                }
                crate::app::RemoteLeaseRuntime::new(app).commit_leased_agent_profile(prepared)
            })
            .await;
        drop(claim);
        // The operation guard excludes relay prompt admission during compaction.
        // A backing backlog must wait for the home's credential-bearing leased
        // submission: profile updates carry no launch credential. That launch
        // marks the run leased and promotes queued work when it becomes ready.
        // Never cold-launch this backlog through the worker's local account.
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
