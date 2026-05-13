use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use base64::Engine;

use crate::agent::CreateAgentRequest;
use crate::agent::GitWorktreePlacement;
use crate::app::{provider_output, DaemonApp};
use crate::attachment::{AttachRequest, ClientCapabilityLevel};
use crate::error::DaemonError;
use crate::execution_lease::{
    ExecutionLease, LeasedAgent, LeasedWorkflowTurnBinding, RemoteWorkflowTurnContext,
};
use crate::history::SessionHistoryEntry;
use crate::provider::{LaunchProviderRequest, ProviderRunState, RuntimeProviderRun};
use crate::session::CreateSessionRequest;
use crate::terminal::TerminalOutputKind;
use crate::transport::relay_peer::{
    RelayPeerEvent, RelayProjectedCompletion, RelayProjectedOutputChunk, RelayPromptAttachment,
    RemoteGitObservation, RemoteGitTurnContext, RemoteManagedIoContext, RemoteMcpAvailability,
    RemoteMcpAvailabilityStatus, RemoteMcpCheckContext, RemoteNativeInteractionContext,
    RemoteSkillMaterialization, RemoteSkillSyncContext, RequiredRemoteMcp,
};

pub(crate) struct RemoteLeaseRuntime<'a> {
    app: &'a mut DaemonApp,
}

fn validate_worker_mcp_runtime(
    config: &crate::mcp::ArrobaMcpServerConfig,
) -> RemoteMcpAvailabilityStatus {
    match &config.transport {
        crate::mcp::ArrobaMcpTransportConfig::Stdio {
            command,
            env_vars,
            cwd,
            ..
        } => {
            let missing_env = env_vars
                .iter()
                .filter(|name| std::env::var_os(name.as_str()).is_none())
                .cloned()
                .collect::<Vec<_>>();
            if !missing_env.is_empty() {
                return RemoteMcpAvailabilityStatus::MissingEnv { names: missing_env };
            }
            if let Some(cwd) = cwd {
                if !cwd.exists() {
                    return RemoteMcpAvailabilityStatus::Invalid {
                        reason: format!("cwd `{}` does not exist on worker", cwd.display()),
                    };
                }
            }
            if !command_is_available(command, cwd.as_deref()) {
                return RemoteMcpAvailabilityStatus::MissingCommand {
                    command: command.clone(),
                };
            }
            RemoteMcpAvailabilityStatus::Available
        }
        crate::mcp::ArrobaMcpTransportConfig::StreamableHttp {
            bearer_token_env_var,
            env_http_headers,
            ..
        } => {
            let mut missing_env = Vec::new();
            if let Some(name) = bearer_token_env_var {
                if std::env::var_os(name).is_none() {
                    missing_env.push(name.clone());
                }
            }
            for name in env_http_headers.values() {
                if std::env::var_os(name).is_none() {
                    missing_env.push(name.clone());
                }
            }
            missing_env.sort();
            missing_env.dedup();
            if !missing_env.is_empty() {
                return RemoteMcpAvailabilityStatus::MissingEnv { names: missing_env };
            }
            RemoteMcpAvailabilityStatus::Available
        }
    }
}

fn provider_run_mcp_set_matches(
    run: &RuntimeProviderRun,
    required_mcps: &[RequiredRemoteMcp],
) -> Result<bool, DaemonError> {
    if run.state() == ProviderRunState::Ended {
        return Ok(false);
    }
    let mut current = run
        .mcp_servers()
        .iter()
        .map(|config| Ok((config.name.clone(), config.definition_hash()?)))
        .collect::<Result<Vec<_>, DaemonError>>()?;
    let mut required = required_mcps
        .iter()
        .map(|required| {
            (
                required.config.name.clone(),
                required.definition_hash.clone(),
            )
        })
        .collect::<Vec<_>>();
    current.sort();
    required.sort();
    Ok(current == required)
}

