use std::collections::BTreeSet;
use std::path::PathBuf;

use rand::distributions::{Alphanumeric, DistString};

use crate::agent::AgentInstance;
use crate::app::{DaemonApp, TrackedProviderProcess};
use crate::error::DaemonError;
use crate::provider::{
    AgentEndpointMode, LaunchProviderRequest, ProviderProcessInfo, ProviderProcessService,
    ProviderResumeState, ProviderRunState, ProviderRuntimeBinding, RuntimeMcpBinding,
    RuntimeProviderRun,
};

#[derive(Debug, Clone)]
pub(crate) struct StartedProviderLaunch {
    pub(crate) run: RuntimeProviderRun,
    previous_active_run_id: Option<String>,
}

impl DaemonApp {
    pub(crate) fn project_session_runtime_view(
        &self,
        session: &mut crate::session::RuntimeSession,
    ) {
        if let Some(active_provider_run_id) = session.active_provider_run_id() {
            if let Ok(active_run) = self.providers.get_run(active_provider_run_id) {
                let active_run_agent_id = active_run.agent_instance_id();
                let active_prompt_is_running = active_run_agent_id
                    .and_then(|agent_id| session.active_prompt_for_agent(agent_id))
                    .is_some();
                if active_run.state() == ProviderRunState::Running && active_prompt_is_running {
                    return;
                }
            }
        }

        let projected_run_id = session.focused_agent_id().and_then(|agent_id| {
            self.providers
                .get_run_for_agent(session.id(), agent_id)
                .map(|run| run.id().to_string())
        });
        session.set_active_provider_run(projected_run_id);
    }

    pub(crate) fn project_active_provider_run_for_agent(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        let projected_run_id = self
            .providers
            .get_run_for_agent(session_id, agent_id)
            .map(|run| run.id().to_string());
        let _ = self
            .sessions
            .set_active_provider_run(session_id, projected_run_id)?;
        Ok(())
    }

    fn register_managed_provider_process(
        &mut self,
        run: &RuntimeProviderRun,
    ) -> Result<(), DaemonError> {
        let process_key = self.pty.process_key(run.id())?;
        let pid = self.pty.process_id(run.id())?;
        let process_id = format!("managed:{}:{}", run.provider(), process_key);
        let entry = self
            .tracked_provider_processes
            .entry(process_key.clone())
            .or_insert_with(|| TrackedProviderProcess {
                process_id: process_id.clone(),
                provider: run.provider().to_string(),
                pid,
                endpoint_mode: run.endpoint_mode(),
                process_label: run.process_label().to_string(),
                started_at_ms: run.started_at_ms(),
                owner_provider_run_ids: Vec::new(),
            });
        entry.pid = pid.or(entry.pid);
        if !entry.owner_provider_run_ids.iter().any(|id| id == run.id()) {
            entry.owner_provider_run_ids.push(run.id().to_string());
        }
        self.tracked_provider_run_processes
            .insert(run.id().to_string(), process_key);
        Ok(())
    }

    pub(crate) fn remove_tracked_provider_process_for_run(
        &mut self,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        let process_key = self
            .tracked_provider_run_processes
            .get(provider_run_id)
            .cloned()
            .or_else(|| self.pty.process_key(provider_run_id).ok());
        let removed = self.pty.remove_process(provider_run_id)?;
        let Some(process_key) = process_key else {
            return Ok(removed);
        };
        self.tracked_provider_run_processes.remove(provider_run_id);
        let should_remove_entry =
            if let Some(entry) = self.tracked_provider_processes.get_mut(&process_key) {
                entry
                    .owner_provider_run_ids
                    .retain(|id| id != provider_run_id);
                entry.owner_provider_run_ids.is_empty()
            } else {
                false
            };
        if should_remove_entry {
            self.tracked_provider_processes.remove(&process_key);
        }
        Ok(removed)
    }

