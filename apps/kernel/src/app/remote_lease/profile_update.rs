//! Validates a home-owned worker profile before the async compaction seam.
use super::RemoteLeaseRuntime;
use crate::error::DaemonError;
use crate::execution_lease::LeasedAgent;
use crate::transport::relay_peer::RelayAgentExecutionProfile;

pub(crate) struct PreparedLeasedProfileUpdate {
    pub(crate) leased_agent: LeasedAgent,
    pub(crate) profile: RelayAgentExecutionProfile,
    pub(crate) changed: bool,
}

impl RemoteLeaseRuntime<'_> {
    #[cfg(test)]
    pub(crate) fn update_leased_agent_profile(
        &mut self,
        leased_agent_id: &str,
        provider: String,
        account_profile: String,
        model: Option<String>,
        effort: Option<String>,
    ) -> Result<LeasedAgent, DaemonError> {
        let prepared = self.prepare_leased_agent_profile(
            leased_agent_id,
            provider,
            account_profile,
            model,
            effort,
        )?;
        self.commit_leased_agent_profile(prepared)
    }

    pub(crate) fn prepare_leased_agent_profile(
        &mut self,
        leased_agent_id: &str,
        provider: String,
        account_profile: String,
        model: Option<String>,
        effort: Option<String>,
    ) -> Result<PreparedLeasedProfileUpdate, DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        let account_profile =
            self.resolve_leased_profile_account(&leased_agent, &provider, &account_profile)?;
        let profile_changed = leased_agent.provider != provider
            || leased_agent.account_profile != account_profile
            || leased_agent.model != model
            || leased_agent.effort != effort;
        // A delivery retry may confirm its profile after the original prompt started.
        // Confirmation is read-only; an actual change still requires an idle agent.
        if !profile_changed {
            let backing = self.app.agents.get_agent(&leased_agent.backing_agent_id)?;
            if backing.provider() != provider
                || backing.provider_account_profile() != account_profile
                || backing.model() != model.as_deref()
                || backing.effort() != effort.as_deref()
            {
                return Err(DaemonError::LocalTransport {
                    operation: "confirm leased agent profile",
                    message: "leased profile differs from the backing agent; rebind the remote agent before dispatch".to_string(),
                });
            }
            return Ok(PreparedLeasedProfileUpdate {
                leased_agent,
                profile: crate::transport::relay_peer::RelayAgentExecutionProfile {
                    provider,
                    account_profile,
                    model,
                    effort,
                },
                changed: false,
            });
        }
        if self
            .app
            .prompt_owner_active_prompt_for_agent(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )?
            .is_some()
            || self
                .app
                .prompt_owner_peek_next_queued_prompt(
                    &leased_agent.backing_session_id,
                    &leased_agent.backing_agent_id,
                )?
                .is_some()
        {
            return Err(DaemonError::LocalTransport {
                operation: "update leased agent profile",
                message: format!(
                    "leased agent `{leased_agent_id}` has an active turn or queued prompt; update the profile after pending work finishes"
                ),
            });
        }

        Ok(PreparedLeasedProfileUpdate {
            leased_agent,
            profile: RelayAgentExecutionProfile {
                provider,
                account_profile,
                model,
                effort,
            },
            changed: true,
        })
    }

    pub(crate) fn commit_leased_agent_profile(
        &mut self,
        prepared: PreparedLeasedProfileUpdate,
    ) -> Result<LeasedAgent, DaemonError> {
        let crate::transport::relay_peer::RelayAgentExecutionProfile {
            provider,
            account_profile,
            model,
            effort,
        } = prepared.profile;
        let leased_agent = prepared.leased_agent;
        let leased_agent_id = leased_agent.id.as_str();
        // Projection cursors may advance while /compact runs. The selected
        // profile and backing identity must still be the validated ones.
        let current = self.app.leased_agents.get(leased_agent_id).ok_or_else(|| {
            DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            }
        })?;
        if current.provider != leased_agent.provider
            || current.account_profile != leased_agent.account_profile
            || current.model != leased_agent.model
            || current.effort != leased_agent.effort
            || current.backing_agent_id != leased_agent.backing_agent_id
            || current.backing_session_id != leased_agent.backing_session_id
        {
            return Err(DaemonError::LocalTransport {
                operation: "update leased agent profile",
                message: "leased profile changed while preparing the transition".into(),
            });
        }
        if !prepared.changed {
            return Ok(current.clone());
        }
        self.resolve_leased_profile_account(&leased_agent, &provider, &account_profile)?;
        if self
            .app
            .prompt_owner_active_prompt_for_agent(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )?
            .is_some()
        {
            return Err(DaemonError::LocalTransport {
                operation: "update leased agent profile",
                message: "leased agent has an active turn".into(),
            });
        }
        self.terminate_backing_provider_runtime(&leased_agent);
        let backing_agent = self.app.agents.get_agent(&leased_agent.backing_agent_id)?;
        let resume_state = backing_agent.provider_resume_state().after_profile_change(
            backing_agent.provider(),
            backing_agent.provider_account_profile(),
            &provider,
            &account_profile,
        );
        self.app
            .agents
            .set_agent_runtime_profile_with_account_profile(
                &leased_agent.backing_agent_id,
                &provider,
                model.clone(),
                effort.clone(),
                Some(account_profile.clone()),
                resume_state,
            )?;
        let updated = self
            .app
            .leased_agents
            .get_mut(leased_agent_id)
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        updated.provider = provider;
        updated.account_profile = account_profile;
        updated.model = model;
        updated.effort = effort;
        Ok(updated.clone())
    }
}