fn command_is_available(command: &str, cwd: Option<&std::path::Path>) -> bool {
    let command_path = std::path::PathBuf::from(command);
    if command_path.is_absolute() || command_path.components().count() > 1 {
        let candidate = if command_path.is_absolute() {
            command_path
        } else if let Some(cwd) = cwd {
            cwd.join(command_path)
        } else {
            command_path
        };
        return candidate.is_file();
    }
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path_var)
        .map(|directory| directory.join(&command_path))
        .any(|candidate| candidate.is_file())
}

fn format_remote_mcp_unavailable_message(
    leased_agent: &LeasedAgent,
    unavailable: &[RemoteMcpAvailability],
) -> String {
    let details = unavailable
        .iter()
        .map(|entry| {
            let status = match &entry.status {
                RemoteMcpAvailabilityStatus::Available => "available".to_string(),
                RemoteMcpAvailabilityStatus::Missing => "missing on worker".to_string(),
                RemoteMcpAvailabilityStatus::DefinitionMismatch { worker_hash } => {
                    format!("definition mismatch; worker has {worker_hash}")
                }
                RemoteMcpAvailabilityStatus::MissingCommand { command } => {
                    format!("missing command `{command}` on worker")
                }
                RemoteMcpAvailabilityStatus::MissingEnv { names } => {
                    format!(
                        "missing environment variable(s) on worker: {}",
                        names.join(", ")
                    )
                }
                RemoteMcpAvailabilityStatus::Invalid { reason } => reason.clone(),
            };
            format!(
                "- {} expected hash {}: {}",
                entry.name, entry.expected_hash, status
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "remote agent `{}` requires MCPs that are not available in worker Arroba. Install the matching MCP definition in the worker project or user registry, then retry.\n{}",
        leased_agent.id, details
    )
}

fn prepare_remote_git_worktree(
    placement: &GitWorktreePlacement,
    target_hint: Option<&str>,
) -> Result<String, DaemonError> {
    let base_directory = std::env::current_dir().map_err(|error| DaemonError::LocalTransport {
        operation: "resolve remote git worktree base",
        message: error.to_string(),
    })?;
    let repo_root = run_remote_git(&base_directory, &["rev-parse", "--show-toplevel"])?;
    let repo_root = PathBuf::from(repo_root.trim());
    if repo_root.as_os_str().is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "create remote git worktree",
            message: format!(
                "git did not report a repository root for `{}`",
                base_directory.display()
            ),
        });
    }

    let from_ref = placement.from_ref.as_deref().unwrap_or("HEAD");
    let target_directory = placement
        .target_directory
        .as_deref()
        .or(target_hint)
        .map(|target| {
            let path = PathBuf::from(target);
            if path.is_absolute() {
                path
            } else {
                base_directory.join(path)
            }
        })
        .unwrap_or_else(|| {
            let slug = slugify_git_branch(placement.branch.as_deref().unwrap_or(from_ref));
            let repo_name = repo_root
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("worktree");
            repo_root
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(format!("{repo_name}-{slug}"))
        });

    let target = target_directory.display().to_string();
    let args = if let Some(branch) = placement.branch.as_deref() {
        if remote_git_branch_exists(&repo_root, branch)? {
            vec![
                "worktree".to_string(),
                "add".to_string(),
                target.clone(),
                branch.to_string(),
            ]
        } else {
            vec![
                "worktree".to_string(),
                "add".to_string(),
                "-b".to_string(),
                branch.to_string(),
                target.clone(),
                from_ref.to_string(),
            ]
        }
    } else {
        vec![
            "worktree".to_string(),
            "add".to_string(),
            target.clone(),
            from_ref.to_string(),
        ]
    };
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    run_remote_git(&repo_root, &arg_refs)?;
    Ok(target)
}

fn remote_git_branch_exists(repo_root: &Path, branch: &str) -> Result<bool, DaemonError> {
    match run_remote_git(
        repo_root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    ) {
        Ok(_) => Ok(true),
        Err(error) => {
            if error.to_string().contains("git rev-parse") {
                Ok(false)
            } else {
                Err(error)
            }
        }
    }
}