    pub(crate) fn start_provider_launch(
        &mut self,
        mut request: LaunchProviderRequest,
    ) -> Result<StartedProviderLaunch, DaemonError> {
        if request.agent_id.is_none() {
            request.agent_id = self
                .sessions
                .get_session(&request.session_id)?
                .focused_agent_id()
                .map(str::to_string);
        }
        if request.resume_state.is_none() {
            if let Some(agent_id) = request.agent_id.as_deref() {
                if let Ok(agent) = self.agents.get_agent(agent_id) {
                    let resume_state = sanitize_resume_state_for_launch(&request, &agent);
                    if !resume_state.is_empty() {
                        request = request.with_resume_state(resume_state);
                    }
                }
            }
        }
        crate::logging::info_with_fields(
            "daemon.app",
            "launching provider run",
            serde_json::json!({
                "adapter_key": request.adapter_key.clone(),
                "agent_id": request.agent_id.clone(),
                "provider": request.provider.clone(),
                "session_id": request.session_id.clone(),
            }),
        );
        if (request.adapter_key == "opencode" || request.adapter_key == "codex")
            && request.working_directory.is_none()
        {
            let agent_worktree = request.agent_id.as_deref().and_then(|agent_id| {
                self.agents
                    .get_agent(agent_id)
                    .ok()
                    .and_then(|agent| agent.worktree_id().map(PathBuf::from))
            });
            request.working_directory = Some(agent_worktree.unwrap_or_else(|| {
                PathBuf::from(
                    self.sessions
                        .get_session(&request.session_id)
                        .map(|session| session.worktree_id().to_string())
                        .unwrap_or_default(),
                )
            }));
        }
        if request.runtime_mcp_binding.is_none() {
            let shared_auth_token = self
                .providers
                .get_session_run_for_provider(&request.session_id, &request.provider)
                .and_then(|run| run.runtime_mcp_auth_token().map(str::to_string));
            request = request.with_runtime_mcp_binding(RuntimeMcpBinding::new(
                self.config.runtime_mcp_url(),
                shared_auth_token.unwrap_or_else(generate_runtime_mcp_auth_token),
            ));
        }
        let previous_active_run_id = self
            .sessions
            .get_session(&request.session_id)?
            .active_provider_run_id()
            .map(str::to_owned);
        let recipients = self
            .attachments
            .list_session_attachment_ids(&request.session_id);
        let run = self.providers.launch_run(&mut self.sessions, request)?;
        crate::logging::info_with_fields(
            "daemon.app",
            "prepared provider run endpoint metadata",
            serde_json::json!({
                "provider_run_id": run.id(),
                "endpoint_mode": run.endpoint_mode().to_string(),
                "session_id": run.session_id(),
                "provider": run.provider(),
            }),
        );
        if run.endpoint_mode() == AgentEndpointMode::Managed {
            if let Err(error) = self.pty.spawn_for_run(&run) {
                crate::logging::error_with_fields(
                    "daemon.app",
                    "PTY spawn failed for provider run",
                    serde_json::json!({
                        "provider_run_id": run.id(),
                        "session_id": run.session_id(),
                        "error": error.to_string(),
                    }),
                );
                let _ =
                    self.providers
                        .terminate_run(&mut self.sessions, run.session_id(), run.id());
                if let Some(previous_active_run_id) = previous_active_run_id.as_deref() {
                    match self.providers.resume_run(
                        &mut self.sessions,
                        run.session_id(),
                        previous_active_run_id,
                    ) {
                        Ok(resumed_run) => {
                            self.record_notice(
                                run.session_id(),
                                Some(resumed_run.id()),
                                recipients,
                                format!(
                                    "Provider switch failed for session `{}`. Arroba resumed the previous provider run `{}` automatically.",
                                    run.session_id(),
                                    resumed_run.id()
                                ),
                            );
                        }
                        Err(resume_error) => {
                            self.record_notice(
                                run.session_id(),
                                None,
                                recipients,
                                format!(
                                    "Provider switch failed for session `{}` and Arroba could not resume the previous provider run: {}",
                                    run.session_id(),
                                    resume_error
                                ),
                            );
                        }
                    }
                }
                return Err(error);
            }
            self.register_managed_provider_process(&run)?;
        }
        Ok(StartedProviderLaunch {
            run,
            previous_active_run_id,
        })
    }

    pub(crate) fn initialize_provider_runtime_binding(
        run: &RuntimeProviderRun,
    ) -> Result<Option<ProviderRuntimeBinding>, DaemonError> {
        ProviderProcessService::initialize_runtime_binding(run)
    }

