//! Agent profile, alias, and substitute administration.
//!
//! Execution-mode and permission overrides live in `agent_config_owned_state`; capability grants
//! live in `capability_owned_state`.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn update_agent_profile(
        &self,
        session_id: &str,
        agent_id: &str,
        caller_user_id: &str,
        provider: Option<String>,
        model: Option<String>,
        effort: Option<Option<String>>,
    ) -> Result<owned::OwnedAgentProfileUpdate, DaemonError> {
        let provider = provider
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let model = model
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let effort = effort.map(|value| {
            value
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        });
        let agent = self.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        self.ensure_agent_owner(agent_id, caller_user_id, "update agent profile")?;
        let session = self.session_store.get_session(session_id)?;
        if self
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .is_some()
        {
            return Err(DaemonError::LocalTransport {
                operation: "update agent profile",
                message: format!(
                    "agent `{agent_id}` has an active turn; update the profile after it finishes"
                ),
            });
        }
        self.ensure_agent_config_not_provider_native_tui(
            session_id,
            agent_id,
            "update agent profile",
        )?;
        let target_provider = provider
            .as_deref()
            .unwrap_or_else(|| agent.provider())
            .to_string();
        let target_model = model
            .as_deref()
            .or_else(|| agent.model())
            .map(str::to_string);
        let target_effort = match effort.as_ref() {
            Some(value) => value.as_deref(),
            None => agent.effort(),
        };
        let provider_or_model_changed =
            target_provider != agent.provider() || target_model.as_deref() != agent.model();
        if !provider_or_model_changed && target_effort == agent.effort() {
            return Ok(owned::OwnedAgentProfileUpdate {
                agent,
                terminated_run_ids: Vec::new(),
                remote_update: None,
            });
        }
        let remote_update =
            agent
                .remote_execution()
                .map(|binding| owned::OwnedRemoteAgentProfileUpdate {
                    worker_kernel_id: binding.worker_kernel_id.clone(),
                    leased_agent_id: binding.leased_agent_id.clone(),
                    relay_url: binding.relay_url.clone(),
                    relay_token: binding.relay_token.clone(),
                    provider: target_provider.clone(),
                    model: target_model.clone(),
                    effort: target_effort.map(str::to_string),
                });
        let mut terminated_run_ids = Vec::new();
        if remote_update.is_none() {
            if let Some(run) = self.provider_store.get_run_for_agent(session_id, agent_id) {
                match run.state() {
                    crate::provider::ProviderRunState::Starting
                    | crate::provider::ProviderRunState::Running
                    | crate::provider::ProviderRunState::Parked => {
                        if provider_or_model_changed {
                            self.prepare_agent_profile_context_handoff(
                                &run,
                                &target_provider,
                                target_model.as_deref(),
                            );
                        }
                        let outcome = self
                            .provider_store
                            .terminate_run_provider_only(session_id, run.id())?;
                        self.clear_active_provider_run_session_pointer(
                            session_id,
                            outcome.run().id(),
                        )?;
                        let ended = outcome.into_run();
                        terminated_run_ids.push(ended.id().to_string());
                        self.provider_run_projection.update(ended);
                    }
                    crate::provider::ProviderRunState::Ended => {
                        self.provider_store.clear_runtime(run.id());
                    }
                }
            }
        }
        let agent = if remote_update.is_some() {
            agent
        } else {
            let agent = self
                .agent_store
                .update_agent_profile(agent_id, provider, model, effort)?;
            let _ = self.session_snapshot(session_id)?;
            agent
        };
        Ok(owned::OwnedAgentProfileUpdate {
            agent,
            terminated_run_ids,
            remote_update,
        })
    }

    pub(super) fn commit_remote_agent_profile_update(
        &self,
        session_id: &str,
        agent_id: &str,
        provider: String,
        model: Option<String>,
        effort: Option<String>,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        let session = self.session_store.get_session(session_id)?;
        if self
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .is_some()
        {
            return Err(DaemonError::LocalTransport {
                operation: "commit remote agent profile",
                message: format!(
                    "agent `{agent_id}` has an active turn; update the profile after it finishes"
                ),
            });
        }
        self.agent_store
            .update_agent_profile(agent_id, Some(provider), model, Some(effort))?;
        self.agent_store
            .set_remote_execution_active_worker_provider_run_id(agent_id, None)?;
        let agent = self.agent_store.get_agent(agent_id)?;
        let _ = self.session_snapshot(session_id)?;
        Ok(agent)
    }

    pub(super) fn alias_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        caller_user_id: &str,
        alias: Option<String>,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        self.ensure_agent_owner(agent_id, caller_user_id, "alias agent")?;
        self.agent_store.alias_agent(agent_id, alias)
    }

    pub(super) fn update_agent_substitutes(
        &self,
        session_id: &str,
        agent_id: &str,
        caller_user_id: &str,
        action: crate::local::AgentSubstituteAction,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        self.ensure_agent_owner(agent_id, caller_user_id, "update agent substitutes")?;
        match action {
            crate::local::AgentSubstituteAction::Add {
                provider,
                model,
                variant,
                kernel_id,
                worktree_id,
            } => {
                let kernel_id = kernel_id.and_then(|value| {
                    let trimmed = value.trim();
                    (!trimmed.is_empty()).then(|| trimmed.to_string())
                });
                let worktree_id = worktree_id.and_then(|value| {
                    let trimmed = value.trim();
                    (!trimmed.is_empty()).then(|| trimmed.to_string())
                });
                if let Some(kernel_id) = kernel_id.as_deref() {
                    let local_kernel_id = self.config_projection.snapshot().daemon_id;
                    if kernel_id != local_kernel_id {
                        return Err(DaemonError::LocalTransport {
                            operation: "add agent substitute",
                            message: format!(
                                "remote substitute kernel `{kernel_id}` is not supported yet"
                            ),
                        });
                    }
                }
                self.agent_store.add_agent_substitute(
                    agent_id,
                    crate::agent::AgentSubstituteProfile::new(provider, model, variant)
                        .with_kernel_id(kernel_id)
                        .with_worktree_id(worktree_id),
                )
            }
            crate::local::AgentSubstituteAction::Remove { index } => {
                self.agent_store.remove_agent_substitute(agent_id, index)
            }
            crate::local::AgentSubstituteAction::Clear {} => {
                self.agent_store.clear_agent_substitutes(agent_id)
            }
            crate::local::AgentSubstituteAction::SetTimeout { timeout_ms } => self
                .agent_store
                .set_agent_substitution_timeout(agent_id, timeout_ms),
            crate::local::AgentSubstituteAction::Activate { index, reason } => self
                .agent_store
                .activate_agent_substitute(
                    agent_id,
                    index,
                    reason.unwrap_or_else(|| "manual".to_string()),
                )
                .map(|(agent, _profile)| agent),
            crate::local::AgentSubstituteAction::Primary {} => {
                self.agent_store.deactivate_agent_substitute(agent_id)
            }
        }
    }
}