fn run_remote_git(cwd: &Path, args: &[&str]) -> Result<String, DaemonError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "run remote git",
            message: format!(
                "git {} failed in `{}`: {error}",
                args.join(" "),
                cwd.display()
            ),
        })?;
    if !output.status.success() {
        return Err(DaemonError::LocalTransport {
            operation: "run remote git",
            message: format!(
                "git {} failed in `{}`: {}",
                args.join(" "),
                cwd.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn slugify_git_branch(value: &str) -> String {
    let slug = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if slug.is_empty() {
        "worktree".to_string()
    } else {
        slug
    }
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

    fn observe_leased_git_before(
        &mut self,
        leased_agent: &LeasedAgent,
        provider_run_id: &str,
        git_context: RemoteGitTurnContext,
    ) {
        let Some(lease) = self
            .app
            .execution_leases
            .get(&leased_agent.lease_id)
            .cloned()
        else {
            return;
        };
        let Ok(provider_run) = self.app.providers.get_run(provider_run_id) else {
            return;
        };
        let worktree_path = provider_run.working_directory().cloned().or_else(|| {
            self.app
                .sessions
                .get_session(&leased_agent.backing_session_id)
                .ok()
                .map(|session| PathBuf::from(session.worktree_id()))
        });
        let Some(worktree_path) = worktree_path else {
            return;
        };
        let context = crate::git_observer::GitTurnContext {
            session_id: git_context.home_session_id,
            agent_id: git_context.home_agent_id,
            provider: leased_agent.provider.clone(),
            model: leased_agent
                .model
                .clone()
                .unwrap_or_else(|| "default".to_string()),
            provider_run_id: provider_run_id.to_string(),
            provider_session_id: provider_run.provider_session_id().map(str::to_string),
            prompt_id: git_context.home_prompt_id,
            turn_id: git_context.home_turn_id,
            worktree_path,
            machine_id: Some(lease.machine_id),
            prompt_summary: git_context.prompt_summary,
        };
        if let Some(snapshot) = crate::git_observer::capture_turn_snapshot(context) {
            self.app.remote_git_turn_snapshots.insert(snapshot);
        }
    }

    pub(crate) fn observe_leased_git_after(
        &mut self,
        leased_agent_id: &str,
        provider_run_id: &str,
    ) -> Result<Vec<RemoteGitObservation>, DaemonError> {
        let _leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        let Some(before) = self
            .app
            .remote_git_turn_snapshots
            .remove_for_provider_run(provider_run_id)
        else {
            return Ok(Vec::new());
        };
        let candidates = self.app.remote_git_turn_snapshots.candidates_for(&before);
        let after_context = crate::git_observer::GitTurnContext {
            session_id: before.session_id.clone(),
            agent_id: before.agent_id.clone(),
            provider: before.provider.clone(),
            model: before.model.clone(),
            provider_run_id: before.provider_run_id.clone(),
            provider_session_id: before.provider_session_id.clone(),
            prompt_id: before.prompt_id.clone(),
            turn_id: before.turn_id.clone(),
            worktree_path: PathBuf::from(before.worktree_path.clone()),
            machine_id: before.machine_id.clone(),
            prompt_summary: before.prompt_summary.clone(),
        };
        let Some(after) = crate::git_observer::capture_turn_snapshot(after_context) else {
            return Ok(Vec::new());
        };
        Ok(crate::git_observer::observations_after_turn(
            before, after, candidates,
        ))
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

    pub(crate) fn ensure_remote_skill_packages(
        &mut self,
        context: RemoteSkillSyncContext,
        packages: Vec<crate::skill::ArrobaSkillPackage>,
    ) -> Result<Vec<RemoteSkillMaterialization>, DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get(&context.leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: context.leased_agent_id.clone(),
            })?;
        let lease = self
            .app
            .execution_leases
            .get(&leased_agent.lease_id)
            .cloned()
            .ok_or_else(|| DaemonError::ExecutionLeaseNotFound {
                lease_id: leased_agent.lease_id.clone(),
            })?;
        if lease.home_kernel_id != context.home_kernel_id
            || lease.home_session_id != context.home_session_id
            || lease.home_agent_id != context.home_agent_id
        {
            return Err(DaemonError::LocalTransport {
                operation: "ensure remote skill packages",
                message: "remote skill sync context does not match leased agent".to_string(),
            });
        }
        let session = self
            .app
            .sessions
            .get_session(&leased_agent.backing_session_id)?;
        let base_dir = std::path::PathBuf::from(session.worktree_id())
            .join(".arroba")
            .join("remote")
            .join("skills")
            .join(&context.home_kernel_id);
        packages
            .iter()
            .map(|package| {
                let materialized_root =
                    crate::skill::materialize_skill_package(&base_dir, package)?;
                Ok(RemoteSkillMaterialization {
                    name: package.metadata.name.clone(),
                    version_hash: package.version_hash.clone(),
                    materialized_root: materialized_root.to_string_lossy().to_string(),
                })
            })
            .collect()
    }

    pub(crate) fn check_remote_mcp_availability(
        &mut self,
        context: RemoteMcpCheckContext,
        required_mcps: Vec<RequiredRemoteMcp>,
    ) -> Result<Vec<RemoteMcpAvailability>, DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get(&context.leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: context.leased_agent_id.clone(),
            })?;
        self.validate_mcp_check_context(&leased_agent, &context)?;
        Ok(self.remote_mcp_availability_for_leased_agent(&leased_agent, &required_mcps))
    }

    fn ensure_required_remote_mcps_available(
        &mut self,
        leased_agent: &LeasedAgent,
        required_mcps: &[RequiredRemoteMcp],
    ) -> Result<(), DaemonError> {
        let unavailable = self
            .remote_mcp_availability_for_leased_agent(leased_agent, required_mcps)
            .into_iter()
            .filter(|result| !matches!(result.status, RemoteMcpAvailabilityStatus::Available))
            .collect::<Vec<_>>();
        if unavailable.is_empty() {
            return Ok(());
        }
        Err(DaemonError::LocalTransport {
            operation: "remote mcp availability",
            message: format_remote_mcp_unavailable_message(leased_agent, &unavailable),
        })
    }

    fn validate_mcp_check_context(
        &self,
        leased_agent: &LeasedAgent,
        context: &RemoteMcpCheckContext,
    ) -> Result<(), DaemonError> {
        let lease = self
            .app
            .execution_leases
            .get(&leased_agent.lease_id)
            .cloned()
            .ok_or_else(|| DaemonError::ExecutionLeaseNotFound {
                lease_id: leased_agent.lease_id.clone(),
            })?;
        if lease.home_kernel_id != context.home_kernel_id
            || lease.home_session_id != context.home_session_id
            || lease.home_agent_id != context.home_agent_id
        {
            return Err(DaemonError::LocalTransport {
                operation: "check remote MCP availability",
                message: "remote MCP check context does not match leased agent".to_string(),
            });
        }
        Ok(())
    }

    fn remote_mcp_availability_for_leased_agent(
        &self,
        leased_agent: &LeasedAgent,
        required_mcps: &[RequiredRemoteMcp],
    ) -> Vec<RemoteMcpAvailability> {
        let session = match self
            .app
            .sessions
            .get_session(&leased_agent.backing_session_id)
        {
            Ok(session) => session,
            Err(error) => {
                return required_mcps
                    .iter()
                    .map(|required| RemoteMcpAvailability {
                        name: required.config.name.clone(),
                        expected_hash: required.definition_hash.clone(),
                        status: RemoteMcpAvailabilityStatus::Invalid {
                            reason: error.to_string(),
                        },
                    })
                    .collect();
            }
        };
        let mut roots = vec![crate::mcp::ArrobaMcpRegistry::project_root(
            session.worktree_id(),
        )];
        if let Some(user_root) = crate::mcp::ArrobaMcpRegistry::user_root() {
            roots.push(user_root);
        }
        let registry = crate::mcp::ArrobaMcpRegistry::new(roots);
        required_mcps
            .iter()
            .map(|required| {
                let status = match registry.get(&required.config.name) {
                    Ok(Some(worker_config)) => match worker_config.definition_hash() {
                        Ok(worker_hash) if worker_hash == required.definition_hash => {
                            validate_worker_mcp_runtime(&worker_config)
                        }
                        Ok(worker_hash) => {
                            RemoteMcpAvailabilityStatus::DefinitionMismatch { worker_hash }
                        }
                        Err(error) => RemoteMcpAvailabilityStatus::Invalid {
                            reason: error.to_string(),
                        },
                    },
                    Ok(None) => RemoteMcpAvailabilityStatus::Missing,
                    Err(error) => RemoteMcpAvailabilityStatus::Invalid {
                        reason: error.to_string(),
                    },
                };
                RemoteMcpAvailability {
                    name: required.config.name.clone(),
                    expected_hash: required.definition_hash.clone(),
                    status,
                }
            })
            .collect()
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
    pub(crate) fn leased_agent_active_prompt_attachments(
        &self,
        leased_agent_id: &str,
    ) -> Result<Vec<crate::session::PromptAttachment>, DaemonError> {
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
            .prompt_owner_active_prompt_for_agent_snapshot(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )?
            .map(|prompt| prompt.attachments().to_vec())
            .unwrap_or_default())
    }

    fn materialize_leased_prompt_attachments(
        &self,
        leased_agent: &LeasedAgent,
        attachments: Vec<RelayPromptAttachment>,
    ) -> Result<Vec<crate::session::PromptAttachment>, DaemonError> {
        attachments
            .into_iter()
            .enumerate()
            .map(|(index, attachment)| {
                if let Some(contents_base64) = attachment.contents_base64 {
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(contents_base64)
                        .map_err(|error| DaemonError::LocalTransport {
                            operation: "decode remote prompt attachment",
                            message: error.to_string(),
                        })?;
                    let filename = attachment
                        .filename
                        .clone()
                        .unwrap_or_else(|| format!("attachment-{index}"));
                    let root = std::env::temp_dir()
                        .join("arroba-remote-prompt-attachments")
                        .join(&leased_agent.backing_session_id)
                        .join(&leased_agent.id);
                    fs::create_dir_all(&root).map_err(|error| DaemonError::LocalTransport {
                        operation: "create remote prompt attachment directory",
                        message: error.to_string(),
                    })?;
                    let path = root.join(format!(
                        "{}-{}-{}",
                        crate::session::unix_epoch_ms(),
                        index,
                        filename
                    ));
                    fs::write(&path, bytes).map_err(|error| DaemonError::LocalTransport {
                        operation: "write remote prompt attachment",
                        message: error.to_string(),
                    })?;
                    Ok(crate::session::PromptAttachment::new(
                        format!("file://{}", path.display()),
                        attachment.mime,
                        Some(filename),
                    ))
                } else {
                    Ok(crate::session::PromptAttachment::new(
                        attachment.url,
                        attachment.mime,
                        attachment.filename,
                    ))
                }
            })
            .collect()
    }

    pub(crate) fn drain_leased_runtime_projection(
        &mut self,
        leased_agent_id: &str,
        provider_run_id: &str,
        pump_output: bool,
    ) -> Result<Option<(String, RelayPeerEvent)>, DaemonError> {
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
        if pump_output {
            let _ = provider_output::pump_terminal_output_for_attachment(
                self.app,
                &leased_agent.backing_session_id,
                &leased_agent.backing_attachment_id,
            )?;
        }
        let output_chunks = self
            .app
            .terminal
            .drain_output_records(
                &leased_agent.backing_session_id,
                &leased_agent.backing_attachment_id,
            )
            .into_iter()
            .map(|record| RelayProjectedOutputChunk {
                kind: record.kind,
                merge_key: record.merge_key,
                bytes: record.bytes,
            })
            .collect::<Vec<_>>();
        let notices = self
            .app
            .terminal
            .drain_notice_records(
                &leased_agent.backing_session_id,
                &leased_agent.backing_attachment_id,
            )
            .into_iter()
            .map(|record| record.message)
            .collect::<Vec<_>>();
        let mut completions = self
            .app
            .terminal
            .drain_completion_records(
                &leased_agent.backing_session_id,
                &leased_agent.backing_attachment_id,
            )
            .into_iter()
            .map(|record| RelayProjectedCompletion {
                message_id: record.message_id,
                completed_at_ms: record.completed_at_ms,
            })
            .collect::<Vec<_>>();
        let backing_prompt_active = self
            .app
            .prompt_owner_active_prompt_for_agent(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )?
            .is_some();
        let completion_already_projected = leased_agent
            .projected_completion_provider_run_ids
            .iter()
            .any(|id| id == provider_run_id);
        if completions.is_empty() && !backing_prompt_active && !completion_already_projected {
            completions.push(RelayProjectedCompletion {
                message_id: format!("leased-{provider_run_id}-completion"),
                completed_at_ms: crate::session::unix_epoch_ms(),
            });
        }
        if !completions.is_empty() {
            if backing_prompt_active {
                let _ = self.app.complete_active_prompt(
                    &leased_agent.backing_session_id,
                    &leased_agent.backing_agent_id,
                    Some(provider_run_id),
                )?;
            }
            if let Some(agent) = self.app.leased_agents.get_mut(leased_agent_id) {
                if !agent
                    .projected_completion_provider_run_ids
                    .iter()
                    .any(|id| id == provider_run_id)
                {
                    agent
                        .projected_completion_provider_run_ids
                        .push(provider_run_id.to_string());
                }
            }
            self.app.leased_workflow_turns.remove(provider_run_id);
        }
        if output_chunks.is_empty() && notices.is_empty() && completions.is_empty() {
            return Ok(None);
        }
        Ok(Some((
            lease.home_kernel_id,
            RelayPeerEvent::LeasedRuntimeProjection {
                home_session_id: lease.home_session_id,
                home_agent_id: lease.home_agent_id,
                provider_run_id: provider_run_id.to_string(),
                output_chunks,
                notices,
                completions,
            },
        )))
    }

    pub(crate) fn pump_leased_runtime_projections(
        &mut self,
    ) -> Result<Vec<(String, RelayPeerEvent)>, DaemonError> {
        let leased_agents = self.app.leased_agents.values().cloned().collect::<Vec<_>>();
        let mut events = Vec::new();
        for leased_agent in leased_agents {
            let Some(provider_run_id) = self
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
                .map(|run| run.id().to_string())
            else {
                continue;
            };
            let _ = provider_output::ProviderOutputPump::new(self.app).pump_provider_output(
                provider_output::ProviderOutputPumpRequest {
                    session_id: &leased_agent.backing_session_id,
                    provider_run_id: &provider_run_id,
                    recipient_attachment_ids: vec![leased_agent.backing_attachment_id.clone()],
                    initial_liveness_already_checked: false,
                },
            )?;
            if let Some(event) =
                self.drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)?
            {
                events.push(event);
            }
        }
        Ok(events)
    }

    pub(crate) fn project_remote_runtime_projection(
        &mut self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
        output_chunks: Vec<RelayProjectedOutputChunk>,
        notices: Vec<String>,
        completions: Vec<RelayProjectedCompletion>,
    ) -> Result<(), DaemonError> {
        let _ = self.app.sessions.get_session(session_id)?;
        let recipient_attachment_ids = self.app.attachments.list_session_attachment_ids(session_id);
        let saw_completion = !completions.is_empty();
        for chunk in output_chunks {
            self.app.terminal.fan_out_output(
                session_id,
                provider_run_id,
                Some(agent_id),
                chunk.kind.clone(),
                chunk.merge_key.clone(),
                recipient_attachment_ids.clone(),
                &chunk.bytes,
            );
            if chunk.kind != TerminalOutputKind::PromptEcho {
                self.app.append_history_entry(
                    session_id,
                    SessionHistoryEntry::provider_output(
                        session_id,
                        provider_run_id,
                        Some(agent_id),
                        chunk.kind,
                        chunk.merge_key,
                        String::from_utf8_lossy(&chunk.bytes).into_owned(),
                    ),
                );
            }
        }
        for notice in notices {
            self.app.terminal.record_notice(
                session_id,
                Some(provider_run_id),
                Some(agent_id),
                recipient_attachment_ids.clone(),
                notice.clone(),
            );
            self.app.append_history_entry(
                session_id,
                SessionHistoryEntry::notice(
                    session_id,
                    Some(provider_run_id),
                    Some(agent_id),
                    notice,
                ),
            );
        }
        for completion in completions {
            self.app.terminal.record_assistant_message_completion(
                session_id,
                provider_run_id,
                Some(agent_id),
                recipient_attachment_ids.clone(),
                &completion.message_id,
                completion.completed_at_ms,
            );
        }
        if let Some(active_prompt) = self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, agent_id)?
        {
            let workflow_output_ready = active_prompt.workflow_run_id().is_some()
                && crate::app::workflow_runtime::workflow_prompt_has_completion_output_from_runtime(
                    self.app,
                    session_id,
                    &active_prompt,
                    Some(provider_run_id),
                );
            if !saw_completion && !workflow_output_ready {
                return Ok(());
            }
            if active_prompt.workflow_run_id().is_some() && !workflow_output_ready {
                if let (Some(workflow_run_id), Some(workflow_node_run_id)) = (
                    active_prompt.workflow_run_id(),
                    active_prompt.workflow_node_run_id(),
                ) {
                    let message =
                        "provider completed workflow turn without a validated workflow output";
                    let provider_diagnostic = self
                        .app
                        .providers()
                        .get_run(provider_run_id)
                        .ok()
                        .and_then(|run| run.terminal_diagnostic().map(str::to_string))
                        .filter(|message| !message.trim().is_empty());
                    let (failure_kind, failure_message, notice_message) = if let Some(diagnostic) =
                        provider_diagnostic
                    {
                        (
                            crate::session::WorkflowFailureKind::ProviderFailure,
                            diagnostic.clone(),
                            format!(
                                "Workflow run `{workflow_run_id}` failed after provider turn failure: {diagnostic}"
                            ),
                        )
                    } else {
                        (
                            crate::session::WorkflowFailureKind::MissingStructuredOutput,
                            message.to_string(),
                            format!(
                                "Workflow run `{workflow_run_id}` failed after provider turn completion without workflow output."
                            ),
                        )
                    };
                    let failure = crate::session::WorkflowFailureEvent::new(
                        failure_kind,
                        workflow_node_run_id,
                        Vec::new(),
                        failure_message,
                    );
                    let _ = self.app.sessions_mut().record_workflow_failure_event(
                        session_id,
                        workflow_run_id,
                        failure,
                    );
                    self.app.sessions_mut().fail_workflow_node_run(
                        session_id,
                        workflow_run_id,
                        workflow_node_run_id,
                    )?;
                    self.app.record_notice(
                        session_id,
                        Some(provider_run_id),
                        recipient_attachment_ids.clone(),
                        notice_message,
                    );
                    let _ = crate::app::KernelSessionReadService::new(self.app)
                        .session_snapshot(session_id);
                    let _ = self
                        .app
                        .prompt_owner_complete_active_prompt_only(session_id, agent_id)?;
                }
                return Ok(());
            }
            let remote_execution = self
                .app
                .agents
                .get_agent(agent_id)?
                .remote_execution()
                .cloned();
            let completed = self
                .app
                .prompt_owner_complete_active_prompt_only(session_id, agent_id)?;
            let _ =
                crate::app::KernelSessionReadService::new(self.app).session_snapshot(session_id);
            crate::app::workflow_runtime::complete_workflow_prompt_from_runtime(
                self.app,
                session_id,
                &completed,
                Some(provider_run_id),
            )?;
            if let Some(remote_execution) = remote_execution {
                if self
                    .app
                    .prompt_owner_active_prompt_for_agent(session_id, agent_id)?
                    .is_none()
                {
                    let started_next = self.app.advance_next_queued_prompt_remote(
                        session_id,
                        agent_id,
                        &remote_execution.worker_kernel_id,
                        &remote_execution.leased_agent_id,
                        remote_execution.relay_url.as_deref(),
                        remote_execution.relay_token.as_deref(),
                    )?;
                    if started_next.is_none() {
                        self.app.sync_focused_provider_run_if_idle(session_id)?;
                    }
                }
            }
        }
        Ok(())
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