    pub(crate) fn finish_provider_launch(
        &mut self,
        started: &StartedProviderLaunch,
        binding: Option<ProviderRuntimeBinding>,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        if let Some(binding) = binding {
            self.providers
                .apply_runtime_binding(started.run.id(), binding)?;
        }
        self.finish_provider_launch_success(&started.run)
    }

    pub(crate) fn fail_provider_launch(
        &mut self,
        started: &StartedProviderLaunch,
        error: &DaemonError,
    ) {
        crate::logging::error_with_fields(
            "daemon.app",
            "provider runtime initialization failed",
            serde_json::json!({
                "provider_run_id": started.run.id(),
                "session_id": started.run.session_id(),
                "error": error.to_string(),
            }),
        );
        let _ = self.remove_tracked_provider_process_for_run(started.run.id());
        self.providers.clear_runtime(started.run.id());
        let _ = self.providers.terminate_run(
            &mut self.sessions,
            started.run.session_id(),
            started.run.id(),
        );
        if let Some(previous_active_run_id) = started.previous_active_run_id.as_deref() {
            let _ = self.providers.resume_run(
                &mut self.sessions,
                started.run.session_id(),
                previous_active_run_id,
            );
        }
    }

    fn finish_provider_launch_success(
        &mut self,
        run: &RuntimeProviderRun,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        crate::logging::info_with_fields(
            "daemon.app",
            "initializing provider runtime",
            serde_json::json!({
                "provider_run_id": run.id(),
                "session_id": run.session_id(),
            }),
        );
        crate::logging::info_with_fields(
            "daemon.app",
            "provider runtime initialized successfully",
            serde_json::json!({
                "provider_run_id": run.id(),
                "session_id": run.session_id(),
            }),
        );
        let run = self.providers.get_run(run.id())?;
        let _ = self.providers.record_run_activity(run.id());
        if let Some(agent_id) = run.agent_instance_id() {
            let _ = self.agents.set_agent_runtime_profile(
                agent_id,
                run.provider(),
                Some(run.model().to_string()),
                run.variant().map(str::to_string),
                run.resume_state().clone(),
            )?;
        }
        Ok(run)
    }

    pub fn launch_provider(
        &mut self,
        request: LaunchProviderRequest,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let started = self.start_provider_launch(request)?;
        let binding = match Self::initialize_provider_runtime_binding(&started.run) {
            Ok(binding) => binding,
            Err(error) => {
                self.fail_provider_launch(&started, &error);
                return Err(error);
            }
        };
        if let Err(error) = self.finish_provider_launch(&started, binding) {
            self.fail_provider_launch(&started, &error);
            return Err(error);
        }
        self.providers.get_run(started.run.id())
    }

    pub(crate) fn launch_provider_detached(
        &mut self,
        mut request: LaunchProviderRequest,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        if request.agent_id.is_none() {
            request.agent_id = self
                .sessions
                .get_session(&request.session_id)?
                .focused_agent_id()
                .map(str::to_string);
        }
        if request.resume_state.is_none() {
            if let Some(agent_id) = request.agent_id.as_deref() {
                if let Ok(agent) = self.agents.get_agent(agent_id) {
                    let resume_state = sanitize_resume_state_for_launch(&request, &agent);
                    if !resume_state.is_empty() {
                        request = request.with_resume_state(resume_state);
                    }
                }
            }
        }
        if (request.adapter_key == "opencode" || request.adapter_key == "codex")
            && request.working_directory.is_none()
        {
            let agent_worktree = request.agent_id.as_deref().and_then(|agent_id| {
                self.agents
                    .get_agent(agent_id)
                    .ok()
                    .and_then(|agent| agent.worktree_id().map(PathBuf::from))
            });
            request.working_directory = Some(agent_worktree.unwrap_or_else(|| {
                PathBuf::from(
                    self.sessions
                        .get_session(&request.session_id)
                        .map(|session| session.worktree_id().to_string())
                        .unwrap_or_default(),
                )
            }));
        }
        if request.runtime_mcp_binding.is_none() {
            let shared_auth_token = self
                .providers
                .get_session_run_for_provider(&request.session_id, &request.provider)
                .and_then(|run| run.runtime_mcp_auth_token().map(str::to_string));
            request = request.with_runtime_mcp_binding(RuntimeMcpBinding::new(
                self.config.runtime_mcp_url(),
                shared_auth_token.unwrap_or_else(generate_runtime_mcp_auth_token),
            ));
        }
        let run = self.providers.launch_run_detached(request)?;
        if run.endpoint_mode() == AgentEndpointMode::Managed {
            if let Err(error) = self.pty.spawn_for_run(&run) {
                let _ =
                    self.providers
                        .terminate_run(&mut self.sessions, run.session_id(), run.id());
                return Err(error);
            }
            self.register_managed_provider_process(&run)?;
        }
        self.providers.initialize_runtime(&run)?;
        let run = self.providers.get_run(run.id())?;
        let _ = self.providers.record_run_activity(run.id());
        if let Some(agent_id) = run.agent_instance_id() {
            let _ = self.agents.set_agent_runtime_profile(
                agent_id,
                run.provider(),
                Some(run.model().to_string()),
                run.variant().map(str::to_string),
                run.resume_state().clone(),
            )?;
        }
        Ok(run)
    }

