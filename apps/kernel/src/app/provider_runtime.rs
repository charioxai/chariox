use crate::agent::AgentInstance;
use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::{
    AgentEndpointMode, LaunchProviderRequest, ProviderProcessService, ProviderRuntimeBinding,
    RuntimeProviderRun,
};

use super::provider_activation::ProviderRunActivationState;
pub(crate) use super::provider_activation::StartedProviderLaunch;
use super::provider_launch_policy::failed_provider_resume_state_replacement;
use super::provider_liveness::clear_active_provider_run_session_pointer;
pub(crate) use super::provider_liveness::ProviderRunLivenessRuntime;
pub(crate) use super::provider_processes::ProviderProcessTracker;

impl DaemonApp {
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
                    clear_active_provider_run_session_pointer(
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
        if let Some(agent) = self.clear_failed_provider_resume_state(started, error) {
            let _ = self.durable_state_store().append_event(
                "agent.runtime_profile_updated",
                Some(agent.id().to_string()),
                serde_json::json!({
                    "agent": &agent,
                    "provider_run_id": started.run.id(),
                    "reason": "failed_provider_resume_state_cleared",
                }),
            );
        }
        if let Some(agent_id) = started.run.agent_instance_id() {
            if let Ok(Some(active_prompt)) =
                self.prompt_owner_active_prompt_for_agent(started.run.session_id(), agent_id)
            {
                if active_prompt.durable_delivery_phase().is_none() {
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
        }
        let _ = ProviderProcessTracker::new(self).remove_run(started.run.id());
        self.providers.clear_runtime(started.run.id());
        if let Ok(outcome) = self
            .providers
            .terminate_run_provider_only(started.run.session_id(), started.run.id())
        {
            clear_active_provider_run_session_pointer(
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

    fn clear_failed_provider_resume_state(
        &mut self,
        started: &StartedProviderLaunch,
        error: &DaemonError,
    ) -> Option<AgentInstance> {
        let replacement_resume_state =
            failed_provider_resume_state_replacement(&started.run, error)?;
        let agent_id = started.run.agent_instance_id()?;
        let provider = started.run.adapter_key();
        let stale_provider_session_id = started
            .run
            .resume_state()
            .provider_session_id(provider)?
            .to_string();
        let current = self.agents.get_agent(agent_id).ok()?;
        if current
            .provider_resume_state()
            .provider_session_id(provider)
            != Some(stale_provider_session_id.as_str())
        {
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
            crate::provider::provider_resume_failure_notice(provider, &stale_provider_session_id)
                .unwrap_or_else(|| {
                    format!(
                        "Provider session `{stale_provider_session_id}` is no longer available. Arroba cleared it from the agent profile so the next prompt can start a new durable provider session."
                    )
                }),
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
            let agent = self.agents.set_agent_runtime_profile_with_account_profile(
                agent_id,
                run.provider(),
                Some(run.model().to_string()),
                run.variant().map(str::to_string),
                Some(run.account_profile().to_string()),
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
        let prepared =
            self.prepare_app_provider_launch_request(request.clone(), "launch provider run")?;
        if let Some(run) =
            ProviderRunActivationState::reusable_native_tui_run_for_launch(self, &prepared)?
        {
            return Ok(run);
        }
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
        self.update_provider_run_projection(run.clone());
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
            let agent = self.agents.set_agent_runtime_profile_with_account_profile(
                agent_id,
                run.provider(),
                Some(run.model().to_string()),
                run.variant().map(str::to_string),
                Some(run.account_profile().to_string()),
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
}

#[cfg(test)]
mod tests;
