//! MP-08 / MP-10 / MP-11: prompt admission follows the selected run's account.
use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn require_agent_account_authenticated(
        &self,
        agent: &crate::agent::AgentInstance,
        selected_run: Option<&crate::provider::RuntimeProviderRun>,
        operation: &'static str,
    ) -> Result<(), DaemonError> {
        let Some((owner, provider, profile)) =
            self.agent_account_for_prompt(agent, selected_run)?
        else {
            return Ok(());
        };
        self.provider_account_profiles.require_authenticated(
            &owner,
            provider,
            &profile,
            agent.model(),
            operation,
        )?;
        Ok(())
    }

    fn agent_account_for_prompt(
        &self,
        agent: &crate::agent::AgentInstance,
        selected_run: Option<&crate::provider::RuntimeProviderRun>,
    ) -> Result<Option<(String, &'static str, String)>, DaemonError> {
        let config = self.config_projection.snapshot();
        let Some(provider) = crate::provider::canonical_provider_family(agent.provider()) else {
            return Ok(None);
        };
        let current = selected_run.cloned().or_else(|| {
            self.provider_store
                .get_run_for_agent(agent.session_id(), agent.id())
        });
        if let Some(run) = current.filter(|run| {
            agent.remote_execution().is_none()
                && run.agent_instance_id() == Some(agent.id())
                && run.session_id() == agent.session_id()
                && run.owner_user_id() == agent.owner_user_id()
                && run.provider() == agent.provider()
                && run.state() != crate::provider::ProviderRunState::Ended
                && agent.model().is_none_or(|model| model == run.model())
                && (agent.provider_account_profile() == run.account_profile()
                    || agent.provider_account_profile()
                        == crate::account_profile::provider_account_selection_for_run(run)
                    || (agent.provider_account_profile() == "default"
                        && run.resolved_provider_account.is_none()))
        }) {
            let owner = crate::account_profile::provider_account_authority_for_run(
                &config,
                &self.provider_account_profiles,
                &run,
            )?;
            // Restored runs recover their namespace from provider-local roots.
            // A home alias must never adopt a leased replica's namespace.
            let selection_matches = agent.provider_account_profile() == run.account_profile()
                || agent.provider_account_profile()
                    == crate::account_profile::provider_account_selection_for_run(&run);
            let alias_owner = crate::account_profile::provider_account_authority_owner_for_profile(
                &config,
                &self.provider_account_profiles,
                agent.owner_user_id(),
                provider,
                agent.provider_account_profile(),
            )?;
            if selection_matches || owner == alias_owner {
                // MP-08 / MP-11: the marked worker process already holds HOME's
                // launch credential; it has no independent worker login.
                if provider == "claude"
                    && self
                        .provider_run_projection
                        .is_leased_provider_run(run.id())
                    && matches!(
                        run.state(),
                        crate::provider::ProviderRunState::Starting
                            | crate::provider::ProviderRunState::Running
                    )
                    && owner == self.provider_account_authority_owner_for_agent(agent)?
                {
                    let selected = self.provider_account_profiles.get(
                        &owner,
                        provider,
                        agent.provider_account_profile(),
                    )?;
                    let launched = self.provider_account_profiles.get(
                        &owner,
                        provider,
                        run.account_profile(),
                    )?;
                    if selected.profile_id == launched.profile_id {
                        return Ok(None);
                    }
                }
                return Ok(Some((owner, provider, run.account_profile().to_string())));
            }
        }
        Ok(Some((
            self.provider_account_authority_owner_for_agent(agent)?,
            provider,
            agent.provider_account_profile().to_string(),
        )))
    }
}

impl KernelRuntimeState {
    pub(super) async fn refresh_unknown_prompt_account(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        let Some((owner, provider, selection)) =
            self.owned.agent_account_for_prompt(&agent, None)?
        else {
            return Ok(());
        };
        let registry = self.owned.provider_account_profiles.clone();
        let profile = registry.get(&owner, provider, &selection)?;
        if profile.auth_state != crate::account_profile::ProviderAccountAuthState::Unknown {
            return Ok(());
        }
        // Registered Claude credentials already have their own first-use check,
        // including home-token replicas that cannot authenticate independently.
        if !matches!(
            registry.require_authenticated(
                &owner,
                provider,
                &profile.profile_id,
                agent.model(),
                "submit prompt"
            ),
            Err(DaemonError::ProviderProfileRefused {
                reason: crate::error::ProviderProfileRefusal::Unverified,
                ..
            })
        ) {
            return Ok(());
        }
        let config = self.owned.config_projection.snapshot();
        let result = tokio::task::spawn_blocking(move || {
            crate::local::provider_requests::refresh_provider_account_profile_explicitly(
                &registry,
                &owner,
                provider,
                &profile.profile_id,
                &config,
            )
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "refresh unknown prompt account",
            message: error.to_string(),
        })?;
        self.with_app_side_effect(|app| app.invalidate_provider_catalog_cache())
            .await;
        self.record_waiting_room_change();
        result?;
        Ok(())
    }
}