    pub fn list_provider_processes(
        &self,
        provider: Option<&str>,
    ) -> Result<Vec<ProviderProcessInfo>, DaemonError> {
        let mut processes = Vec::new();
        for tracked in self.tracked_provider_processes.values() {
            if provider.is_some_and(|value| tracked.provider != value) {
                continue;
            }
            let runs = tracked
                .owner_provider_run_ids
                .iter()
                .filter_map(|run_id| self.providers.get_run(run_id).ok())
                .filter(|run| run.state() != ProviderRunState::Ended)
                .collect::<Vec<_>>();
            if runs.is_empty() {
                continue;
            }
            let owner_session_ids = runs
                .iter()
                .map(|run| run.session_id().to_string())
                .collect::<BTreeSet<_>>();
            let attached_session_ids = owner_session_ids
                .iter()
                .filter(|session_id| {
                    !self
                        .attachments
                        .list_session_attachment_ids(session_id)
                        .is_empty()
                })
                .cloned()
                .collect::<BTreeSet<_>>();
            let active_workflow_run_ids = owner_session_ids
                .iter()
                .flat_map(|session_id| {
                    self.sessions
                        .get_session(session_id)
                        .ok()
                        .map(|session| session.workflow_runs().iter().cloned().collect::<Vec<_>>())
                        .into_iter()
                        .flatten()
                        .filter(|run| {
                            !matches!(
                                run.status(),
                                crate::session::WorkflowRunStatus::Completed
                                    | crate::session::WorkflowRunStatus::Failed
                                    | crate::session::WorkflowRunStatus::Stopped
                            )
                        })
                        .map(|run| run.id().to_string())
                })
                .collect::<BTreeSet<_>>();
            let has_active_prompt = owner_session_ids.iter().any(|session_id| {
                self.sessions
                    .get_session(session_id)
                    .ok()
                    .and_then(|session| session.active_prompt().cloned())
                    .is_some()
            });
            let mut teardown_blockers = Vec::new();
            if !attached_session_ids.is_empty() {
                teardown_blockers.push(format!(
                    "attached sessions: {}",
                    attached_session_ids
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(",")
                ));
            }
            if has_active_prompt {
                teardown_blockers.push("active prompt".to_string());
            }
            if !active_workflow_run_ids.is_empty() {
                teardown_blockers.push(format!(
                    "active workflow runs: {}",
                    active_workflow_run_ids
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(",")
                ));
            }
            let teardown_safe = attached_session_ids.is_empty()
                && active_workflow_run_ids.is_empty()
                && !has_active_prompt;
            if let Some(mut process) = ProviderProcessInfo::from_runs(
                tracked.process_id.clone(),
                &runs,
                attached_session_ids,
                active_workflow_run_ids,
                teardown_safe,
                teardown_blockers,
            ) {
                process.pid = tracked.pid;
                process.process_label = tracked.process_label.clone();
                process.endpoint_mode = tracked.endpoint_mode;
                process.started_at_ms = tracked.started_at_ms;
                processes.push(process);
            }
        }
        Ok(processes)
    }

