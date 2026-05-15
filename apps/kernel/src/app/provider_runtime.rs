use std::path::PathBuf;

use rand::distributions::{Alphanumeric, DistString};

use crate::agent::AgentInstance;
use crate::app::DaemonApp;
use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::provider::{
    AgentEndpointMode, LaunchProviderRequest, ProviderProcessInfo, ProviderProcessService,
    ProviderResumeState, ProviderRunLivenessReconciliation, ProviderRunState,
    ProviderRuntimeBinding, RuntimeMcpBinding, RuntimeProviderRun,
};
use crate::pty::PtyProcessState;
use crate::session::PromptStatus;

pub(crate) use super::provider_processes::ProviderProcessTracker;

#[derive(Debug, Clone)]
pub(crate) struct StartedProviderLaunch {
    pub(crate) run: RuntimeProviderRun,
    pub(crate) previous_active_run_id: Option<String>,
}

pub(crate) struct ProviderLaunchProcessRuntime<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> ProviderLaunchProcessRuntime<'a> {
    pub(crate) fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn spawn_for_launch(&mut self, run: &RuntimeProviderRun) -> Result<(), DaemonError> {
        if run.endpoint_mode() != AgentEndpointMode::Managed {
            return Ok(());
        }
        self.app.pty.spawn_for_run(run)?;
        ProviderProcessTracker::new(self.app).register_managed_run(run)
    }

    pub(crate) fn remove_run(
        &mut self,
        provider_run_id: &str,
    ) -> Result<(bool, Option<String>), DaemonError> {
        let process_key = self.app.pty.process_key(provider_run_id).ok();
        let removed = self.app.pty.remove_process(provider_run_id)?;
        Ok((removed, process_key))
    }

    pub(crate) fn poll_running(&mut self, provider_run_id: &str) -> Result<bool, DaemonError> {
        ProviderRunLivenessProcesses::poll_process_running(self.app, provider_run_id)
    }
}

pub(crate) struct ProviderRunReadService<'a> {
    app: &'a DaemonApp,
}

