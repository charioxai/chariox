use std::path::PathBuf;

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::{
    AgentEndpointMode, LaunchProviderRequest, ProviderRunState, RuntimeProviderRun,
};

impl DaemonApp {
    pub fn launch_provider(
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
        if request.adapter_key == "opencode" && request.working_directory.is_none() {
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
        }
        crate::logging::info_with_fields(
            "daemon.app",
            "initializing provider runtime",
            serde_json::json!({
                "provider_run_id": run.id(),
                "session_id": run.session_id(),
            }),
        );
        if let Err(error) = self.providers.initialize_runtime(&run) {
            crate::logging::error_with_fields(
                "daemon.app",
                "provider runtime initialization failed",
                serde_json::json!({
                    "provider_run_id": run.id(),
                    "session_id": run.session_id(),
                    "error": error.to_string(),
                }),
            );
            let _ = self.pty.remove_process(run.id());
            self.providers.clear_runtime(run.id());
            let _ = self
                .providers
                .terminate_run(&mut self.sessions, run.session_id(), run.id());
            if let Some(previous_active_run_id) = previous_active_run_id.as_deref() {
                let _ = self.providers.resume_run(
                    &mut self.sessions,
                    run.session_id(),
                    previous_active_run_id,
                );
            }
            return Err(error);
        }
        crate::logging::info_with_fields(
            "daemon.app",
            "provider runtime initialized successfully",
            serde_json::json!({
                "provider_run_id": run.id(),
                "session_id": run.session_id(),
            }),
        );
        if let Some(agent_id) = run.agent_instance_id() {
            let _ = self.agents.set_agent_runtime_profile(
                agent_id,
                run.provider(),
                Some(run.model().to_string()),
                run.variant().map(str::to_string),
            )?;
        }
        self.providers.get_run(run.id())
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

    pub(crate) fn ensure_active_provider_run_for_agent(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<String, DaemonError> {
        self.sync_active_provider_run_for_agent(session_id, agent_id)?;
        if let Some(provider_run_id) = self
            .sessions
            .get_session(session_id)?
            .active_provider_run_id()
            .map(str::to_string)
        {
            return Ok(provider_run_id);
        }

        Err(DaemonError::NoActiveProviderRun {
            session_id: session_id.to_string(),
        })
    }
}