    pub fn teardown_provider_processes(
        &mut self,
        provider: Option<&str>,
    ) -> Result<Vec<ProviderProcessInfo>, DaemonError> {
        let safe_processes = self
            .list_provider_processes(provider)?
            .into_iter()
            .filter(|process| process.teardown_safe)
            .collect::<Vec<_>>();
        for process in &safe_processes {
            let run_ids: Vec<String> = self
                .tracked_provider_processes
                .values()
                .find(|tracked| tracked.process_id == process.process_id)
                .map(|tracked| tracked.owner_provider_run_ids.clone())
                .unwrap_or_else(|| process.owner_provider_run_ids.clone());
            for run_id in run_ids {
                let run = match self.providers.get_run(&run_id) {
                    Ok(run) => run,
                    Err(_) => continue,
                };
                if run.state() == ProviderRunState::Ended {
                    continue;
                }
                let _ =
                    self.providers
                        .terminate_run(&mut self.sessions, run.session_id(), run.id());
                let _ = self.remove_tracked_provider_process_for_run(run.id());
            }
        }
        Ok(safe_processes)
    }

    pub(crate) fn sync_active_provider_run_for_agent(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        let current_active_run_id = self
            .sessions
            .get_session(session_id)?
            .active_provider_run_id()
            .map(str::to_string);

        if let Some(current_active_run_id) = current_active_run_id.as_deref() {
            let active_run = self.providers.get_run(current_active_run_id)?;
            if active_run.agent_instance_id() != Some(agent_id)
                && active_run.state() == ProviderRunState::Running
            {
                self.providers
                    .park_run(&mut self.sessions, session_id, current_active_run_id)?;
            }
        }

        if let Some(agent_run) = self.providers.get_run_for_agent(session_id, agent_id) {
            match agent_run.state() {
                ProviderRunState::Running => {
                    self.sessions
                        .set_active_provider_run(session_id, Some(agent_run.id().to_string()))?;
                }
                ProviderRunState::Parked => {
                    self.providers
                        .resume_run(&mut self.sessions, session_id, agent_run.id())?;
                }
                ProviderRunState::Starting => {
                    self.sessions
                        .set_active_provider_run(session_id, Some(agent_run.id().to_string()))?;
                }
                ProviderRunState::Ended => {
                    self.sessions.set_active_provider_run(session_id, None)?;
                }
            }
        } else {
            self.sessions.set_active_provider_run(session_id, None)?;
        }

        Ok(())
    }

    pub(crate) fn should_defer_provider_run_sync_for_focus_change(
        &self,
        session_id: &str,
        target_agent_id: &str,
    ) -> Result<bool, DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        let Some(active_provider_run_id) = session.active_provider_run_id().map(str::to_string)
        else {
            return Ok(false);
        };
        let active_run = self.providers.get_run(&active_provider_run_id)?;
        if active_run.agent_instance_id() == Some(target_agent_id)
            || active_run.state() != ProviderRunState::Running
        {
            return Ok(false);
        }