impl<'a> ProviderRunReadService<'a> {
    pub(crate) fn new(app: &'a DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn ensure_provider_run_in_session(
        &self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let provider_run = self.app.providers.get_run(provider_run_id)?;

        if provider_run.session_id() != session_id {
            return Err(DaemonError::ProviderRunNotInSession {
                session_id: session_id.to_string(),
                provider_run_id: provider_run_id.to_string(),
            });
        }

        Ok(provider_run)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum ProviderRunExitPromptSettlement {
    FinalizeCancellation,
    CompleteActivePrompt,
    SyncIdleProvider,
}

impl ProviderRunExitPromptSettlement {
    fn from_active_prompt_status(active_prompt_status: Option<PromptStatus>) -> Self {
        match active_prompt_status {
            Some(PromptStatus::Cancelling) => Self::FinalizeCancellation,
            Some(_) => Self::CompleteActivePrompt,
            None => Self::SyncIdleProvider,
        }
    }
}

pub(crate) struct ProviderRunLivenessRuntime<'a> {
    app: &'a mut DaemonApp,
}

#[derive(Debug, Clone)]
struct ProviderRunLivenessOutcome {
    ended_run: RuntimeProviderRun,
    session_id: String,
    provider_run_id: String,
    agent_id: String,
    transition: ProviderRunLivenessTransition,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum ProviderRunLivenessTransition {
    AlreadyEnded,
    UnexpectedExit,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
struct ProviderRunExitSessionOutcome {
    had_active_prompt: bool,
    started_next_prompt: bool,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) struct ProviderRunExitSessionSummary {
    pub(crate) had_active_prompt: bool,
    pub(crate) started_next_prompt: bool,
}

struct ProviderRunLivenessProcesses;

impl ProviderRunLivenessProcesses {
    fn poll_process_running(
        app: &mut DaemonApp,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        match app.pty.poll_process_state(provider_run_id) {
            Ok(PtyProcessState::Running) => Ok(true),
            Ok(PtyProcessState::Exited) => Ok(false),
            Err(DaemonError::PtyProcessNotFound { .. }) => Ok(false),
            Err(error) => Err(error),
        }
    }

    fn remove_tracked_process(
        app: &mut DaemonApp,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        ProviderProcessTracker::new(app).remove_run(provider_run_id)
    }
}

struct ProviderRunLivenessState;

impl ProviderRunLivenessState {
    fn reconcile_run_liveness(
        app: &mut DaemonApp,
        session_id: &str,
        provider_run_id: &str,
        process_running: Option<bool>,
    ) -> Result<ProviderRunLivenessReconciliation, DaemonError> {
        let reconciliation = app.providers.reconcile_run_liveness_provider_only(
            session_id,
            provider_run_id,
            process_running,
        )?;
        Self::sync_ended_provider_run_session_state(
            app,
            session_id,
            provider_run_id,
            &reconciliation,
        )?;
        Ok(reconciliation)
    }

    fn sync_ended_provider_run_session_state(
        app: &mut DaemonApp,
        session_id: &str,
        provider_run_id: &str,
        reconciliation: &ProviderRunLivenessReconciliation,
    ) -> Result<(), DaemonError> {
        if !matches!(
            reconciliation,
            ProviderRunLivenessReconciliation::AlreadyEnded(_)
                | ProviderRunLivenessReconciliation::NewlyEnded(_)
        ) {
            return Ok(());
        }
        Self::clear_active_provider_run_session_pointer(app, session_id, provider_run_id)
    }

    fn clear_active_provider_run_session_pointer(
        app: &mut DaemonApp,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<(), DaemonError> {
        if app
            .sessions
            .get_session(session_id)?
            .active_provider_run_id()
            == Some(provider_run_id)
        {
            app.sessions.set_active_provider_run(session_id, None)?;
        }
        Ok(())
    }
}

struct ProviderRunActivationState;

impl ProviderRunActivationState {
    fn start_provider_run_for_session(
        app: &mut DaemonApp,
        request: LaunchProviderRequest,
    ) -> Result<StartedProviderLaunch, DaemonError> {
        let session_id = request.session_id.clone();
        let previous_active_run_id = app
            .sessions
            .get_session(&session_id)?
            .active_provider_run_id()
            .map(str::to_owned);

        if let Some(active_run_id) = previous_active_run_id.as_deref() {
            let active_run = app.providers.get_run(active_run_id)?;
            match active_run.state() {
                ProviderRunState::Ended => {
                    app.sessions.set_active_provider_run(&session_id, None)?;
                    app.providers.clear_runtime(active_run_id);
                }
                ProviderRunState::Starting => {
                    let outcome = app
                        .providers
                        .terminate_run_provider_only(&session_id, active_run_id)?;
                    ProviderRunLivenessState::clear_active_provider_run_session_pointer(
                        app,
                        &session_id,
                        outcome.run().id(),
                    )?;
                    app.update_provider_run_projection(outcome.into_run());
                }
                ProviderRunState::Running => {
                    if !app.provider_run_has_active_prompt(&session_id, &active_run)? {
                        let outcome = app
                            .providers
                            .park_run_provider_only(&session_id, active_run_id)?;
                        ProviderRunLivenessState::clear_active_provider_run_session_pointer(
                            app,
                            &session_id,
                            outcome.run().id(),
                        )?;
                        app.update_provider_run_projection(outcome.into_run());
                    }
                }
                ProviderRunState::Parked => {
                    app.sessions.set_active_provider_run(&session_id, None)?;
                }
            }
        }

        let outcome = app.providers.start_run_provider_only(request)?;
        app.sessions
            .set_active_provider_run(&session_id, Some(outcome.run().id().to_string()))?;
        Ok(StartedProviderLaunch {
            run: outcome.into_run(),
            previous_active_run_id,
        })
    }

    fn resume_provider_run_for_session(
        app: &mut DaemonApp,
        session_id: &str,
        run_id: &str,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let active_run_id = app
            .sessions
            .get_session(session_id)?
            .active_provider_run_id()
            .map(str::to_owned);

        if let Some(active_run_id) = active_run_id.as_deref() {
            if active_run_id != run_id {
                let active_run = app.providers.get_run(active_run_id)?;
                match active_run.state() {
                    ProviderRunState::Running => {
                        if !app.provider_run_has_active_prompt(session_id, &active_run)? {
                            let outcome = app
                                .providers
                                .park_run_provider_only(session_id, active_run_id)?;
                            ProviderRunLivenessState::clear_active_provider_run_session_pointer(
                                app,
                                session_id,
                                outcome.run().id(),
                            )?;
                            app.update_provider_run_projection(outcome.into_run());
                        }
                    }
                    ProviderRunState::Starting => {
                        let outcome = app
                            .providers
                            .terminate_run_provider_only(session_id, active_run_id)?;
                        ProviderRunLivenessState::clear_active_provider_run_session_pointer(
                            app,
                            session_id,
                            outcome.run().id(),
                        )?;
                        app.update_provider_run_projection(outcome.into_run());
                    }
                    ProviderRunState::Parked | ProviderRunState::Ended => {
                        app.sessions.set_active_provider_run(session_id, None)?;
                    }
                }
            }
        }

        let outcome = app.providers.resume_run_provider_only(session_id, run_id)?;
        app.sessions
            .set_active_provider_run(session_id, Some(outcome.run().id().to_string()))?;
        let run = outcome.into_run();
        app.update_provider_run_projection(run.clone());
        Ok(run)
    }
}

struct ProviderRunLivenessNotices;

impl ProviderRunLivenessNotices {
    fn record_provider_exit(
        app: &mut DaemonApp,
        session_id: &str,
        provider_run_id: &str,
        message: String,
    ) {
        let recipients = app.attachments.list_session_attachment_ids(session_id);
        app.record_notice(session_id, Some(provider_run_id), recipients, message);
    }
}

struct ProviderRunLivenessSessionEffects;

impl ProviderRunLivenessSessionEffects {
    fn apply_provider_exit(
        app: &mut DaemonApp,
        outcome: &ProviderRunLivenessOutcome,
    ) -> Result<ProviderRunExitSessionOutcome, DaemonError> {
        let active_prompt_status = app
            .prompt_owner_active_prompt_for_agent(&outcome.session_id, &outcome.agent_id)?
            .map(|prompt| prompt.status());
        let had_active_prompt = active_prompt_status.is_some();
        let started_next_prompt = match ProviderRunExitPromptSettlement::from_active_prompt_status(
            active_prompt_status,
        ) {
            ProviderRunExitPromptSettlement::FinalizeCancellation => app
                .finalize_active_prompt_cancellation(
                    &outcome.session_id,
                    &outcome.agent_id,
                    Some(&outcome.provider_run_id),
                )?
                .started_next
                .is_some(),
            ProviderRunExitPromptSettlement::CompleteActivePrompt => app
                .complete_active_prompt(
                    &outcome.session_id,
                    &outcome.agent_id,
                    Some(&outcome.provider_run_id),
                )?
                .started_next
                .is_some(),
            ProviderRunExitPromptSettlement::SyncIdleProvider => {
                app.sync_focused_provider_run_if_idle(&outcome.session_id)?;
                false
            }
        };

        Ok(ProviderRunExitSessionOutcome {
            had_active_prompt,
            started_next_prompt,
        })
    }
}

impl<'a> ProviderRunLivenessRuntime<'a> {
    pub(crate) fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn reconcile_provider_run_exit(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        let Some(outcome) =
            self.reconcile_provider_run_exit_provider_phase(session_id, provider_run_id)?
        else {
            return Ok(false);
        };
        if outcome.transition == ProviderRunLivenessTransition::AlreadyEnded {
            return Ok(true);
        }

        let session_outcome =
            ProviderRunLivenessSessionEffects::apply_provider_exit(self.app, &outcome)?;
        ProviderRunLivenessNotices::record_provider_exit(
            self.app,
            &outcome.session_id,
            &outcome.provider_run_id,
            format!(
                "Provider run `{}` for `{}` ended unexpectedly. {}",
                outcome.provider_run_id,
                outcome.ended_run.provider(),
                if session_outcome.had_active_prompt {
                    if session_outcome.started_next_prompt {
                        "The active prompt was closed and Arroba advanced the queued backlog onto the next available provider run."
                    } else {
                        "The active prompt was closed without starting the queued backlog."
                    }
                } else {
                    "No active prompt was running."
                }
            ),
        );

        Ok(true)
    }

    fn reconcile_provider_run_exit_provider_phase(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<Option<ProviderRunLivenessOutcome>, DaemonError> {
        let provider_run = ProviderRunReadService::new(self.app)
            .ensure_provider_run_in_session(session_id, provider_run_id)?;
        let agent_id = provider_run
            .agent_instance_id()
            .map(str::to_string)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: "provider run has no agent".to_string(),
            })?;
        match ProviderRunLivenessState::reconcile_run_liveness(
            self.app,
            session_id,
            provider_run_id,
            None,
        )? {
            ProviderRunLivenessReconciliation::AlreadyEnded(run) => {
                self.app.update_provider_run_projection(run.clone());
                let _ = ProviderRunLivenessProcesses::remove_tracked_process(
                    self.app,
                    provider_run_id,
                )?;
                return Ok(Some(ProviderRunLivenessOutcome {
                    ended_run: run,
                    session_id: session_id.to_string(),
                    provider_run_id: provider_run_id.to_string(),
                    agent_id,
                    transition: ProviderRunLivenessTransition::AlreadyEnded,
                }));
            }
            ProviderRunLivenessReconciliation::ExternalEndpoint(_)
            | ProviderRunLivenessReconciliation::NewlyEnded(_) => return Ok(None),
            ProviderRunLivenessReconciliation::StillRunning(_) => {}
        }

        let process_running =
            ProviderRunLivenessProcesses::poll_process_running(self.app, provider_run_id)?;
        let ended_run = match ProviderRunLivenessState::reconcile_run_liveness(
            self.app,
            session_id,
            provider_run_id,
            Some(process_running),
        )? {
            ProviderRunLivenessReconciliation::AlreadyEnded(run)
            | ProviderRunLivenessReconciliation::NewlyEnded(run) => run,
            ProviderRunLivenessReconciliation::ExternalEndpoint(_)
            | ProviderRunLivenessReconciliation::StillRunning(_) => return Ok(None),
        };
        self.app.update_provider_run_projection(ended_run.clone());
        let _ = ProviderRunLivenessProcesses::remove_tracked_process(self.app, provider_run_id)?;

        Ok(Some(ProviderRunLivenessOutcome {
            ended_run,
            session_id: session_id.to_string(),
            provider_run_id: provider_run_id.to_string(),
            agent_id,
            transition: ProviderRunLivenessTransition::UnexpectedExit,
        }))
    }
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
                    .and_then(|agent_id| {
                        self.prompt_state_owner
                            .active_prompt_for_agent_snapshot(session, agent_id)
                    })
                    .is_some();
                if active_run.state() == ProviderRunState::Running && active_prompt_is_running {
                    return;
                }
            }
        }

