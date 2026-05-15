use std::path::{Path, PathBuf};

use crate::agent::CreateAgentRequest;
use crate::agent::GitWorktreePlacement;
use crate::app::DaemonApp;
use crate::attachment::{AttachRequest, ClientCapabilityLevel};
use crate::error::DaemonError;
use crate::execution_lease::{
    ExecutionLease, LeasedAgent, LeasedWorkflowTurnBinding, RemoteWorkflowTurnContext,
};
use crate::provider::{
    LaunchProviderRequest, ProviderClientInterface, ProviderResumeState, ProviderRunState,
    RuntimeProviderRun,
};
use crate::session::CreateSessionRequest;
use crate::transport::relay_peer::{
    RelayPromptAttachment, RemoteGitTurnContext, RemoteManagedIoContext,
    RemoteNativeInteractionContext, RequiredRemoteMcp,
};

mod git_observation;
mod git_worktree;
mod mcp_availability;
mod projection;
mod prompt_attachments;
mod skill_sync;

use git_worktree::prepare_remote_git_worktree;
use mcp_availability::provider_run_mcp_set_matches;

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
        if self.app.providers.registry().resolve(provider).is_none() {
            return Err(DaemonError::ProviderAdapterNotFound {
                adapter_key: provider.to_string(),
            });
        }
        let worktree = if let Some(placement) = worktree_placement {
            prepare_remote_git_worktree(&placement, worktree_id.as_deref())?
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
            Some(session) => session,
            None => self.app.sessions.create_session(
                CreateSessionRequest::new(workspace_id.clone(), worktree.clone())
                    .with_hidden(true)
                    .with_owner_user_id(lease.owner_user_id.clone()),
            )?,
        };
        let session_store = self.app.session_state_store();
        let attachment = {
            let mut sessions = session_store.write();
            self.app.attachments.attach(
                &mut sessions,
                AttachRequest::new(
                    session.id(),
                    format!("leased-agent:{}", lease.home_agent_id),
                    ClientCapabilityLevel::MessageTransport,
                ),
            )?
        };
        let backing_agent = {
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
        let _ = self.app.sessions.end_session(&agent.backing_session_id);
        let _ = self.app.sessions.delete_session(&agent.backing_session_id);
        self.app
            .history_projection
            .remove(&agent.backing_session_id);
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

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn launch_leased_native_provider_run(
        &mut self,
        leased_agent_id: &str,
        adapter_key: &str,
        provider: &str,
        account_profile: &str,
        model: &str,
        variant: Option<String>,
        structured_endpoint: Option<String>,
        provider_session_id: Option<String>,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        let lease = self
            .app
            .execution_leases
            .get(&leased_agent.lease_id)
            .cloned()
            .ok_or_else(|| DaemonError::ExecutionLeaseNotFound {
                lease_id: leased_agent.lease_id.clone(),
            })?;
        let backing_session = self
            .app
            .sessions
            .get_session(&leased_agent.backing_session_id)?;
        let mut request = LaunchProviderRequest::new(
            leased_agent.backing_session_id.clone(),
            adapter_key,
            provider,
            account_profile,
            model,
        )
        .with_agent_id(leased_agent.backing_agent_id.clone())
        .with_owner_user_id(lease.owner_user_id)
        .with_working_directory(PathBuf::from(backing_session.worktree_id()))
        .with_client_interface(ProviderClientInterface::NativeTui)
        .with_variant(variant);
        if let Some(execution_mode) = leased_agent.execution_mode {
            request = request.with_execution_mode(execution_mode);
        }
        if let Some(permission_level) = leased_agent.permission_level {
            request = request.with_permission_level(permission_level);
        }
        if let Some(endpoint) = structured_endpoint {
            request = request.with_structured_endpoint(endpoint);
        }
        if let Some(provider_session_id) = provider_session_id {
            request = match adapter_key {
                "codex" => request.with_resume_state(ProviderResumeState::from_codex_thread_id(
                    provider_session_id,
                )),
                "opencode" => request.with_resume_state(
                    ProviderResumeState::from_opencode_session_id(provider_session_id),
                ),
                "claude" => request.with_resume_state(ProviderResumeState::from_claude_session_id(
                    provider_session_id,
                )),
                _ => request,
            };
        }
        self.app.launch_provider(request)
    }

    pub(crate) fn native_interaction_context_for_backing_agent(
        &mut self,
        backing_session_id: &str,
        backing_agent_id: &str,
        worker_provider_run_id: &str,
    ) -> Option<(String, RemoteNativeInteractionContext)> {
        let leased_agent = self
            .app
            .leased_agents
            .values()
            .find(|agent| {
                agent.backing_session_id == backing_session_id
                    && agent.backing_agent_id == backing_agent_id
            })?
            .clone();
        let lease = self
            .app
            .execution_leases
            .get(&leased_agent.lease_id)?
            .clone();
        Some((
            lease.home_kernel_id,
            RemoteNativeInteractionContext {
                home_session_id: lease.home_session_id,
                home_agent_id: lease.home_agent_id,
                leased_agent_id: leased_agent.id,
                worker_provider_run_id: worker_provider_run_id.to_string(),
            },
        ))
    }

    #[cfg(test)]
    pub(crate) fn submit_leased_prompt(
        &mut self,
        leased_agent_id: &str,
        prompt: &str,
        attachments: Vec<RelayPromptAttachment>,
    ) -> Result<(String, crate::session::PromptSubmissionOutcome), DaemonError> {
        self.submit_leased_prompt_with_workflow_context(
            leased_agent_id,
            prompt,
            attachments,
            None,
            None,
            Vec::new(),
        )
    }

    pub(crate) fn submit_leased_prompt_with_workflow_context(
        &mut self,
        leased_agent_id: &str,
        prompt: &str,
        attachments: Vec<RelayPromptAttachment>,
        workflow_context: Option<RemoteWorkflowTurnContext>,
        git_context: Option<RemoteGitTurnContext>,
        required_mcps: Vec<RequiredRemoteMcp>,
    ) -> Result<(String, crate::session::PromptSubmissionOutcome), DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        let materialized_attachments =
            self.materialize_leased_prompt_attachments(&leased_agent, attachments)?;
        self.ensure_required_remote_mcps_available(&leased_agent, &required_mcps)?;
        let provider_run_id =
            self.ensure_leased_provider_run_matches_mcps(&leased_agent, &required_mcps)?;
        if let Some(git_context) = git_context {
            if self
                .app
                .prompt_owner_active_prompt_for_agent(
                    &leased_agent.backing_session_id,
                    &leased_agent.backing_agent_id,
                )?
                .is_none()
            {
                self.observe_leased_git_before(&leased_agent, &provider_run_id, git_context);
            }
        }
        let outcome = self.app.submit_prompt(
            &leased_agent.backing_session_id,
            &leased_agent.backing_attachment_id,
            Some(&leased_agent.backing_agent_id),
            prompt,
            materialized_attachments,
        )?;
        if let Some(context) = workflow_context {
            self.app.leased_workflow_turns.insert(
                provider_run_id.clone(),
                LeasedWorkflowTurnBinding {
                    leased_agent_id: leased_agent_id.to_string(),
                    provider_run_id: provider_run_id.clone(),
                    context,
                },
            );
        }
        Ok((provider_run_id, outcome))
    }

    fn ensure_leased_provider_run_matches_mcps(
        &mut self,
        leased_agent: &LeasedAgent,
        required_mcps: &[RequiredRemoteMcp],
    ) -> Result<String, DaemonError> {
        let lease = self
            .app
            .execution_leases
            .get(&leased_agent.lease_id)
            .cloned()
            .ok_or_else(|| DaemonError::ExecutionLeaseNotFound {
                lease_id: leased_agent.lease_id.clone(),
            })?;
        let existing = self.app.providers.get_run_for_agent(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
        );
        if let Some(run) = existing.as_ref() {
            if provider_run_mcp_set_matches(run, required_mcps)? {
                return Ok(run.id().to_string());
            }
            if self
                .app
                .prompt_owner_active_prompt_for_agent(
                    &leased_agent.backing_session_id,
                    &leased_agent.backing_agent_id,
                )?
                .is_some()
            {
                return Err(DaemonError::LocalTransport {
                    operation: "remote MCP provider reload",
                    message: format!(
                        "remote worker provider run `{}` does not have the required MCP set and is currently busy; retry after the active turn completes",
                        run.id()
                    ),
                });
            }
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

        let mut request = LaunchProviderRequest::new(
            &leased_agent.backing_session_id,
            &leased_agent.provider,
            &leased_agent.provider,
            "default",
            leased_agent
                .model
                .clone()
                .unwrap_or_else(|| "default".to_string()),
        )
        .with_agent_id(&leased_agent.backing_agent_id)
        .with_owner_user_id(lease.owner_user_id)
        .with_working_directory(std::path::PathBuf::from(
            self.app
                .sessions
                .get_session(&leased_agent.backing_session_id)?
                .worktree_id(),
        ))
        .with_mcp_servers(
            required_mcps
                .iter()
                .map(|required| required.config.clone())
                .collect(),
        );
        if let Some(execution_mode) = leased_agent.execution_mode {
            request = request.with_execution_mode(execution_mode);
        }
        if let Some(permission_level) = leased_agent.permission_level {
            request = request.with_permission_level(permission_level);
        }
        if leased_agent.effort.is_some() {
            request = request.with_variant(leased_agent.effort.clone());
        }
        if let Some(run) = existing.as_ref() {
            request = request.with_resume_state(run.resume_state().clone());
            if request.variant.is_none() {
                request = request.with_variant(run.variant().map(str::to_string));
            }
        }
        if crate::provider::provider_requires_managed_io_by_default(
            &leased_agent.provider,
            self.app.config(),
        ) {
            request = request.with_managed_io_required();
        }
        let run = self.app.launch_provider(request)?;
        Ok(run.id().to_string())
    }

    pub(crate) fn complete_leased_prompt(
        &mut self,
        leased_agent_id: &str,
    ) -> Result<crate::session::PromptCompletion, DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        let provider_run_id = self
            .app
            .providers
            .get_run_for_agent(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )
            .map(|run| run.id().to_string());
        let completion = self.app.complete_active_prompt(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
            provider_run_id.as_deref(),
        )?;
        if let Some(provider_run_id) = provider_run_id {
            self.app.leased_workflow_turns.remove(&provider_run_id);
        }
        Ok(completion)
    }

    pub(crate) fn cancel_leased_prompt(
        &mut self,
        leased_agent_id: &str,
    ) -> Result<crate::session::PromptCancellation, DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        let cancellation = self.app.cancel_active_prompt_internal(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
            None,
        )?;
        self.app
            .leased_workflow_turns
            .retain(|_, binding| binding.leased_agent_id != leased_agent_id);
        Ok(cancellation)
    }

    pub(crate) fn leased_agent_provider_run_id(
        &self,
        leased_agent_id: &str,
    ) -> Result<Option<String>, DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        Ok(self
            .app
            .providers
            .get_run_for_agent(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )
            .or_else(|| {
                self.app.providers.get_latest_run_for_agent(
                    &leased_agent.backing_session_id,
                    &leased_agent.backing_agent_id,
                )
            })
            .map(|run| run.id().to_string()))
    }

    pub(crate) fn leased_workflow_turn_context_for_provider_run(
        &self,
        provider_run_id: &str,
    ) -> Option<RemoteWorkflowTurnContext> {
        self.app
            .leased_workflow_turns
            .get(provider_run_id)
            .map(|binding| binding.context.clone())
    }

    pub(crate) fn leased_managed_io_context_for_provider_run(
        &self,
        provider_run_id: &str,
        worker_workspace_identity: crate::io::WorkspaceIdentity,
    ) -> Option<RemoteManagedIoContext> {
        let leased_agent = self.app.leased_agents.values().find(|leased_agent| {
            self.app
                .providers
                .get_run_for_agent(
                    &leased_agent.backing_session_id,
                    &leased_agent.backing_agent_id,
                )
                .map(|run| run.id() == provider_run_id)
                .unwrap_or(false)
        })?;
        let lease = self.app.execution_leases.get(&leased_agent.lease_id)?;
        Some(RemoteManagedIoContext {
            home_kernel_id: lease.home_kernel_id.clone(),
            home_session_id: lease.home_session_id.clone(),
            home_agent_id: lease.home_agent_id.clone(),
            leased_agent_id: leased_agent.id.clone(),
            worker_provider_run_id: provider_run_id.to_string(),
            worker_workspace_identity,
        })
    }

    pub(crate) fn complete_leased_workflow_prompt_for_provider_run(
        &mut self,
        provider_run_id: &str,
    ) -> Result<Option<crate::session::PromptCompletion>, DaemonError> {
        let Some(binding) = self.app.leased_workflow_turns.get(provider_run_id).cloned() else {
            return Ok(None);
        };
        let leased_agent = self
            .app
            .leased_agents
            .get(&binding.leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: binding.leased_agent_id.clone(),
            })?;
        if self
            .app
            .prompt_owner_active_prompt_for_agent(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )?
            .is_none()
        {
            self.app.leased_workflow_turns.remove(provider_run_id);
            return Ok(None);
        }
        let completion = self.app.complete_active_prompt(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
            Some(provider_run_id),
        )?;
        self.app.leased_workflow_turns.remove(provider_run_id);
        Ok(Some(completion))
    }

    #[cfg(test)]
    pub(crate) fn execution_lease_count(&self) -> usize {
        self.app.execution_leases.len()
    }

    #[cfg(test)]
    pub(crate) fn leased_agent_count(&self) -> usize {
        self.app.leased_agents.len()
    }
}