        Ok(session.active_prompt().is_some()
            || session.agents().iter().any(|agent| agent.is_processing()))
    }

    pub(crate) fn sync_focused_provider_run_if_idle(
        &mut self,
        session_id: &str,
    ) -> Result<(), DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        if session.agents().len() > 1 {
            let focused_agent_id = session.focused_agent_id().map(str::to_string);
            if let Some(focused_agent_id) = focused_agent_id {
                if session.active_prompt().is_none() {
                    let current_active_run_id =
                        session.active_provider_run_id().map(str::to_string);
                    if let Some(current_active_run_id) = current_active_run_id.as_deref() {
                        let active_run = self.providers.get_run(current_active_run_id)?;
                        if active_run.agent_instance_id() != Some(focused_agent_id.as_str())
                            && active_run.state() == ProviderRunState::Running
                        {
                            self.providers.park_run(
                                &mut self.sessions,
                                session_id,
                                current_active_run_id,
                            )?;
                        }
                    }
                }
                self.project_active_provider_run_for_agent(session_id, &focused_agent_id)?;
            } else {
                self.sessions.set_active_provider_run(session_id, None)?;
            }
            return Ok(());
        }
        if session.active_prompt().is_some()
            || session.agents().iter().any(|agent| agent.is_processing())
        {
            return Ok(());
        }

        let focused_agent_id = session.focused_agent_id().map(str::to_string);
        if let Some(focused_agent_id) = focused_agent_id {
            self.sync_active_provider_run_for_agent(session_id, &focused_agent_id)?;
        } else {
            self.sessions.set_active_provider_run(session_id, None)?;
        }

        Ok(())
    }

    pub(crate) fn ensure_prompt_provider_run_for_agent(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<String, DaemonError> {
        if let Some(agent_run) = self.providers.get_run_for_agent(session_id, agent_id) {
            return match agent_run.state() {
                ProviderRunState::Running | ProviderRunState::Starting => {
                    Ok(agent_run.id().to_string())
                }
                ProviderRunState::Parked => {
                    let resumed = self.providers.resume_run_detached(agent_run.id())?;
                    Ok(resumed.id().to_string())
                }
                ProviderRunState::Ended => Err(DaemonError::NoActiveProviderRun {
                    session_id: session_id.to_string(),
                }),
            };
        }

        let agent = self.agents.get_agent(agent_id)?;
        let adapter_key = match agent.provider() {
            "default" => "opencode",
            value => value,
        };
        let provider = match agent.provider() {
            "default" => "opencode",
            value => value,
        };
        let mut request = LaunchProviderRequest::new(
            session_id,
            adapter_key,
            provider,
            "default",
            agent.model().unwrap_or("default"),
        )
        .with_agent_id(agent.id().to_string())
        .with_variant(agent.effort().map(str::to_string));
        if let Some(worktree_id) = agent.worktree_id() {
            request = request.with_working_directory(PathBuf::from(worktree_id));
        }
        let provider_run = self.launch_provider_detached(request)?;
        Ok(provider_run.id().to_string())
    }
}

fn sanitize_resume_state_for_launch(
    request: &LaunchProviderRequest,
    agent: &AgentInstance,
) -> ProviderResumeState {
    let resume_state = agent.provider_resume_state().clone();
    let requested_variant = request
        .variant
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    let agent_variant = agent.effort().filter(|value| !value.trim().is_empty());
    let model_or_variant_changed =
        agent.model() != Some(request.model.as_str()) || agent_variant != requested_variant;
    if !model_or_variant_changed {
        return resume_state;
    }

    match request.adapter_key.as_str() {
        "opencode" => resume_state.without_opencode_session_id(),
        "codex" => resume_state.without_codex_thread_id(),
        _ => resume_state,
    }
}

fn generate_runtime_mcp_auth_token() -> String {
    Alphanumeric.sample_string(&mut rand::thread_rng(), 32)
}

#[cfg(test)]
mod tests {
    use crate::agent::{AgentInstance, GridPosition};
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::config::DaemonConfig;
    use crate::provider::{LaunchProviderRequest, ProviderResumeState};
    use crate::session::CreateSessionRequest;

    use super::*;

    #[test]
    fn sanitize_resume_state_keeps_adapter_resume_when_model_and_variant_match() {
        let mut agent = AgentInstance::new(
            "agent-1",
            "agent-1",
            "session-1",
            None,
            "opencode",
            Some("openai/gpt-5.4".to_string()),
            Some("high".to_string()),
            None,
            GridPosition::new(0, 0, 1, 1),
        );
        let mut resume_state = ProviderResumeState::from_opencode_session_id("open-session-1");
        resume_state.set_codex_thread_id("thread-1");
        agent.set_provider_resume_state(resume_state.clone());
        let request = LaunchProviderRequest::new(
            "session-1",
            "opencode",
            "opencode",
            "default",
            "openai/gpt-5.4",
        )
        .with_variant(Some("high".to_string()));

        assert_eq!(
            sanitize_resume_state_for_launch(&request, &agent),
            resume_state
        );
    }

    #[test]
    fn sanitize_resume_state_clears_opencode_resume_when_model_changes() {
        let mut agent = AgentInstance::new(
            "agent-1",
            "agent-1",
            "session-1",
            None,
            "opencode",
            Some("openai/gpt-5.4".to_string()),
            Some("high".to_string()),
            None,
            GridPosition::new(0, 0, 1, 1),
        );
        let mut resume_state = ProviderResumeState::from_opencode_session_id("open-session-1");
        resume_state.set_codex_thread_id("thread-1");
        agent.set_provider_resume_state(resume_state);
        let request = LaunchProviderRequest::new(
            "session-1",
            "opencode",
            "opencode",
            "default",
            "anthropic/claude-sonnet-4",
        )
        .with_variant(Some("high".to_string()));

        let sanitized = sanitize_resume_state_for_launch(&request, &agent);
        assert_eq!(sanitized.opencode_session_id(), None);
        assert_eq!(sanitized.codex_thread_id(), Some("thread-1"));
    }

