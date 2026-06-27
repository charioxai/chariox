use std::path::Path;

use crate::agent::CreateAgentRequest;
use crate::agent::GitWorktreePlacement;
use crate::app::DaemonApp;
use crate::attachment::{AttachRequest, ClientCapabilityLevel};
use crate::error::DaemonError;
use crate::execution_lease::{ExecutionLease, LeasedAgent};
use crate::provider::ProviderRunState;
use crate::session::CreateSessionRequest;

mod git_observation;
mod mcp_availability;
mod native_provider;
mod projection;
mod prompt_attachments;
mod prompt_lifecycle;
mod provider_run;
mod relay_context;
mod skill_sync;

pub(crate) use prompt_lifecycle::PreparedLeasedProviderRun;

pub(crate) struct RemoteLeaseRuntime<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> RemoteLeaseRuntime<'a> {
    pub(crate) fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn create_execution_lease(
        &mut self,
        home_kernel_id: &str,
        home_session_id: &str,
        home_agent_id: &str,
        home_agent_metaagent: bool,
        owner_user_id: &str,
    ) -> Result<ExecutionLease, DaemonError> {
        if !self.app.config.accept_remote_leases {
            return Err(DaemonError::RemoteLeasesDisabled {
                machine_id: self.app.config.host_machine_id.clone(),
            });
        }
        self.app.next_execution_lease_number = self.app.next_execution_lease_number.wrapping_add(1);
        let lease_id = format!(
            "lease-{:016x}",
            crate::session::unix_epoch_ms() ^ self.app.next_execution_lease_number.rotate_left(11)
        );
        let lease = ExecutionLease::new(
            lease_id.clone(),
            home_kernel_id.to_string(),
            home_session_id.to_string(),
            home_agent_id.to_string(),
            home_agent_metaagent,
            owner_user_id.to_string(),
            self.app.config.daemon_id.clone(),
            self.app.config.host_machine_id.clone(),
        );
        self.app.execution_leases.insert(lease_id, lease.clone());
        Ok(lease)
    }

    pub(crate) fn destroy_execution_lease(
        &mut self,
        lease_id: &str,
    ) -> Result<ExecutionLease, DaemonError> {
        self.app
            .leased_agents
            .retain(|_, agent| agent.lease_id != lease_id);
        self.app.execution_leases.remove(lease_id).ok_or_else(|| {
            DaemonError::ExecutionLeaseNotFound {
                lease_id: lease_id.to_string(),
            }
        })
    }

    pub(crate) fn create_leased_agent(
        &mut self,
        lease_id: &str,
        provider: &str,
        model: Option<String>,
        effort: Option<String>,
        execution_mode: Option<crate::provider::AgentExecutionMode>,
        permission_level: Option<crate::provider::AgentPermissionLevel>,
        workspace_live_sync_mode: Option<crate::config::WorkspaceLiveSyncMode>,
        worktree_id: Option<String>,
        worktree_placement: Option<GitWorktreePlacement>,
    ) -> Result<LeasedAgent, DaemonError> {
        let lease = self
            .app
            .execution_leases
            .get(lease_id)
            .cloned()
            .ok_or_else(|| DaemonError::ExecutionLeaseNotFound {
                lease_id: lease_id.to_string(),
            })?;
        let adapter_key = crate::provider::adapter_key_for_provider(provider);
        if self.app.providers.registry().resolve(adapter_key).is_none() {
            return Err(DaemonError::ProviderAdapterNotFound {
                adapter_key: adapter_key.to_string(),
            });
        }
        let worktree = if let Some(placement) = worktree_placement {
            let base_directory =
                std::env::current_dir().map_err(|error| DaemonError::LocalTransport {
                    operation: "resolve leased agent working directory",
                    message: error.to_string(),
                })?;
            crate::git_worktree_placement::prepare_git_worktree(
                &placement,
                base_directory,
                worktree_id.as_deref(),
                "create remote git worktree",
            )?
        } else {
            match worktree_id {
                Some(worktree) => worktree,
                None => std::env::current_dir()
                    .map_err(|error| DaemonError::LocalTransport {
                        operation: "resolve leased agent working directory",
                        message: error.to_string(),
                    })?
                    .display()
                    .to_string(),
            }
        };
        let worktree_path = Path::new(&worktree);
        if !worktree_path.exists() {
            return Err(DaemonError::LocalTransport {
                operation: "resolve leased agent working directory",
                message: format!("remote working directory `{worktree}` does not exist"),
            });
        }
        if !worktree_path.is_dir() {
            return Err(DaemonError::LocalTransport {
                operation: "resolve leased agent working directory",
                message: format!("remote working directory `{worktree}` is not a directory"),
            });
        }
        let workspace_id = format!("remote-lease:{}", lease.home_session_id);
        let existing_session = self
            .app
            .leased_agents
            .values()
            .filter(|agent| {
                self.app
                    .execution_leases
                    .get(&agent.lease_id)
                    .is_some_and(|existing_lease| {
                        existing_lease.home_session_id == lease.home_session_id
                    })
            })
            .filter_map(|agent| {
                self.app
                    .sessions
                    .get_session(&agent.backing_session_id)
                    .ok()
            })
            .find(|session| {
                session.workspace_id() == workspace_id
                    && session.worktree_id() == worktree
                    && session.owner_user_id() == lease.owner_user_id
            });
        let session = match existing_session {
            Some(session) => {
                if let Some(mode) = workspace_live_sync_mode {
                    if session.workspace_live_sync_mode() != Some(mode) {
                        self.app
                            .sessions
                            .write()
                            .set_workspace_live_sync_mode(session.id(), mode)?
                    } else {
                        session
                    }
                } else {
                    session
                }
            }
            None => {
                let mut request = CreateSessionRequest::new(workspace_id.clone(), worktree.clone())
                    .with_hidden(true)
                    .with_owner_user_id(lease.owner_user_id.clone());
                if let Some(mode) = workspace_live_sync_mode {
                    request = request.with_workspace_live_sync_mode(mode);
                }
                self.app.sessions.create_session(request)?
            }
        };
        let session_store = self.app.session_state_store();
        let attachment = {
            let mut sessions = session_store.write();
            self.app.attachments.attach(
                &mut sessions,
                AttachRequest::for_user(
                    session.id(),
                    format!("leased-agent:{}", lease.home_agent_id),
                    ClientCapabilityLevel::MessageTransport,
                    lease.owner_user_id.clone(),
                ),
            )?
        };
        let mut backing_agent = {
            let mut sessions = session_store.write();
            let mut request = CreateAgentRequest::new(session.id(), provider)
                .with_owner_user_id(lease.owner_user_id.clone())
                .with_worktree(session.worktree_id())
                .with_model(model.clone().unwrap_or_else(|| "default".to_string()))
                .with_effort(effort.clone().unwrap_or_else(|| "medium".to_string()));
            if let Some(execution_mode) = execution_mode {
                request = request.with_execution_mode_override(execution_mode);
            }
            if let Some(permission_level) = permission_level {
                request = request.with_permission_level_override(permission_level);
            }
            self.app.agents.create_agent(request, &mut sessions)?
        };
        if lease.home_agent_metaagent {
            backing_agent = self
                .app
                .agents_mut()
                .activate_agent_meta_mode(backing_agent.id(), None)?;
        }
        self.app.next_leased_agent_number = self.app.next_leased_agent_number.wrapping_add(1);
        let agent_id = format!(
            "leased-agent-{:016x}",
            crate::session::unix_epoch_ms() ^ self.app.next_leased_agent_number.rotate_left(13)
        );
        let agent = LeasedAgent::new(
            agent_id.clone(),
            lease_id.to_string(),
            lease.home_agent_id.clone(),
            provider.to_string(),
            model,
            effort,
            execution_mode,
            permission_level,
            session.id().to_string(),
            backing_agent.id().to_string(),
            attachment.id().to_string(),
        );
        self.app.leased_agents.insert(agent_id, agent.clone());
        Ok(agent)
    }

    pub(crate) fn destroy_leased_agent(
        &mut self,
        leased_agent_id: &str,
    ) -> Result<LeasedAgent, DaemonError> {
        let agent = self
            .app
            .leased_agents
            .remove(leased_agent_id)
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        self.app
            .leased_workflow_turns
            .retain(|_, binding| binding.leased_agent_id != leased_agent_id);
        let provider_runs = self
            .app
            .providers
            .list_runs()
            .into_iter()
            .filter(|run| {
                run.session_id() == agent.backing_session_id
                    && run.agent_instance_id() == Some(agent.backing_agent_id.as_str())
                    && run.state() != ProviderRunState::Ended
            })
            .collect::<Vec<_>>();
        for provider_run in provider_runs {
            let run_id = provider_run.id().to_string();
            let _ = crate::app::provider_runtime::ProviderProcessTracker::new(self.app)
                .remove_run(&run_id);
            if let Ok(outcome) = self
                .app
                .providers
                .terminate_run_provider_only(provider_run.session_id(), provider_run.id())
            {
                let _ = self
                    .app
                    .sessions
                    .set_active_provider_run(outcome.run().session_id(), None);
                self.app.update_provider_run_projection(outcome.into_run());
            }
        }
        let backing_session_still_used = self
            .app
            .leased_agents
            .values()
            .any(|candidate| candidate.backing_session_id == agent.backing_session_id);
        let session_store = self.app.session_state_store();
        let _ = {
            let mut sessions = session_store.write();
            self.app
                .attachments
                .detach(&mut sessions, &agent.backing_attachment_id)
        };
        let _ = {
            let mut sessions = session_store.write();
            self.app
                .agents
                .destroy_agent(&agent.backing_agent_id, &mut sessions)
        };
        if !backing_session_still_used {
            let _ = self.app.sessions.end_session(&agent.backing_session_id);
            let _ = self.app.sessions.delete_session(&agent.backing_session_id);
        }
        Ok(agent)
    }

    pub(crate) fn update_leased_agent_config(
        &mut self,
        leased_agent_id: &str,
        execution_mode: crate::provider::AgentExecutionMode,
        permission_level: crate::provider::AgentPermissionLevel,
    ) -> Result<LeasedAgent, DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        let backing_agent = self.app.agents.get_agent(&leased_agent.backing_agent_id)?;
        if self
            .app
            .prompt_owner_active_prompt_for_agent(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )?
            .is_some()
            || backing_agent.is_processing()
        {
            return Err(DaemonError::LocalTransport {
                operation: "update leased agent config",
                message: format!(
                    "leased agent `{leased_agent_id}` has an active turn; update the config after it finishes"
                ),
            });
        }

        let config_changed = leased_agent.execution_mode != Some(execution_mode)
            || leased_agent.permission_level != Some(permission_level);
        if config_changed {
            if let Some(run) = self.app.providers.get_run_for_agent(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            ) {
                match run.state() {
                    ProviderRunState::Starting
                    | ProviderRunState::Running
                    | ProviderRunState::Parked => {
                        let run_id = run.id().to_string();
                        let _ = crate::app::provider_runtime::ProviderProcessTracker::new(self.app)
                            .remove_run(&run_id);
                        if let Ok(outcome) = self
                            .app
                            .providers
                            .terminate_run_provider_only(run.session_id(), run.id())
                        {
                            let _ = self
                                .app
                                .sessions
                                .set_active_provider_run(outcome.run().session_id(), None);
                            self.app.update_provider_run_projection(outcome.into_run());
                        }
                    }
                    ProviderRunState::Ended => {
                        self.app.providers.clear_runtime(run.id());
                    }
                }
            }
        }

        let _ = self.app.agents.update_agent_config(
            &leased_agent.backing_agent_id,
            Some(Some(execution_mode)),
            Some(Some(permission_level)),
            None,
            None,
        )?;
        let updated = self
            .app
            .leased_agents
            .get_mut(leased_agent_id)
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        updated.execution_mode = Some(execution_mode);
        updated.permission_level = Some(permission_level);
        Ok(updated.clone())
    }

    pub(crate) fn update_leased_agent_meta_mode(
        &mut self,
        leased_agent_id: &str,
        active: bool,
    ) -> Result<LeasedAgent, DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        let backing_agent = self.app.agents.get_agent(&leased_agent.backing_agent_id)?;
        if self
            .app
            .prompt_owner_active_prompt_for_agent(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )?
            .is_some()
            || backing_agent.is_processing()
        {
            return Err(DaemonError::LocalTransport {
                operation: "update leased agent meta mode",
                message: format!(
                    "leased agent `{leased_agent_id}` has an active turn; update meta mode after it finishes"
                ),
            });
        }

        let changed = backing_agent.is_metaagent() != active;
        if changed {
            if active {
                self.app
                    .agents_mut()
                    .activate_agent_meta_mode(&leased_agent.backing_agent_id, None)?;
            } else {
                self.app
                    .agents_mut()
                    .deactivate_agent_meta_mode(&leased_agent.backing_agent_id)?;
            }
            self.terminate_backing_provider_runtime(&leased_agent);
        }

        Ok(leased_agent)
    }

    pub(crate) fn update_leased_agent_remote_extension_manifest(
        &mut self,
        leased_agent_id: &str,
        remote_extension_manifest: crate::extension::RemoteExtensionManifest,
    ) -> Result<(), DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        if let Some(run) = self.app.providers.get_run_for_agent(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
        ) {
            let updated = self
                .app
                .providers
                .update_run_remote_extension_manifest(run.id(), remote_extension_manifest)?;
            self.app.update_provider_run_projection(updated);
        }
        Ok(())
    }

    fn terminate_backing_provider_runtime(&mut self, leased_agent: &LeasedAgent) {
        let Some(run) = self.app.providers.get_run_for_agent(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
        ) else {
            return;
        };
        match run.state() {
            ProviderRunState::Starting | ProviderRunState::Running | ProviderRunState::Parked => {
                let run_id = run.id().to_string();
                let _ = crate::app::provider_runtime::ProviderProcessTracker::new(self.app)
                    .remove_run(&run_id);
                if let Ok(outcome) = self
                    .app
                    .providers
                    .terminate_run_provider_only(run.session_id(), run.id())
                {
                    let _ = self
                        .app
                        .sessions
                        .set_active_provider_run(outcome.run().session_id(), None);
                    self.app.update_provider_run_projection(outcome.into_run());
                }
            }
            ProviderRunState::Ended => {
                self.app.providers.clear_runtime(run.id());
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn execution_lease_count(&self) -> usize {
        self.app.execution_leases.len()
    }

    #[cfg(test)]
    pub(crate) fn leased_agent_count(&self) -> usize {
        self.app.leased_agents.len()
    }

    #[cfg(test)]
    pub(crate) fn push_projected_output_history_key_for_test(
        &mut self,
        leased_agent_id: &str,
        key: String,
    ) {
        if let Some(agent) = self.app.leased_agents.get_mut(leased_agent_id) {
            agent.projected_output_history_keys.push(key);
        }
    }
}
