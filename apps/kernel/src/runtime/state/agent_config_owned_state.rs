use super::*;

impl KernelRuntimeOwnedState {
    #[allow(
        clippy::too_many_arguments,
        reason = "Preserve the existing update_agent_config operation signature and explicit context arguments"
    )]
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
        let active_turn = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .is_some();
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
        let working_directory_changed = next_agent.workspace_id() != agent.workspace_id()
            || next_agent.worktree_id() != agent.worktree_id();
        // The person can lower or raise a running agent: every permission lookup reads the new
        // level and mode at use, and a provider process that takes them only at launch keeps its
        // own for the running turn; the next dispatch moves it. Moving the agent needs the turn to end.
        if active_turn && working_directory_changed {
            return Err(DaemonError::LocalTransport {
                operation: "update agent config",
                message: format!(
                    "agent `{agent_id}` has an active turn; move its workspace or worktree after it finishes"
                ),
            });
        }
        let scope_changed = effective_config_changed || working_directory_changed;
        // The agent runs, here or on its worker, with what its parent chain allows.
        let agents = self
            .agent_store
            .get_session_agents(session_id)
            .into_iter()
            .map(|other| {
                if other.id() == agent_id {
                    next_agent.clone()
                } else {
                    other
                }
            })
            .collect::<Vec<_>>();
        let limited = crate::session::effective_agent_execution_config_in_tree(
            &session,
            &next_agent,
            &agents,
        );
        let remote_update = effective_config_changed
            .then(|| {
                agent
                    .remote_execution()
                    .map(|binding| leased_agent_limit(binding, limited))
            })
            .flatten();

        let terminated_run_ids = if scope_changed && remote_update.is_none() {
            self.apply_agent_execution_config_to_provider(
                session_id,
                agent_id,
                limited,
                working_directory_changed,
                active_turn,
            )?
        } else {
            Vec::new()
        };
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
            scope_changed,
        })
    }

    /// The leased agents among `agent_id` and its descendants, by home agent id.
    pub(super) fn leased_subtree(&self, session_id: &str, agent_id: &str) -> Vec<String> {
        self.subtree_agents(session_id, agent_id, true)
    }

    /// The local agents among `agent_id` and its descendants.
    pub(super) fn local_subtree(&self, session_id: &str, agent_id: &str) -> Vec<String> {
        self.subtree_agents(session_id, agent_id, false)
    }

    fn subtree_agents(&self, session_id: &str, agent_id: &str, leased: bool) -> Vec<String> {
        let agents = self.agent_store.get_session_agents(session_id);
        agents
            .iter()
            .filter(|agent| agent.id() == agent_id)
            .chain(crate::agent::authority_limit::descendants(
                agent_id, &agents,
            ))
            .filter(|agent| !agent.archived() && agent.remote_execution().is_some() == leased)
            .map(|agent| agent.id().to_owned())
            .collect()
    }

    /// The leased agents of the room, by home agent id.
    pub(super) fn leased_room_agents(&self, session_id: &str) -> Vec<String> {
        self.room_agents(session_id, true)
    }

    /// The local agents of the room.
    pub(super) fn local_room_agents(&self, session_id: &str) -> Vec<String> {
        self.room_agents(session_id, false)
    }

    fn room_agents(&self, session_id: &str, leased: bool) -> Vec<String> {
        self.agent_store
            .get_session_agents(session_id)
            .into_iter()
            .filter(|agent| !agent.archived() && agent.remote_execution().is_some() == leased)
            .map(|agent| agent.id().to_owned())
            .collect()
    }

    /// Moves the live provider run of each local agent among `agent_ids` to the limit its parent
    /// chain sets now, so its next turn starts there. An idle run moves now: in place when its
    /// provider takes settings per turn, otherwise its process ends and the next turn launches at
    /// the limit. A run in a turn keeps that turn: it only takes a lowered limit in place, and
    /// the next dispatch moves it otherwise. The limit is read again after each move, so the run
    /// ends at the current limit whichever change moved it last.
    pub(super) fn limit_local_runs(
        &self,
        operation: &'static str,
        session_id: &str,
        agent_ids: &[String],
        retry: &str,
    ) -> Result<LocalRunLimits, DaemonError> {
        const MOVES: usize = 4;
        let mut limits = LocalRunLimits::default();
        for agent_id in agent_ids {
            for moved in 0.. {
                let Some(off_limit) = self.local_run_off_limit(session_id, agent_id)? else {
                    break;
                };
                if !limits.relimited.contains(agent_id) {
                    limits.relimited.push(agent_id.clone());
                }
                let OffLimitRun::MovesNow(limit) = off_limit else {
                    break;
                };
                if moved == MOVES {
                    return Err(DaemonError::LocalTransport {
                        operation,
                        message: format!(
                            "the limit of agent `{agent_id}` kept changing while its provider run moved to it; {retry}"
                        ),
                    });
                }
                // Only an idle run, or one its provider moves in place, gets here: no turn ends.
                limits
                    .terminated_run_ids
                    .extend(self.apply_agent_execution_config_to_provider(
                        session_id, agent_id, limit, false, false,
                    )?);
            }
        }
        Ok(limits)
    }

    /// How local `agent_id`'s live provider run differs from its limit; `None` once it runs at
    /// it, has no live run, is controlled by a provider-native TUI or is leased.
    pub(super) fn local_run_off_limit(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Option<OffLimitRun>, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let agents = self.agent_store.get_session_agents(session_id);
        let Some(agent) = agents.iter().find(|agent| {
            agent.id() == agent_id && !agent.archived() && agent.remote_execution().is_none()
        }) else {
            return Ok(None);
        };
        let limit =
            crate::session::effective_agent_execution_config_in_tree(&session, agent, &agents);
        let Some(run) = self
            .provider_store
            .get_run_for_agent(session_id, agent_id)
            .filter(|run| {
                run.state() != crate::provider::ProviderRunState::Ended
                    && run.client_interface().is_chariox()
                    && (run.execution_mode(), run.permission_level())
                        != (limit.mode, limit.permission_level)
            })
        else {
            return Ok(None);
        };
        let active_turn = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .is_some();
        // As for one agent's own update: a running turn takes a lowered setting in place at once,
        // since every permission lookup reads it at use, and anything else at its next dispatch.
        let lowers_only = run.execution_mode().permits(limit.mode)
            && run.permission_level().permits(limit.permission_level);
        let movable = !active_turn
            || (lowers_only
                && self
                    .provider_store
                    .adapter_supports_turn_scoped_execution_config(run.adapter_key()));
        Ok(Some(if movable {
            OffLimitRun::MovesNow(limit)
        } else {
            OffLimitRun::AtNextDispatch
        }))
    }

    /// What leased `agent_id` must run under on its worker, as its parent chain limits it now;
    /// `None` once it is no longer leased.
    pub(super) fn leased_agent_limit_now(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Option<owned::OwnedRemoteAgentConfigUpdate>, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let agents = self.agent_store.get_session_agents(session_id);
        Ok(agents
            .iter()
            .find(|agent| agent.id() == agent_id && !agent.archived())
            .and_then(|agent| {
                let binding = agent.remote_execution()?;
                let limited = crate::session::effective_agent_execution_config_in_tree(
                    &session, agent, &agents,
                );
                Some(leased_agent_limit(binding, limited))
            }))
    }

    /// Apply the next scope to the provider too: its process may hold session rules.
    pub(super) fn apply_agent_execution_config_to_provider(
        &self,
        session_id: &str,
        agent_id: &str,
        next_config: crate::session::EffectiveAgentExecutionConfig,
        working_directory_changed: bool,
        active_turn: bool,
    ) -> Result<Vec<String>, DaemonError> {
        let mut terminated_run_ids = Vec::new();
        if let Some(run) = self.provider_store.get_run_for_agent(session_id, agent_id) {
            match run.state() {
                crate::provider::ProviderRunState::Starting
                | crate::provider::ProviderRunState::Running
                | crate::provider::ProviderRunState::Parked => {
                    // A turn-scoped config update cannot revoke rules held by a process
                    // whose working directory just changed.
                    if !working_directory_changed
                        && self
                            .provider_store
                            .adapter_supports_turn_scoped_execution_config(run.adapter_key())
                    {
                        let updated = self.provider_store.update_run_execution_config(
                            run.id(),
                            next_config.mode,
                            next_config.permission_level,
                        )?;
                        self.provider_run_projection.update(updated);
                    } else if !active_turn {
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
        Ok(terminated_run_ids)
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

    pub(super) fn ensure_agent_config_not_provider_native_tui(
        &self,
        session_id: &str,
        agent_id: &str,
        operation: &'static str,
    ) -> Result<(), DaemonError> {
        let Some(run) = self.provider_store.get_run_for_agent(session_id, agent_id) else {
            return Ok(());
        };
        if run.client_interface().is_chariox() {
            return Ok(());
        }
        Err(DaemonError::LocalTransport {
            operation,
            message: format!(
                "agent `{agent_id}` is controlled by a provider-native TUI; change provider settings in that TUI"
            ),
        })
    }
}

/// How a local run off its limit moves.
pub(super) enum OffLimitRun {
    /// It takes the limit now without ending a turn: in place, or by ending its idle process
    /// so the next turn launches at it.
    MovesNow(crate::session::EffectiveAgentExecutionConfig),
    /// Its turn keeps running as it is; the next dispatch moves the run.
    AtNextDispatch,
}

/// What moving local runs to their limit did.
#[derive(Default)]
pub(super) struct LocalRunLimits {
    /// The agents whose run moved now or moves at its next dispatch.
    pub(super) relimited: Vec<String>,
    /// The runs that ended to relaunch at the limit.
    pub(super) terminated_run_ids: Vec<String>,
}

fn leased_agent_limit(
    binding: &crate::agent::RemoteAgentBinding,
    limited: crate::session::EffectiveAgentExecutionConfig,
) -> owned::OwnedRemoteAgentConfigUpdate {
    owned::OwnedRemoteAgentConfigUpdate {
        worker_kernel_id: binding.worker_kernel_id.clone(),
        leased_agent_id: binding.leased_agent_id.clone(),
        relay_url: binding.relay_url.clone(),
        relay_token: binding.relay_token.clone(),
        execution_mode: limited.mode,
        permission_level: limited.permission_level,
    }
}
