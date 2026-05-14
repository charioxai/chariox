use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn update_agent_config(
        &self,
        session_id: &str,
        agent_id: &str,
        caller_user_id: &str,
        execution_mode_override: Option<Option<crate::provider::AgentExecutionMode>>,
        permission_level_override: Option<Option<crate::provider::AgentPermissionLevel>>,
        workspace_id: Option<Option<String>>,
        worktree_id: Option<Option<String>>,
    ) -> Result<owned::OwnedAgentConfigUpdate, DaemonError> {
        let workspace_id = workspace_id.map(|value| {
            value.and_then(|value| {
                let trimmed = value.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_string())
            })
        });
        let worktree_id = worktree_id.map(|value| {
            value.and_then(|value| {
                let trimmed = value.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_string())
            })
        });
        let agent = self.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        self.ensure_agent_owner(agent_id, caller_user_id, "update agent config")?;
        let session = self.session_store.get_session(session_id)?;
        if self
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .is_some()
            || agent.is_processing()
        {
            return Err(DaemonError::LocalTransport {
                operation: "update agent config",
                message: format!(
                    "agent `{agent_id}` has an active turn; update the config after it finishes"
                ),
            });
        }
        self.ensure_agent_config_not_provider_native_tui(
            session_id,
            agent_id,
            "update agent config",
        )?;
        let previous_config =
            crate::session::effective_agent_execution_config(&session, Some(&agent));
        let mut next_agent = agent.clone();
        if let Some(execution_mode_override) = execution_mode_override {
            next_agent.set_execution_mode_override(execution_mode_override);
        }
        if let Some(permission_level_override) = permission_level_override {
            next_agent.set_permission_level_override(permission_level_override);
        }
        if let Some(workspace_id) = workspace_id.clone() {
            next_agent.set_workspace_id(workspace_id);
        }
        if let Some(worktree_id) = worktree_id.clone() {
            next_agent.set_worktree_id(worktree_id);
        }
        let next_config =
            crate::session::effective_agent_execution_config(&session, Some(&next_agent));
        let effective_config_changed = previous_config.mode != next_config.mode
            || previous_config.permission_level != next_config.permission_level;
        let remote_update = effective_config_changed
            .then(|| {
                agent
                    .remote_execution()
                    .map(|binding| owned::OwnedRemoteAgentConfigUpdate {
                        worker_kernel_id: binding.worker_kernel_id.clone(),
                        leased_agent_id: binding.leased_agent_id.clone(),
                        relay_url: binding.relay_url.clone(),
                        relay_token: binding.relay_token.clone(),
                        execution_mode: next_config.mode,
                        permission_level: next_config.permission_level,
                    })
            })
            .flatten();

        let mut terminated_run_ids = Vec::new();
        if effective_config_changed && remote_update.is_none() {
            if let Some(run) = self.provider_store.get_run_for_agent(session_id, agent_id) {
                match run.state() {
                    crate::provider::ProviderRunState::Starting
                    | crate::provider::ProviderRunState::Running
                    | crate::provider::ProviderRunState::Parked => {
                        if self
                            .provider_store
                            .adapter_supports_turn_scoped_execution_config(run.adapter_key())
                        {
                            let updated = self.provider_store.update_run_execution_config(
                                run.id(),
                                next_config.mode,
                                next_config.permission_level,
                            )?;
                            self.provider_run_projection.update(updated);
                        } else {
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
                    }
                    crate::provider::ProviderRunState::Ended => {
                        self.provider_store.clear_runtime(run.id());
                    }
                }
            }
        }
        let agent = if remote_update.is_some() {
            next_agent
        } else {
            self.agent_store.update_agent_config(
                agent_id,
                execution_mode_override,
                permission_level_override,
                workspace_id,
                worktree_id,
            )?;
            let agent = self.agent_store.get_agent(agent_id)?;
            let _ = self.session_snapshot(session_id)?;
            agent
        };
        Ok(owned::OwnedAgentConfigUpdate {
            agent,
            terminated_run_ids,
            remote_update,
        })
    }

    pub(super) fn commit_remote_agent_config_update(
        &self,
        session_id: &str,
        agent_id: &str,
        execution_mode_override: Option<Option<crate::provider::AgentExecutionMode>>,
        permission_level_override: Option<Option<crate::provider::AgentPermissionLevel>>,
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
            || agent.is_processing()
        {
            return Err(DaemonError::LocalTransport {
                operation: "commit remote agent config",
                message: format!(
                    "agent `{agent_id}` has an active turn; update the config after it finishes"
                ),
            });
        }
        self.agent_store.update_agent_config(
            agent_id,
            execution_mode_override,
            permission_level_override,
            None,
            None,
        )?;
        let agent = self.agent_store.get_agent(agent_id)?;
        let _ = self.session_snapshot(session_id)?;
        Ok(agent)
    }

    pub(super) fn update_agent_profile(
        &self,
        session_id: &str,
        agent_id: &str,
        caller_user_id: &str,
        provider: Option<String>,
        model: Option<String>,
        effort: Option<Option<String>>,
    ) -> Result<(crate::agent::AgentInstance, Vec<String>), DaemonError> {
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
            || agent.is_processing()
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
        let mut terminated_run_ids = Vec::new();
        if let Some(run) = self.provider_store.get_run_for_agent(session_id, agent_id) {
            match run.state() {
                crate::provider::ProviderRunState::Starting
                | crate::provider::ProviderRunState::Running
                | crate::provider::ProviderRunState::Parked => {
                    let outcome = self
                        .provider_store
                        .terminate_run_provider_only(session_id, run.id())?;
                    self.clear_active_provider_run_session_pointer(session_id, outcome.run().id())?;
                    let ended = outcome.into_run();
                    terminated_run_ids.push(ended.id().to_string());
                    self.provider_run_projection.update(ended);
                }
                crate::provider::ProviderRunState::Ended => {
                    self.provider_store.clear_runtime(run.id());
                }
            }
        }
        let agent = self
            .agent_store
            .update_agent_profile(agent_id, provider, model, effort)?;
        let _ = self.session_snapshot(session_id)?;
        Ok((agent, terminated_run_ids))
    }

    fn ensure_agent_config_not_provider_native_tui(
        &self,
        session_id: &str,
        agent_id: &str,
        operation: &'static str,
    ) -> Result<(), DaemonError> {
        let Some(run) = self.provider_store.get_run_for_agent(session_id, agent_id) else {
            return Ok(());
        };
        if run.client_interface().is_arroba() {
            return Ok(());
        }
        Err(DaemonError::LocalTransport {
            operation,
            message: format!(
                "agent `{agent_id}` is controlled by a provider-native TUI; change provider settings in that TUI"
            ),
        })
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

    pub(super) fn grant_agent_mcp(
        &self,
        agent_ref: &str,
        name: String,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.ensure_agent_ref_owner(agent_ref, caller_user_id, "grant agent capability")?;
        let agent = self.agent_store.grant_mcp(agent_ref, name)?;
        let _ = self.session_snapshot(agent.session_id())?;
        Ok(agent)
    }

    pub(super) fn revoke_agent_mcp(
        &self,
        agent_ref: &str,
        name: &str,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.ensure_agent_ref_owner(agent_ref, caller_user_id, "revoke agent capability")?;
        let agent = self.agent_store.revoke_mcp(agent_ref, name)?;
        let _ = self.session_snapshot(agent.session_id())?;
        Ok(agent)
    }

    pub(super) fn grant_agent_skill(
        &self,
        agent_ref: &str,
        name: String,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.ensure_agent_ref_owner(agent_ref, caller_user_id, "grant agent capability")?;
        let agent = self.agent_store.grant_skill(agent_ref, name)?;
        let _ = self.session_snapshot(agent.session_id())?;
        Ok(agent)
    }

    pub(super) fn revoke_agent_skill(
        &self,
        agent_ref: &str,
        name: &str,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.ensure_agent_ref_owner(agent_ref, caller_user_id, "revoke agent capability")?;
        let agent = self.agent_store.revoke_skill(agent_ref, name)?;
        let _ = self.session_snapshot(agent.session_id())?;
        Ok(agent)
    }
}