        let projected_agent_id = self
            .prompt_state_owner
            .active_prompt_agent_id(session)
            .or_else(|| session.focused_agent_id().map(str::to_string));
        let projected_run_id = projected_agent_id.as_deref().and_then(|agent_id| {
            self.providers
                .get_run_for_agent(session.id(), agent_id)
                .or_else(|| {
                    self.provider_run_projection
                        .get_for_agent(session.id(), agent_id)
                })
                .and_then(|run| match run.state() {
                    ProviderRunState::Running | ProviderRunState::Starting => {
                        Some(run.id().to_string())
                    }
                    ProviderRunState::Parked | ProviderRunState::Ended => None,
                })
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
            .or_else(|| {
                self.provider_run_projection
                    .get_for_agent(session_id, agent_id)
            })
            .and_then(|run| match run.state() {
                ProviderRunState::Running | ProviderRunState::Starting => {
                    Some(run.id().to_string())
                }
                ProviderRunState::Parked | ProviderRunState::Ended => None,
            });
        let _ = self
            .sessions
            .set_active_provider_run(session_id, projected_run_id)?;
        Ok(())
    }

    pub(crate) fn start_provider_launch(
        &mut self,
        mut request: LaunchProviderRequest,
    ) -> Result<StartedProviderLaunch, DaemonError> {
        request = self.prepare_app_provider_launch_request(request, "launch provider run")?;
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
        let request_session_id = request.session_id.clone();
        let recipients = self
            .attachments
            .list_session_attachment_ids(&request_session_id);
        let started = ProviderRunActivationState::start_provider_run_for_session(self, request)?;
        let run = started.run.clone();
        if let Some(previous_active_run_id) = started.previous_active_run_id.as_deref() {
            if let Ok(previous_run) = self.providers.get_run(previous_active_run_id) {
                self.update_provider_run_projection(previous_run);
            }
        }
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
                if let Ok(outcome) = self
                    .providers
                    .terminate_run_provider_only(run.session_id(), run.id())
                {
                    ProviderRunLivenessState::clear_active_provider_run_session_pointer(
                        self,
                        run.session_id(),
                        outcome.run().id(),
                    )?;
                    self.update_provider_run_projection(outcome.into_run());
                }
                if let Some(previous_active_run_id) = started.previous_active_run_id.as_deref() {
                    match ProviderRunActivationState::resume_provider_run_for_session(
                        self,
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
            ProviderProcessTracker::new(self).register_managed_run(&run)?;
        }
        self.update_provider_run_projection(run.clone());
        Ok(started)
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
        let recipients = self
            .attachments
            .list_session_attachment_ids(started.run.session_id());
        self.record_notice(
            started.run.session_id(),
            Some(started.run.id()),
            recipients,
            format!(
                "Provider launch `{}` failed before it became ready: {}",
                started.run.id(),
                error
            ),
        );
        let diagnostic = format!(
            "Provider launch `{}` failed before it became ready: {}",
            started.run.id(),
            error
        );
        if let Ok(run) = self
            .providers
            .record_terminal_diagnostic(started.run.id(), diagnostic.clone())
        {
            self.update_provider_run_projection(run);
        }
        if let Some(agent) = self.clear_failed_codex_resume_state(started, error) {
            let _ = self.durable_state_store().append_event(
                "agent.runtime_profile_updated",
                Some(agent.id().to_string()),
                serde_json::json!({
                    "agent": &agent,
                    "provider_run_id": started.run.id(),
                    "reason": "failed_codex_resume_state_cleared",
                }),
            );
        }
        if let Some(agent_id) = started.run.agent_instance_id() {
            if let Ok(Some(active_prompt)) =
                self.prompt_owner_active_prompt_for_agent(started.run.session_id(), agent_id)
            {
                if active_prompt.workflow_run_id().is_some() {
                    let _ = crate::scheduler::runtime::on_workflow_provider_failure(
                        self,
                        started.run.session_id(),
                        &active_prompt,
                        Some(started.run.id()),
                        &diagnostic,
                    );
                }
                let _ = self.complete_active_prompt(
                    started.run.session_id(),
                    agent_id,
                    Some(started.run.id()),
                );
            }
        }
        let _ = ProviderProcessTracker::new(self).remove_run(started.run.id());
        self.providers.clear_runtime(started.run.id());
        if let Ok(outcome) = self
            .providers
            .terminate_run_provider_only(started.run.session_id(), started.run.id())
        {
            ProviderRunLivenessState::clear_active_provider_run_session_pointer(
                self,
                started.run.session_id(),
                outcome.run().id(),
            )
            .ok();
            self.update_provider_run_projection(outcome.into_run());
        }
        if let Some(previous_active_run_id) = started.previous_active_run_id.as_deref() {
            let _ = ProviderRunActivationState::resume_provider_run_for_session(
                self,
                started.run.session_id(),
                previous_active_run_id,
            );
        }
        let _ = crate::app::KernelSessionReadService::new(self)
            .session_snapshot(started.run.session_id());
    }

    fn clear_failed_codex_resume_state(
        &mut self,
        started: &StartedProviderLaunch,
        error: &DaemonError,
    ) -> Option<AgentInstance> {
        let replacement_resume_state = failed_codex_resume_state_replacement(&started.run, error)?;
        let agent_id = started.run.agent_instance_id()?;
        let stale_thread_id = started.run.resume_state().codex_thread_id()?.to_string();
        let current = self.agents.get_agent(agent_id).ok()?;
        if current.provider_resume_state().codex_thread_id() != Some(stale_thread_id.as_str()) {
            return None;
        }
        let agent = self
            .agents
            .set_agent_runtime_profile(
                agent_id,
                started.run.provider(),
                Some(started.run.model().to_string()),
                started.run.variant().map(str::to_string),
                replacement_resume_state,
            )
            .ok()?;
        self.record_notice(
            started.run.session_id(),
            Some(started.run.id()),
            self.attachments
                .list_session_attachment_ids(started.run.session_id()),
            format!(
                "Codex resume thread `{stale_thread_id}` is no longer available. Arroba cleared it from the agent profile so the next prompt can start a new durable Codex thread."
            ),
        );
        Some(agent)
    }

    fn finish_provider_launch_success(
        &mut self,
        run: &RuntimeProviderRun,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let run = self.providers.mark_run_running(run.id())?;
        self.sessions
            .set_active_provider_run(run.session_id(), Some(run.id().to_string()))?;
        crate::app::KernelSessionReadService::new(self).session_snapshot(run.session_id())?;
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
        let _ = self.providers.record_run_activity(run.id());
        if let Some(agent_id) = run.agent_instance_id() {
            let agent = self.agents.set_agent_runtime_profile(
                agent_id,
                run.provider(),
                Some(run.model().to_string()),
                run.variant().map(str::to_string),
                run.resume_state().clone(),
            )?;
            self.durable_state_store().append_event(
                "agent.runtime_profile_updated",
                Some(agent.id().to_string()),
                serde_json::json!({
                    "agent": &agent,
                    "provider_run_id": run.id(),
                }),
            )?;
            let _ = self.advance_next_queued_prompt(run.session_id(), agent_id)?;
            crate::app::KernelSessionReadService::new(self).session_snapshot(run.session_id())?;
        }
        self.update_provider_run_projection(run.clone());
        Ok(run)
    }

    pub fn launch_provider(
        &mut self,
        request: LaunchProviderRequest,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let started = self.start_provider_launch(request)?;
        let binding = match ProviderProcessService::initialize_runtime_binding(&started.run) {
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
        request = self.prepare_app_provider_launch_request(request, "launch provider run")?;
        let run = self.providers.launch_run_detached(request)?;
        if run.endpoint_mode() == AgentEndpointMode::Managed {
            if let Err(error) = self.pty.spawn_for_run(&run) {
                let started = StartedProviderLaunch {
                    run: run.clone(),
                    previous_active_run_id: None,
                };
                self.fail_provider_launch(&started, &error);
                return Err(error);
            }
            ProviderProcessTracker::new(self).register_managed_run(&run)?;
        }
        if let Err(error) = self.providers.initialize_runtime(&run) {
            let started = StartedProviderLaunch {
                run: run.clone(),
                previous_active_run_id: None,
            };
            self.fail_provider_launch(&started, &error);
            return Err(error);
        }
        let run = self.providers.get_run(run.id())?;
        let _ = self.providers.record_run_activity(run.id());
        if let Some(agent_id) = run.agent_instance_id() {
            let agent = self.agents.set_agent_runtime_profile(
                agent_id,
                run.provider(),
                Some(run.model().to_string()),
                run.variant().map(str::to_string),
                run.resume_state().clone(),
            )?;
            self.durable_state_store().append_event(
                "agent.runtime_profile_updated",
                Some(agent.id().to_string()),
                serde_json::json!({
                    "agent": &agent,
                    "provider_run_id": run.id(),
                }),
            )?;
        }
        self.update_provider_run_projection(run.clone());
        Ok(run)
    }

    fn prepare_app_provider_launch_request(
        &self,
        mut request: LaunchProviderRequest,
        operation: &'static str,
    ) -> Result<LaunchProviderRequest, DaemonError> {
        let session = self.sessions.get_session(&request.session_id)?;
        if request.agent_id.is_none() {
            request.agent_id = session.focused_agent_id().map(str::to_string);
        }
        let agent = request
            .agent_id
            .as_deref()
            .and_then(|agent_id| self.agents.get_agent(agent_id).ok());
        if let Some(agent) = agent.as_ref() {
            if agent.remote_execution().is_some() {
                return Err(DaemonError::LocalTransport {
                    operation,
                    message: format!(
                        "agent `{}` is remote-backed and must launch its provider on the worker kernel",
                        agent.id()
                    ),
                });
            }
            request = request.with_owner_user_id(agent.owner_user_id().to_string());
        } else {
            request = request.with_owner_user_id(session.owner_user_id().to_string());
        }
        if request.resume_state.is_none() {
            if let Some(agent) = agent.as_ref() {
                let resume_state = sanitize_resume_state_for_launch(&request, agent);
                if !resume_state.is_empty() {
                    request = request.with_resume_state(resume_state);
                }
            }
        }
        if request.working_directory.is_none() {
            let working_directory = agent
                .as_ref()
                .and_then(|agent| agent.worktree_id().map(PathBuf::from))
                .unwrap_or_else(|| PathBuf::from(session.worktree_id()));
            request = request.with_working_directory(working_directory);
        }
        if request.runtime_mcp_binding.is_none() {
            let shared_auth_token = request
                .agent_id
                .is_none()
                .then(|| {
                    self.providers
                        .get_session_run_for_provider(&request.session_id, &request.provider)
                        .and_then(|run| run.runtime_mcp_auth_token().map(str::to_string))
                })
                .flatten();
            request = request.with_runtime_mcp_binding(RuntimeMcpBinding::new(
                self.config.runtime_mcp_url(),
                shared_auth_token.unwrap_or_else(generate_runtime_mcp_auth_token),
            ));
        }
        if request.provider_env_remove.is_empty() {
            request = request.with_provider_env_remove(default_provider_env_remove(&self.config));
        }
        Ok(request)
    }

    pub fn list_provider_processes(
        &self,
        provider: Option<&str>,
    ) -> Result<Vec<ProviderProcessInfo>, DaemonError> {
        ProviderProcessTracker::list(self, provider)
    }

    pub fn teardown_provider_processes(
        &mut self,
        provider: Option<&str>,
        force: bool,
    ) -> Result<Vec<ProviderProcessInfo>, DaemonError> {
        ProviderProcessTracker::new(self).teardown_safe_processes(provider, force)
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
            let active_run = self.providers.get_run(current_active_run_id).or_else(|_| {
                self.provider_run_projection
                    .get(current_active_run_id)
                    .ok_or_else(|| DaemonError::ProviderRunNotFound {
                        provider_run_id: current_active_run_id.to_string(),
                    })
            })?;
            if active_run.agent_instance_id() != Some(agent_id)
                && active_run.state() == ProviderRunState::Running
                && active_run.client_interface().is_arroba()
                && !self.provider_run_has_active_prompt(session_id, &active_run)?
            {
                let outcome = self
                    .providers
                    .park_run_provider_only(session_id, current_active_run_id)?;
                ProviderRunLivenessState::clear_active_provider_run_session_pointer(
                    self,
                    session_id,
                    outcome.run().id(),
                )?;
                self.update_provider_run_projection(outcome.into_run());
            }
        }

        if let Some(agent_run) = self
            .providers
            .get_run_for_agent(session_id, agent_id)
            .or_else(|| {
                self.provider_run_projection
                    .get_for_agent(session_id, agent_id)
            })
        {
            match agent_run.state() {
                ProviderRunState::Running => {
                    self.sessions
                        .set_active_provider_run(session_id, Some(agent_run.id().to_string()))?;
                }
                ProviderRunState::Parked => {
                    if self.providers.get_run(agent_run.id()).is_ok() {
                        ProviderRunActivationState::resume_provider_run_for_session(
                            self,
                            session_id,
                            agent_run.id(),
                        )?;
                    } else {
                        self.sessions.set_active_provider_run(session_id, None)?;
                    }
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
        let active_run = self
            .providers
            .get_run(&active_provider_run_id)
            .or_else(|_| {
                self.provider_run_projection
                    .get(&active_provider_run_id)
                    .ok_or_else(|| DaemonError::ProviderRunNotFound {
                        provider_run_id: active_provider_run_id.clone(),
                    })
            })?;
        if active_run.agent_instance_id() == Some(target_agent_id)
            || active_run.state() != ProviderRunState::Running
        {
            return Ok(false);
        }

        Ok(self.prompt_state_owner.has_any_active_prompt(&session)
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
                let active_prompt_agent_id =
                    self.prompt_state_owner.active_prompt_agent_id(&session);
                let has_active_prompt = self.prompt_state_owner.has_any_active_prompt(&session);
                let has_processing_agent =
                    session.agents().iter().any(|agent| agent.is_processing());
                if !has_active_prompt {
                    let current_active_run_id =
                        session.active_provider_run_id().map(str::to_string);
                    if let Some(current_active_run_id) = current_active_run_id.as_deref() {
                        let active_run = self.providers.get_run(current_active_run_id)?;
                        if active_run.agent_instance_id() != Some(focused_agent_id.as_str())
                            && active_run.state() == ProviderRunState::Running
                            && !self.provider_run_has_active_prompt(session_id, &active_run)?
                        {
                            let outcome = self
                                .providers
                                .park_run_provider_only(session_id, current_active_run_id)?;
                            ProviderRunLivenessState::clear_active_provider_run_session_pointer(
                                self,
                                session_id,
                                outcome.run().id(),
                            )?;
                            self.update_provider_run_projection(outcome.into_run());
                        }
                    }
                }
                if has_active_prompt {
                    if let Some(projected_agent_id) = active_prompt_agent_id.as_deref() {
                        self.project_active_provider_run_for_agent(session_id, projected_agent_id)?;
                    }
                } else if has_processing_agent {
                    self.project_active_provider_run_for_agent(session_id, &focused_agent_id)?;
                } else {
                    self.sync_active_provider_run_for_agent(session_id, &focused_agent_id)?;
                }
            } else {
                self.sessions.set_active_provider_run(session_id, None)?;
            }
            return Ok(());
        }
        if self.prompt_state_owner.has_any_active_prompt(&session)
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

    pub(crate) fn provider_run_has_active_prompt(
        &self,
        session_id: &str,
        provider_run: &RuntimeProviderRun,
    ) -> Result<bool, DaemonError> {
        let Some(agent_id) = provider_run.agent_instance_id() else {
            return Ok(false);
        };
        let session = self.sessions.get_session(session_id)?;
        Ok(self
            .prompt_state_owner
            .active_prompt_for_agent_snapshot(&session, agent_id)
            .is_some())
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
                    self.update_provider_run_projection(resumed.clone());
                    Ok(resumed.id().to_string())
                }
                ProviderRunState::Ended => Err(DaemonError::NoActiveProviderRun {
                    session_id: session_id.to_string(),
                }),
            };
        }

        let agent = self.agents.get_agent(agent_id)?;
        if agent.remote_execution().is_some() {
            return Err(DaemonError::LocalTransport {
                operation: "ensure prompt provider run for agent",
                message: format!(
                    "agent `{agent_id}` is remote-backed and must launch its provider on the worker kernel"
                ),
            });
        }
        let adapter_key = match agent.provider() {
            "default" => "opencode",
            value => value,
        };
        let provider = match agent.provider() {
            "default" => "opencode",
            value => value,
        };
        let session = self.sessions.get_session(session_id)?;
        let effective_config =
            crate::session::effective_agent_execution_config(&session, Some(&agent));
        let mut request = LaunchProviderRequest::new(
            session_id,
            adapter_key,
            provider,
            "default",
            agent.model().unwrap_or("default"),
        )
        .with_agent_id(agent.id().to_string())
        .with_variant(agent.effort().map(str::to_string))
        .with_execution_mode(effective_config.mode)
        .with_permission_level(effective_config.permission_level);
        if crate::provider::provider_requires_managed_io_by_default(provider, &self.config) {
            request = request.with_managed_io_required();
        }
        if let Some(worktree_id) = agent.worktree_id() {
            request = request.with_working_directory(PathBuf::from(worktree_id));
        }
        let provider_run = self.launch_provider_detached(request)?;
        Ok(provider_run.id().to_string())
    }
}

fn default_provider_env_remove(config: &DaemonConfig) -> Vec<String> {
    crate::secret::RuntimeSecretService::credential_env_names_from(&config.user_config.credentials)
        .into_iter()
        .collect()
}

pub(crate) fn sanitize_resume_state_for_launch(
    request: &LaunchProviderRequest,
    agent: &AgentInstance,
) -> ProviderResumeState {
    let resume_state = agent.provider_resume_state().clone();
    let requested_variant = request
        .variant
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    let agent_variant = agent.effort().filter(|value| !value.trim().is_empty());
    let requested_model =
        normalize_resume_model_for_adapter(&request.adapter_key, request.model.as_str());
    let agent_model = agent
        .model()
        .map(|model| normalize_resume_model_for_adapter(&request.adapter_key, model));
    let model_or_variant_changed = agent_model.as_deref() != Some(requested_model.as_str())
        || agent_variant != requested_variant;
    if !model_or_variant_changed {
        return resume_state;
    }

    match request.adapter_key.as_str() {
        "opencode" => resume_state.without_opencode_session_id(),
        "codex" => resume_state.without_codex_thread_id(),
        "claude" => resume_state.without_claude_session_id(),
        _ => resume_state,
    }
}

pub(crate) fn failed_codex_resume_state_replacement(
    run: &RuntimeProviderRun,
    error: &DaemonError,
) -> Option<ProviderResumeState> {
    if run.adapter_key() != "codex" || run.resume_state().codex_thread_id().is_none() {
        return None;
    }
    let DaemonError::ProviderProtocol { operation, .. } = error else {
        return None;
    };
    if *operation != "codex_thread_resume" {
        return None;
    }
    Some(run.resume_state().without_codex_thread_id())
}

fn normalize_resume_model_for_adapter(adapter_key: &str, model: &str) -> String {
    let trimmed = model.trim();
    if adapter_key == "codex" {
        trimmed
            .strip_prefix("codex/")
            .unwrap_or(trimmed)
            .to_string()
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn generate_runtime_mcp_auth_token() -> String {
    Alphanumeric.sample_string(&mut rand::thread_rng(), 32)
}

#[cfg(test)]
mod tests {
    use crate::agent::{AgentInstance, GridPosition};
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::config::{
        DaemonConfig, UserCredentialConfig, UserCredentialInjectionConfig,
        UserCredentialSourceConfig, UserCredentialUse,
    };
    use crate::provider::{LaunchProviderRequest, ProviderResumeState, ProviderRunState};
    use crate::session::{CreateSessionRequest, SessionAgentDefaults};

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
    fn sanitize_resume_state_keeps_codex_resume_when_request_model_is_unprefixed() {
        let mut agent = AgentInstance::new(
            "agent-1",
            "agent-1",
            "session-1",
            None,
            "codex",
            Some("codex/gpt-5.5".to_string()),
            Some("high".to_string()),
            None,
            GridPosition::new(0, 0, 1, 1),
        );
        let resume_state = ProviderResumeState::from_codex_thread_id("thread-1");
        agent.set_provider_resume_state(resume_state.clone());
        let request =
            LaunchProviderRequest::new("session-1", "codex", "codex", "default", "gpt-5.5")
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
    fn codex_resume_failure_replacement_clears_only_codex_thread() {
        let mut resume_state = ProviderResumeState::from_codex_thread_id("thread-1");
        resume_state.set_opencode_session_id("open-session-1");
        let request =
            LaunchProviderRequest::new("session-1", "codex", "codex", "default", "gpt-5.5")
                .with_agent_id("agent-1")
                .with_resume_state(resume_state);
        let run = RuntimeProviderRun::new(
            "provider-run-1",
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::Managed,
                process_label: "codex".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: Default::default(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: Some("ws://127.0.0.1:43123".to_string()),
            },
        );
        let error = DaemonError::ProviderProtocol {
            provider_run_id: run.id().to_string(),
            operation: "codex_thread_resume",
            message: "Codex could not resume thread `thread-1`: no rollout found".to_string(),
        };

        let replacement = failed_codex_resume_state_replacement(&run, &error)
            .expect("failed Codex resume should clear the stale thread id");

        assert_eq!(replacement.codex_thread_id(), None);
        assert_eq!(replacement.opencode_session_id(), Some("open-session-1"));
    }

    #[test]
    fn prompt_auto_launch_uses_agent_owner_and_resume_state() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(
                CreateSessionRequest::new("workspace-1", "worktree-1")
                    .with_owner_user_id("cloud-user")
                    .with_agent_defaults(
                        SessionAgentDefaults::new("dev-stub").with_model("sonnet"),
                    ),
            )
            .expect("session create should succeed");
        let resume_state = ProviderResumeState::from_codex_thread_id("codex-thread-1");
        app.agents
            .set_agent_runtime_profile(
                agent.id(),
                "dev-stub",
                Some("sonnet".to_string()),
                None,
                resume_state.clone(),
            )
            .expect("agent resume state should update");

        let run_id = app
            .ensure_prompt_provider_run_for_agent(session.id(), agent.id())
            .expect("prompt auto-launch should create a provider run");
        let run = app
            .providers()
            .get_run(&run_id)
            .expect("provider run should exist");

        assert_eq!(run.owner_user_id(), "cloud-user");
        assert_eq!(run.resume_state(), &resume_state);
    }

    #[test]
    fn prompt_auto_launch_failure_does_not_leave_running_provider_run() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(
                CreateSessionRequest::new("workspace-1", "worktree-1").with_agent_defaults(
                    SessionAgentDefaults::new("dev-stub").with_model("sonnet"),
                ),
            )
            .expect("session create should succeed");

        let result = app.launch_provider_detached(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "runtime-init-fail",
                "default",
                "sonnet",
            )
            .with_agent_id(agent.id().to_string()),
        );

        assert!(result.is_err());
        let run = app
            .providers()
            .get_latest_run_for_agent(session.id(), agent.id())
            .expect("failed launch should still leave an ended run record");
        assert_eq!(run.state(), ProviderRunState::Ended);
        assert!(app
            .sessions()
            .get_session(session.id())
            .expect("session should resolve")
            .active_provider_run_id()
            .is_none());
        assert!(app
            .list_provider_processes(None)
            .expect("provider processes should list")
            .is_empty());
    }

    #[test]
    fn provider_processes_list_and_teardown_safe_idle_managed_runs() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
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
        assert_eq!(app.provider_process_tracking.snapshot().processes.len(), 1);
        assert_eq!(
            app.provider_process_tracking.snapshot().run_processes.len(),
            1
        );
        assert_eq!(
            processes[0].pid,
            app.pty
                .process_id(run.id())
                .expect("pty pid should resolve")
        );

        let torn_down = app
            .teardown_provider_processes(None, false)
            .expect("safe teardown should succeed");
        assert_eq!(torn_down.len(), 1);
        assert!(app
            .list_provider_processes(None)
            .expect("provider processes should relist")
            .is_empty());
        assert!(app
            .provider_process_tracking
            .snapshot()
            .processes
            .is_empty());
        assert!(app
            .provider_process_tracking
            .snapshot()
            .run_processes
            .is_empty());
    }

    #[test]
    fn provider_launch_runtime_profile_survives_kernel_restart() {
        let config = DaemonConfig::for_tests();
        let (agent_id, run_model) = {
            let mut app =
                DaemonApp::bootstrap(config.clone()).expect("daemon bootstrap should succeed");
            let (session, agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
                .expect("session create should succeed");
            let run = app
                .launch_provider(
                    LaunchProviderRequest::new(
                        session.id(),
                        "dev-stub",
                        "claude-code",
                        "default",
                        "sonnet",
                    )
                    .with_agent_id(agent.id()),
                )
                .expect("provider launch should succeed");
            (agent.id().to_string(), run.model().to_string())
        };

        let app =
            DaemonApp::bootstrap(config).expect("daemon bootstrap after restart should succeed");
        let restored_agent = app
            .agents
            .get_agent(&agent_id)
            .expect("agent should restore");
        assert_eq!(restored_agent.provider(), "claude-code");
        assert_eq!(restored_agent.model(), Some(run_model.as_str()));
    }

    #[test]
    fn provider_launch_scrubs_configured_credential_env_names() {
        let mut config = DaemonConfig::for_tests();
        config.user_config.credentials.push(UserCredentialConfig {
            id: "github".to_string(),
            description: None,
            source: UserCredentialSourceConfig::Env {
                name: "ARROBA_TEST_GH_TOKEN".to_string(),
            },
            allowed_hosts: Vec::new(),
            allowed_uses: vec![UserCredentialUse::Http],
            injection: UserCredentialInjectionConfig::Header {
                name: "authorization".to_string(),
                value: "Bearer ${secret}".to_string(),
            },
        });
        let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
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

        assert!(run
            .pty_env_remove()
            .contains(&"ARROBA_TEST_GH_TOKEN".to_string()));
    }

    #[test]
    fn provider_processes_do_not_teardown_when_session_is_attached() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session create should succeed");
        let _attachment = crate::app::KernelSessionService::new(&mut app)
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
            .teardown_provider_processes(None, false)
            .expect("safe teardown should succeed");
        assert!(torn_down.is_empty());
        assert_eq!(
            app.providers()
                .get_run(run.id())
                .expect("run should still exist")
                .state(),
            crate::provider::ProviderRunState::Running,
        );

        let torn_down = app
            .teardown_provider_processes(None, true)
            .expect("forced teardown should succeed without active prompts");
        assert_eq!(torn_down.len(), 1);
        assert!(app
            .list_provider_processes(None)
            .expect("provider processes should relist")
            .is_empty());
    }

    #[test]
    fn ending_session_clears_tracked_provider_processes() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
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
            .provider_process_tracking
            .snapshot()
            .processes
            .values()
            .any(|process| { process.owner_provider_run_ids == vec![run.id().to_string()] }));

        let _ = crate::app::KernelSessionService::new(&mut app)
            .end_session(session.id())
            .expect("session should end");

        assert!(app
            .provider_process_tracking
            .snapshot()
            .processes
            .is_empty());
        assert!(app
            .provider_process_tracking
            .snapshot()
            .run_processes
            .is_empty());
        assert!(app
            .list_provider_processes(None)
            .expect("provider processes should list")
            .is_empty());
    }
}