    #[test]
    fn sanitize_resume_state_clears_codex_resume_when_variant_changes() {
        let mut agent = AgentInstance::new(
            "agent-1",
            "agent-1",
            "session-1",
            None,
            "codex",
            Some("gpt-5.4".to_string()),
            Some("medium".to_string()),
            None,
            GridPosition::new(0, 0, 1, 1),
        );
        let mut resume_state = ProviderResumeState::from_codex_thread_id("thread-1");
        resume_state.set_opencode_session_id("open-session-1");
        agent.set_provider_resume_state(resume_state);
        let request =
            LaunchProviderRequest::new("session-1", "codex", "codex", "default", "gpt-5.4")
                .with_variant(Some("high".to_string()));

        let sanitized = sanitize_resume_state_for_launch(&request, &agent);
        assert_eq!(sanitized.opencode_session_id(), Some("open-session-1"));
        assert_eq!(sanitized.codex_thread_id(), None);
    }

    #[test]
    fn provider_processes_list_and_teardown_safe_idle_managed_runs() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, _agent) = app
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session create should succeed");
        let run = app
            .launch_provider(LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            ))
            .expect("provider launch should succeed");

        let processes = app
            .list_provider_processes(None)
            .expect("provider processes should list");
        assert_eq!(processes.len(), 1);
        assert!(processes[0].teardown_safe);
        assert!(processes[0].attached_session_ids.is_empty());
        assert_eq!(
            processes[0].owner_provider_run_ids,
            vec![run.id().to_string()]
        );
        assert_eq!(app.tracked_provider_processes.len(), 1);
        assert_eq!(app.tracked_provider_run_processes.len(), 1);
        assert_eq!(
            processes[0].pid,
            app.pty
                .process_id(run.id())
                .expect("pty pid should resolve")
        );

        let torn_down = app
            .teardown_provider_processes(None)
            .expect("safe teardown should succeed");
        assert_eq!(torn_down.len(), 1);
        assert!(app
            .list_provider_processes(None)
            .expect("provider processes should relist")
            .is_empty());
        assert!(app.tracked_provider_processes.is_empty());
        assert!(app.tracked_provider_run_processes.is_empty());
    }

    #[test]
    fn provider_processes_do_not_teardown_when_session_is_attached() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, _agent) = app
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session create should succeed");
        let _attachment = app
            .attach(AttachRequest::new(
                session.id(),
                "client-1",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("session should attach");
        let run = app
            .launch_provider(LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            ))
            .expect("provider launch should succeed");

        let processes = app
            .list_provider_processes(None)
            .expect("provider processes should list");
        assert_eq!(processes.len(), 1);
        assert!(!processes[0].teardown_safe);
        assert_eq!(
            processes[0].attached_session_ids,
            vec![session.id().to_string()]
        );
        assert_eq!(
            processes[0].teardown_blockers,
            vec![format!("attached sessions: {}", session.id())]
        );

        let torn_down = app
            .teardown_provider_processes(None)
            .expect("safe teardown should succeed");
        assert!(torn_down.is_empty());
        assert_eq!(
            app.providers()
                .get_run(run.id())
                .expect("run should still exist")
                .state(),
            crate::provider::ProviderRunState::Running,
        );
    }

    #[test]
    fn ending_session_clears_tracked_provider_processes() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, _agent) = app
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session create should succeed");
        let run = app
            .launch_provider(LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            ))
            .expect("provider launch should succeed");

        assert!(app
            .tracked_provider_processes
            .values()
            .any(|process| { process.owner_provider_run_ids == vec![run.id().to_string()] }));

        let _ = app.end_session(session.id()).expect("session should end");

        assert!(app.tracked_provider_processes.is_empty());
        assert!(app.tracked_provider_run_processes.is_empty());
        assert!(app
            .list_provider_processes(None)
            .expect("provider processes should list")
            .is_empty());
    }
}
