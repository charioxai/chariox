use crate::error::DaemonError;
use crate::session::PromptAttachment;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use super::{
    codex_runtime::{initialize_codex_runtime, CodexRuntimeBinding},
    opencode_binding::{initialize_opencode_runtime, OpenCodeRunSelection, OpenCodeRuntimeBinding},
    LaunchProviderRequest, ProviderPromptSignalBatch, ProviderRegistry, ProviderRunActorMailbox,
    ProviderRunOperationLanes, ProviderRunState, RuntimeProviderRun,
};

pub struct ProviderProcessService {
    registry: ProviderRegistry,
    run_actor_mailbox: ProviderRunActorMailbox,
    runs: BTreeMap<String, RuntimeProviderRun>,
    next_run_number: u64,
}

#[derive(Clone)]
pub struct ProviderProcessServiceStore {
    inner: Arc<Mutex<ProviderProcessService>>,
}

impl std::fmt::Debug for ProviderProcessServiceStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderProcessServiceStore")
            .finish_non_exhaustive()
    }
}

impl ProviderProcessServiceStore {
    pub fn new(service: ProviderProcessService) -> Self {
        Self {
            inner: Arc::new(Mutex::new(service)),
        }
    }

    pub fn read(&self) -> MutexGuard<'_, ProviderProcessService> {
        self.inner.lock().expect("provider service mutex poisoned")
    }

    pub fn write(&self) -> MutexGuard<'_, ProviderProcessService> {
        self.inner.lock().expect("provider service mutex poisoned")
    }

    pub fn registry(&self) -> ProviderRegistry {
        *self.read().registry()
    }

    pub(crate) fn run_operation_lanes(&self) -> ProviderRunOperationLanes {
        self.read().run_operation_lanes()
    }

    pub fn get_run(&self, run_id: &str) -> Result<RuntimeProviderRun, DaemonError> {
        self.read().get_run(run_id)
    }

    #[doc(hidden)]
    pub fn structured_runtime_state_bound_for_tests(&self, provider_run_id: &str) -> bool {
        self.read()
            .structured_runtime_state_bound_for_tests(provider_run_id)
    }

    pub(crate) fn start_run_provider_only(
        &self,
        request: LaunchProviderRequest,
    ) -> Result<ProviderRunStartedOutcome, DaemonError> {
        self.write().start_run_provider_only(request)
    }

    pub fn launch_run_detached(
        &self,
        request: LaunchProviderRequest,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        self.write().launch_run_detached(request)
    }

    pub(crate) fn park_run_provider_only(
        &self,
        session_id: &str,
        run_id: &str,
    ) -> Result<ProviderRunParkedOutcome, DaemonError> {
        self.write().park_run_provider_only(session_id, run_id)
    }

    pub(crate) fn resume_run_provider_only(
        &self,
        session_id: &str,
        run_id: &str,
    ) -> Result<ProviderRunResumedOutcome, DaemonError> {
        self.write().resume_run_provider_only(session_id, run_id)
    }

    pub fn resume_run_detached(&self, run_id: &str) -> Result<RuntimeProviderRun, DaemonError> {
        self.write().resume_run_detached(run_id)
    }

    pub(crate) fn terminate_run_provider_only(
        &self,
        session_id: &str,
        run_id: &str,
    ) -> Result<ProviderRunEndedOutcome, DaemonError> {
        self.write().terminate_run_provider_only(session_id, run_id)
    }

    pub fn list_runs(&self) -> Vec<RuntimeProviderRun> {
        self.read().list_runs()
    }

    pub fn get_run_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Option<RuntimeProviderRun> {
        self.read().get_run_for_agent(session_id, agent_id)
    }

    pub fn get_latest_run_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Option<RuntimeProviderRun> {
        self.read().get_latest_run_for_agent(session_id, agent_id)
    }

    pub fn get_session_run_for_provider(
        &self,
        session_id: &str,
        provider: &str,
    ) -> Option<RuntimeProviderRun> {
        self.read()
            .get_session_run_for_provider(session_id, provider)
    }

    pub fn get_run_by_runtime_mcp_auth_token(
        &self,
        auth_token: &str,
    ) -> Option<RuntimeProviderRun> {
        self.read().get_run_by_runtime_mcp_auth_token(auth_token)
    }

    pub fn get_runs_by_runtime_mcp_auth_token(&self, auth_token: &str) -> Vec<RuntimeProviderRun> {
        self.read().get_runs_by_runtime_mcp_auth_token(auth_token)
    }

    pub(crate) fn structured_prompt_io_in_flight(&self, provider_run_id: &str) -> bool {
        self.read().structured_prompt_io_in_flight(provider_run_id)
    }

    pub fn record_run_activity(&self, run_id: &str) -> Result<(), DaemonError> {
        self.write().record_run_activity(run_id)
    }

    pub(crate) fn mark_run_running(&self, run_id: &str) -> Result<RuntimeProviderRun, DaemonError> {
        self.write().mark_run_running(run_id)
    }

    pub(crate) fn reconcile_run_liveness_provider_only(
        &self,
        session_id: &str,
        run_id: &str,
        process_running: Option<bool>,
    ) -> Result<ProviderRunLivenessReconciliation, DaemonError> {
        self.write()
            .reconcile_run_liveness_provider_only(session_id, run_id, process_running)
    }

    pub(crate) fn terminate_session_runs_provider_only(
        &self,
        session_id: &str,
    ) -> Result<ProviderSessionRunsTerminatedOutcome, DaemonError> {
        self.write()
            .terminate_session_runs_provider_only(session_id)
    }

    pub fn initialize_runtime(&self, run: &RuntimeProviderRun) -> Result<(), DaemonError> {
        self.write().initialize_runtime(run)
    }

    pub(crate) fn apply_runtime_binding(
        &self,
        run_id: &str,
        binding: ProviderRuntimeBinding,
    ) -> Result<(), DaemonError> {
        self.write().apply_runtime_binding(run_id, binding)
    }

    pub(crate) fn run_uses_structured_prompt_io(&self, run: &RuntimeProviderRun) -> bool {
        self.read().run_uses_structured_prompt_io(run)
    }

    pub fn enqueue_run_selection_sync(&self, provider_run_id: &str) -> Result<(), DaemonError> {
        self.write().enqueue_run_selection_sync(provider_run_id)
    }

    pub(crate) fn apply_finished_provider_run_selection_sync_jobs(&self) {
        self.write()
            .apply_finished_provider_run_selection_sync_jobs()
    }

    pub fn clear_runtime(&self, provider_run_id: &str) {
        self.write().clear_runtime(provider_run_id)
    }

    pub(crate) fn enqueue_structured_prompt_submit(
        &self,
        session_id: String,
        provider_run_id: String,
        agent_id: String,
        run: &RuntimeProviderRun,
        prompt: &str,
        attachments: &[PromptAttachment],
    ) -> Result<(), DaemonError> {
        self.write().enqueue_structured_prompt_submit(
            session_id,
            provider_run_id,
            agent_id,
            run,
            prompt,
            attachments,
        )
    }

    pub(crate) fn enqueue_structured_prompt_abort(
        &self,
        session_id: String,
        provider_run_id: String,
    ) -> Result<(), DaemonError> {
        self.write()
            .enqueue_structured_prompt_abort(session_id, provider_run_id)
    }

    pub(crate) fn drain_finished_structured_prompt_submit_jobs(
        &self,
    ) -> Vec<super::FinishedProviderPromptSubmitJob> {
        self.write().drain_finished_structured_prompt_submit_jobs()
    }

    pub(crate) fn drain_finished_structured_prompt_abort_jobs(
        &self,
    ) -> Vec<super::FinishedProviderPromptAbortJob> {
        self.write().drain_finished_structured_prompt_abort_jobs()
    }

    pub fn enqueue_structured_output_poll(
        &self,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        self.write().enqueue_structured_output_poll(provider_run_id)
    }

    pub fn set_output_poll_delay_for_tests(
        &self,
        provider_run_id: &str,
        delay: std::time::Duration,
    ) {
        self.read()
            .set_output_poll_delay_for_tests(provider_run_id, delay);
    }

    pub(crate) fn drain_finished_structured_output_poll_jobs(
        &self,
    ) -> Vec<super::FinishedProviderOutputPollJob> {
        self.write().drain_finished_structured_output_poll_jobs()
    }

    pub(crate) fn apply_structured_output_metadata(
        &self,
        provider_run_id: &str,
        batch: &ProviderPromptSignalBatch,
    ) -> Result<(), DaemonError> {
        self.write()
            .apply_structured_output_metadata(provider_run_id, batch)
    }
}

pub(crate) enum ProviderRuntimeBinding {
    Codex(CodexRuntimeBinding),
    OpenCode(OpenCodeRuntimeBinding),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderRunLivenessReconciliation {
    AlreadyEnded(RuntimeProviderRun),
    ExternalEndpoint(RuntimeProviderRun),
    StillRunning(RuntimeProviderRun),
    NewlyEnded(RuntimeProviderRun),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderRunEndedOutcome {
    run: RuntimeProviderRun,
    already_ended: bool,
}

impl ProviderRunEndedOutcome {
    pub(crate) fn run(&self) -> &RuntimeProviderRun {
        &self.run
    }

    pub(crate) fn into_run(self) -> RuntimeProviderRun {
        self.run
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderSessionRunsTerminatedOutcome {
    runs: Vec<ProviderRunEndedOutcome>,
}

impl ProviderSessionRunsTerminatedOutcome {
    pub(crate) fn runs(&self) -> &[ProviderRunEndedOutcome] {
        &self.runs
    }

    pub(crate) fn into_runs(self) -> Vec<ProviderRunEndedOutcome> {
        self.runs
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderRunStartedOutcome {
    run: RuntimeProviderRun,
}

impl ProviderRunStartedOutcome {
    pub(crate) fn run(&self) -> &RuntimeProviderRun {
        &self.run
    }

    pub(crate) fn into_run(self) -> RuntimeProviderRun {
        self.run
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderRunParkedOutcome {
    run: RuntimeProviderRun,
}

impl ProviderRunParkedOutcome {
    pub(crate) fn run(&self) -> &RuntimeProviderRun {
        &self.run
    }

    pub(crate) fn into_run(self) -> RuntimeProviderRun {
        self.run
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderRunResumedOutcome {
    run: RuntimeProviderRun,
}

impl ProviderRunResumedOutcome {
    pub(crate) fn run(&self) -> &RuntimeProviderRun {
        &self.run
    }

    pub(crate) fn into_run(self) -> RuntimeProviderRun {
        self.run
    }
}

impl ProviderProcessService {
    pub fn new() -> Self {
        Self {
            registry: ProviderRegistry::new(),
            run_actor_mailbox: ProviderRunActorMailbox::default(),
            runs: BTreeMap::new(),
            next_run_number: 0,
        }
    }

    pub fn registry(&self) -> &ProviderRegistry {
        &self.registry
    }

    pub(crate) fn start_run_provider_only(
        &mut self,
        request: LaunchProviderRequest,
    ) -> Result<ProviderRunStartedOutcome, DaemonError> {
        let adapter = self.registry.resolve(&request.adapter_key).ok_or_else(|| {
            DaemonError::ProviderAdapterNotFound {
                adapter_key: request.adapter_key.clone(),
            }
        })?;
        if request.requires_managed_io() && !adapter.supports_managed_io_write_enforcement() {
            return Err(DaemonError::ProviderManagedIoUnsupported {
                adapter_key: request.adapter_key.clone(),
                message: "this adapter cannot guarantee that provider-session writes are restricted to Arroba managed I/O tools".to_string(),
            });
        }

        let run_id = self.next_run_id();
        let launch_result = adapter.connect(&request)?;
        crate::logging::info_with_fields(
            "daemon.provider",
            "provider launch planned",
            serde_json::json!({
                "provider_run_id": run_id,
                "session_id": request.session_id.as_str(),
                "agent_id": request.agent_id.as_deref(),
                "adapter_key": request.adapter_key.as_str(),
                "provider": request.provider.as_str(),
                "model": request.model.as_str(),
                "variant": request.variant.as_deref(),
                "requires_managed_io": request.requires_managed_io(),
                "runtime_mcp_binding_present": request.runtime_mcp_binding.is_some(),
                "granted_mcp_servers": request
                    .mcp_servers
                    .iter()
                    .map(|server| server.name.as_str())
                    .collect::<Vec<_>>(),
                "endpoint_mode": launch_result.endpoint_mode.to_string(),
                "process_label": launch_result.process_label.as_str(),
                "structured_endpoint": launch_result.structured_endpoint.as_deref(),
                "pty_env_keys": launch_result.pty_env.keys().cloned().collect::<Vec<_>>(),
            }),
        );
        let run = RuntimeProviderRun::new(run_id.clone(), &request, launch_result);

        self.runs.insert(run_id, run.clone());

        Ok(ProviderRunStartedOutcome { run })
    }

    pub fn launch_run_detached(
        &mut self,
        request: LaunchProviderRequest,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let outcome = self.start_run_provider_only(request)?;
        self.mark_run_running(outcome.run().id())
    }

    pub(crate) fn park_run_provider_only(
        &mut self,
        session_id: &str,
        run_id: &str,
    ) -> Result<ProviderRunParkedOutcome, DaemonError> {
        let run_snapshot = self.get_run(run_id)?;

        if run_snapshot.session_id() != session_id {
            return Err(DaemonError::ProviderRunNotInSession {
                session_id: session_id.to_string(),
                provider_run_id: run_id.to_string(),
            });
        }

        if run_snapshot.state() != ProviderRunState::Running {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: run_id.to_string(),
                state: run_snapshot.state(),
                operation: "park",
            });
        }

        let adapter = self.adapter_for(run_snapshot.adapter_key())?;
        adapter.park(&run_snapshot);

        let run = self.get_run_mut(run_id)?;
        run.mark_parked();

        Ok(ProviderRunParkedOutcome { run: run.clone() })
    }

    pub(crate) fn resume_run_provider_only(
        &mut self,
        session_id: &str,
        run_id: &str,
    ) -> Result<ProviderRunResumedOutcome, DaemonError> {
        let run_snapshot = self.get_run(run_id)?;

        if run_snapshot.session_id() != session_id {
            return Err(DaemonError::ProviderRunNotInSession {
                session_id: session_id.to_string(),
                provider_run_id: run_id.to_string(),
            });
        }

        if run_snapshot.state() != ProviderRunState::Parked {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: run_id.to_string(),
                state: run_snapshot.state(),
                operation: "resume",
            });
        }

        let adapter = self.adapter_for(run_snapshot.adapter_key())?;
        adapter.resume(&run_snapshot);

        let run = self.get_run_mut(run_id)?;
        run.mark_running();

        Ok(ProviderRunResumedOutcome { run: run.clone() })
    }

    pub fn resume_run_detached(&mut self, run_id: &str) -> Result<RuntimeProviderRun, DaemonError> {
        let run_snapshot = self.get_run(run_id)?;

        if run_snapshot.state() != ProviderRunState::Parked {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: run_id.to_string(),
                state: run_snapshot.state(),
                operation: "resume",
            });
        }

        let adapter = self.adapter_for(run_snapshot.adapter_key())?;
        adapter.resume(&run_snapshot);

        let run = self.get_run_mut(run_id)?;
        run.mark_running();

        Ok(run.clone())
    }

    pub(crate) fn terminate_run_provider_only(
        &mut self,
        session_id: &str,
        run_id: &str,
    ) -> Result<ProviderRunEndedOutcome, DaemonError> {
        let run_snapshot = self.get_run(run_id)?;

        if run_snapshot.session_id() != session_id {
            return Err(DaemonError::ProviderRunNotInSession {
                session_id: session_id.to_string(),
                provider_run_id: run_id.to_string(),
            });
        }

        if run_snapshot.state() == ProviderRunState::Ended {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: run_id.to_string(),
                state: run_snapshot.state(),
                operation: "terminate",
            });
        }

        let adapter = self.adapter_for(run_snapshot.adapter_key())?;
        adapter.terminate(&run_snapshot);

        let run = self.get_run_mut(run_id)?;
        run.mark_ended();
        let run = run.clone();

        self.run_actor_mailbox
            .spawn_terminate(run_id.to_string(), run.clone());

        Ok(ProviderRunEndedOutcome {
            run,
            already_ended: false,
        })
    }

    pub fn get_run(&self, run_id: &str) -> Result<RuntimeProviderRun, DaemonError> {
        self.runs
            .get(run_id)
            .cloned()
            .ok_or_else(|| DaemonError::ProviderRunNotFound {
                provider_run_id: run_id.to_string(),
            })
    }

    pub fn list_runs(&self) -> Vec<RuntimeProviderRun> {
        self.runs.values().cloned().collect()
    }

    pub fn record_run_activity(&mut self, run_id: &str) -> Result<(), DaemonError> {
        let run = self.get_run_mut(run_id)?;
        run.touch_activity();
        Ok(())
    }

    pub(crate) fn mark_run_running(
        &mut self,
        run_id: &str,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let run = self.get_run_mut(run_id)?;
        if run.state() != ProviderRunState::Starting {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: run_id.to_string(),
                state: run.state(),
                operation: "finish launch",
            });
        }
        run.mark_running();
        Ok(run.clone())
    }

    pub(crate) fn reconcile_run_liveness_provider_only(
        &mut self,
        session_id: &str,
        run_id: &str,
        process_running: Option<bool>,
    ) -> Result<ProviderRunLivenessReconciliation, DaemonError> {
        let run_snapshot = self.get_run(run_id)?;
        if run_snapshot.session_id() != session_id {
            return Err(DaemonError::ProviderRunNotInSession {
                session_id: session_id.to_string(),
                provider_run_id: run_id.to_string(),
            });
        }

        if run_snapshot.state() == ProviderRunState::Ended {
            self.clear_runtime(run_id);
            return Ok(ProviderRunLivenessReconciliation::AlreadyEnded(
                run_snapshot,
            ));
        }

        if run_snapshot.endpoint_mode() == crate::provider::AgentEndpointMode::External {
            return Ok(ProviderRunLivenessReconciliation::ExternalEndpoint(
                run_snapshot,
            ));
        }

        let Some(process_running) = process_running else {
            return Ok(ProviderRunLivenessReconciliation::StillRunning(
                run_snapshot,
            ));
        };

        if process_running {
            return Ok(ProviderRunLivenessReconciliation::StillRunning(
                run_snapshot,
            ));
        }

        let ended = self
            .mark_run_ended_provider_only(session_id, run_id)?
            .into_run();
        Ok(ProviderRunLivenessReconciliation::NewlyEnded(ended))
    }

    pub fn get_run_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Option<RuntimeProviderRun> {
        self.runs
            .values()
            .filter(|run| {
                run.session_id() == session_id
                    && run.agent_instance_id() == Some(agent_id)
                    && run.state() != ProviderRunState::Ended
            })
            .max_by_key(|run| match run.state() {
                ProviderRunState::Running => 3,
                ProviderRunState::Parked => 2,
                ProviderRunState::Starting => 1,
                ProviderRunState::Ended => 0,
            })
            .cloned()
    }

    pub fn get_latest_run_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Option<RuntimeProviderRun> {
        self.runs
            .values()
            .filter(|run| {
                run.session_id() == session_id && run.agent_instance_id() == Some(agent_id)
            })
            .max_by_key(|run| (run.last_activity_at_ms(), run.started_at_ms()))
            .cloned()
    }

    pub fn get_session_run_for_provider(
        &self,
        session_id: &str,
        provider: &str,
    ) -> Option<RuntimeProviderRun> {
        self.runs
            .values()
            .filter(|run| {
                run.session_id() == session_id
                    && run.provider() == provider
                    && run.state() != ProviderRunState::Ended
            })
            .max_by_key(|run| match run.state() {
                ProviderRunState::Running => 3,
                ProviderRunState::Parked => 2,
                ProviderRunState::Starting => 1,
                ProviderRunState::Ended => 0,
            })
            .cloned()
    }

    pub fn get_run_by_runtime_mcp_auth_token(
        &self,
        auth_token: &str,
    ) -> Option<RuntimeProviderRun> {
        self.runs
            .values()
            .find(|run| run.runtime_mcp_auth_token() == Some(auth_token))
            .cloned()
    }

    pub fn get_runs_by_runtime_mcp_auth_token(&self, auth_token: &str) -> Vec<RuntimeProviderRun> {
        self.runs
            .values()
            .filter(|run| {
                run.runtime_mcp_auth_token() == Some(auth_token)
                    && run.state() != ProviderRunState::Ended
            })
            .cloned()
            .collect()
    }

    pub(crate) fn terminate_session_runs_provider_only(
        &mut self,
        session_id: &str,
    ) -> Result<ProviderSessionRunsTerminatedOutcome, DaemonError> {
        let run_ids: Vec<String> = self
            .runs
            .values()
            .filter(|run| run.session_id() == session_id && run.state() != ProviderRunState::Ended)
            .map(|run| run.id().to_string())
            .collect();

        let mut terminated_runs = Vec::with_capacity(run_ids.len());

        for run_id in run_ids {
            let outcome = self.terminate_run_provider_only(session_id, &run_id)?;
            terminated_runs.push(outcome);
        }

        Ok(ProviderSessionRunsTerminatedOutcome {
            runs: terminated_runs,
        })
    }

    pub fn initialize_runtime(&mut self, run: &RuntimeProviderRun) -> Result<(), DaemonError> {
        if let Some(binding) = Self::initialize_runtime_binding(run)? {
            self.apply_runtime_binding(run.id(), binding)?;
        }
        Ok(())
    }

    pub(crate) fn initialize_runtime_binding(
        run: &RuntimeProviderRun,
    ) -> Result<Option<ProviderRuntimeBinding>, DaemonError> {
        if run.adapter_key() == "dev-stub" && run.provider() == "runtime-init-fail" {
            return Err(DaemonError::ProviderProtocol {
                provider_run_id: run.id().to_string(),
                operation: "dev_stub_runtime_init",
                message: "forced dev-stub runtime initialization failure".to_string(),
            });
        }
        if run.adapter_key() == "codex" {
            return initialize_codex_runtime(run)
                .map(ProviderRuntimeBinding::Codex)
                .map(Some);
        }
        if run.adapter_key() == "opencode" {
            return initialize_opencode_runtime(run)
                .map(ProviderRuntimeBinding::OpenCode)
                .map(Some);
        }
        Ok(None)
    }

    pub(crate) fn apply_runtime_binding(
        &mut self,
        run_id: &str,
        binding: ProviderRuntimeBinding,
    ) -> Result<(), DaemonError> {
        match binding {
            ProviderRuntimeBinding::Codex(binding) => {
                self.run_actor_mailbox
                    .insert_codex_runtime(run_id.to_string(), binding.state);
                let run_mut = self.get_run_mut(run_id)?;
                run_mut.set_resume_state(binding.resume_state.clone());
                run_mut.set_provider_session_id(
                    binding.resume_state.codex_thread_id().map(str::to_string),
                );
                self.apply_codex_run_selection(run_id, binding.selection)?;
            }
            ProviderRuntimeBinding::OpenCode(binding) => {
                self.run_actor_mailbox
                    .insert_opencode_runtime(run_id.to_string(), binding.state);
                let run_mut = self.get_run_mut(run_id)?;
                run_mut.set_resume_state(binding.resume_state.clone());
                run_mut.set_provider_session_id(
                    binding
                        .resume_state
                        .opencode_session_id()
                        .map(str::to_string),
                );
                self.apply_opencode_run_selection(run_id, binding.selection)?;
            }
        }
        Ok(())
    }

    pub(crate) fn run_uses_structured_prompt_io(&self, run: &RuntimeProviderRun) -> bool {
        run.adapter_key() == "codex"
            || run.adapter_key() == "opencode"
            || (run.adapter_key() == "dev-stub" && run.provider() == "slow-structured")
    }

    pub(crate) fn structured_prompt_io_in_flight(&self, provider_run_id: &str) -> bool {
        self.run_actor_mailbox
            .structured_prompt_io_in_flight(provider_run_id)
    }

    #[doc(hidden)]
    pub fn structured_runtime_state_bound_for_tests(&self, provider_run_id: &str) -> bool {
        self.run_actor_mailbox
            .structured_runtime_state_bound(provider_run_id)
    }

    pub(crate) fn run_operation_lanes(&self) -> ProviderRunOperationLanes {
        self.run_actor_mailbox.operation_lanes()
    }

    pub fn enqueue_run_selection_sync(&mut self, provider_run_id: &str) -> Result<(), DaemonError> {
        let run = self.get_run(provider_run_id)?;
        if run.adapter_key() != "opencode" {
            return Ok(());
        }
        self.run_actor_mailbox
            .spawn_selection_sync(provider_run_id.to_string())
    }

    pub(crate) fn apply_finished_provider_run_selection_sync_jobs(&mut self) {
        for finished in self.run_actor_mailbox.drain_finished_selection_syncs() {
            match finished.result {
                Ok(selection) => {
                    let Ok(current_run) = self.get_run(&finished.provider_run_id) else {
                        continue;
                    };
                    let selection = Self::merge_opencode_run_selection(&current_run, selection);
                    if let Err(error) =
                        self.apply_opencode_run_selection(&finished.provider_run_id, selection)
                    {
                        crate::logging::error_with_fields(
                            "daemon.provider",
                            "provider run selection sync apply failed",
                            serde_json::json!({
                                "provider_run_id": finished.provider_run_id,
                                "error": error.to_string(),
                            }),
                        );
                    }
                }
                Err(error) => {
                    crate::logging::debug_with_fields(
                        "daemon.provider",
                        "provider run selection sync failed",
                        serde_json::json!({
                            "provider_run_id": finished.provider_run_id,
                            "error": error.to_string(),
                        }),
                    );
                }
            }
        }
    }

    pub fn clear_runtime(&mut self, provider_run_id: &str) {
        self.run_actor_mailbox.clear_runtime(provider_run_id);
        self.run_actor_mailbox.stop_run(provider_run_id);
    }

    pub(crate) fn enqueue_structured_prompt_submit(
        &mut self,
        session_id: String,
        provider_run_id: String,
        agent_id: String,
        run: &RuntimeProviderRun,
        prompt: &str,
        attachments: &[PromptAttachment],
    ) -> Result<(), DaemonError> {
        let _ = self.record_run_activity(run.id());
        if !self.run_uses_structured_prompt_io(run) {
            return Err(DaemonError::LocalTransport {
                operation: "enqueue structured prompt dispatch",
                message: format!(
                    "provider run `{provider_run_id}` does not use structured prompt I/O"
                ),
            });
        }
        let prompt = super::managed_io_policy::apply_managed_io_instructions(prompt, run);
        self.run_actor_mailbox.spawn_submit(
            session_id,
            provider_run_id,
            agent_id,
            run.clone(),
            prompt,
            attachments.to_vec(),
        )
    }

    pub(crate) fn enqueue_structured_prompt_abort(
        &mut self,
        session_id: String,
        provider_run_id: String,
    ) -> Result<(), DaemonError> {
        let run = self.get_run(&provider_run_id)?;
        if !self.run_uses_structured_prompt_io(&run) {
            return Err(DaemonError::LocalTransport {
                operation: "enqueue structured prompt abort",
                message: format!(
                    "provider run `{provider_run_id}` does not use structured prompt I/O"
                ),
            });
        }
        self.run_actor_mailbox
            .spawn_abort(session_id, provider_run_id, run)
    }

    pub(crate) fn drain_finished_structured_prompt_submit_jobs(
        &mut self,
    ) -> Vec<super::FinishedProviderPromptSubmitJob> {
        self.run_actor_mailbox.drain_finished_submits()
    }

    pub(crate) fn drain_finished_structured_prompt_abort_jobs(
        &mut self,
    ) -> Vec<super::FinishedProviderPromptAbortJob> {
        self.run_actor_mailbox.drain_finished_aborts()
    }

    pub fn enqueue_structured_output_poll(
        &mut self,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        let _ = self.record_run_activity(provider_run_id);
        let run = self.get_run(provider_run_id)?;
        if !self.run_uses_structured_prompt_io(&run) {
            return Ok(false);
        }
        self.run_actor_mailbox
            .spawn_output_poll(provider_run_id.to_string(), run)
    }

    #[doc(hidden)]
    pub fn set_output_poll_delay_for_tests(
        &self,
        provider_run_id: &str,
        delay: std::time::Duration,
    ) {
        self.run_actor_mailbox
            .set_output_poll_delay_for_tests(provider_run_id, delay);
    }

    pub(crate) fn drain_finished_structured_output_poll_jobs(
        &mut self,
    ) -> Vec<super::FinishedProviderOutputPollJob> {
        self.run_actor_mailbox.drain_finished_output_polls()
    }

    #[cfg(test)]
    pub(crate) fn push_finished_structured_output_poll_for_test(
        &mut self,
        provider_run_id: String,
        result: Result<Option<ProviderPromptSignalBatch>, DaemonError>,
    ) {
        self.run_actor_mailbox.push_finished_output_poll_for_test(
            super::FinishedProviderOutputPollJob {
                provider_run_id,
                result,
            },
        );
    }

    pub(crate) fn apply_structured_output_metadata(
        &mut self,
        provider_run_id: &str,
        batch: &ProviderPromptSignalBatch,
    ) -> Result<(), DaemonError> {
        if batch.resolved_model.is_none()
            && batch.resolved_variant.is_none()
            && batch.resolved_usage_tokens_total.is_none()
        {
            return Ok(());
        }
        let run = self.get_run_mut(provider_run_id)?;
        if let Some(model) = batch.resolved_model.as_deref() {
            crate::logging::debug_with_fields(
                "daemon.provider.opencode",
                "resolved provider run model from opencode metadata",
                serde_json::json!({
                    "provider_run_id": provider_run_id,
                    "previous_model": run.model(),
                    "resolved_model": model,
                    "source": batch.resolved_model_source,
                }),
            );
            if run.model() != model {
                run.set_model(model.to_string());
            }
        }
        if let Some(variant) = batch.resolved_variant.as_deref() {
            if run.variant() != Some(variant) {
                crate::logging::debug_with_fields(
                    "daemon.provider.opencode",
                    "resolved provider run variant from opencode metadata",
                    serde_json::json!({
                        "provider_run_id": provider_run_id,
                        "previous_variant": run.variant(),
                        "resolved_variant": variant,
                    }),
                );
                run.set_variant(Some(variant.to_string()));
            }
        }
        if let Some(total_tokens) = batch.resolved_usage_tokens_total {
            if run.usage_tokens_total() != Some(total_tokens) {
                run.set_usage_tokens_total(Some(total_tokens));
            }
        }
        Ok(())
    }

    pub(crate) fn mark_run_ended_provider_only(
        &mut self,
        session_id: &str,
        run_id: &str,
    ) -> Result<ProviderRunEndedOutcome, DaemonError> {
        let run_snapshot = self.get_run(run_id)?;
        if run_snapshot.session_id() != session_id {
            return Err(DaemonError::ProviderRunNotInSession {
                session_id: session_id.to_string(),
                provider_run_id: run_id.to_string(),
            });
        }
        if run_snapshot.state() == ProviderRunState::Ended {
            self.clear_runtime(run_id);
            return Ok(ProviderRunEndedOutcome {
                run: run_snapshot,
                already_ended: true,
            });
        }

        let run = self.get_run_mut(run_id)?;
        run.mark_ended();
        let run = run.clone();

        self.clear_runtime(run_id);

        Ok(ProviderRunEndedOutcome {
            run,
            already_ended: false,
        })
    }

    fn get_run_mut(&mut self, run_id: &str) -> Result<&mut RuntimeProviderRun, DaemonError> {
        self.runs
            .get_mut(run_id)
            .ok_or_else(|| DaemonError::ProviderRunNotFound {
                provider_run_id: run_id.to_string(),
            })
    }

    fn adapter_for(
        &self,
        adapter_key: &str,
    ) -> Result<&'static dyn super::AgentEndpointAdapter, DaemonError> {
        self.registry
            .resolve(adapter_key)
            .ok_or_else(|| DaemonError::ProviderAdapterNotFound {
                adapter_key: adapter_key.to_string(),
            })
    }

    fn next_run_id(&mut self) -> String {
        self.next_run_number += 1;
        format!("provider-run-{}", self.next_run_number)
    }

    fn apply_opencode_run_selection(
        &mut self,
        provider_run_id: &str,
        selection: OpenCodeRunSelection,
    ) -> Result<(), DaemonError> {
        let run = self.get_run_mut(provider_run_id)?;
        if let Some(model) = selection.model {
            if run.model() != model {
                run.set_model(model);
            }
        }
        if let Some(variant) = selection.variant {
            if run.variant() != Some(variant.as_str()) {
                run.set_variant(Some(variant));
            }
        }
        Ok(())
    }

    fn apply_codex_run_selection(
        &mut self,
        provider_run_id: &str,
        selection: super::CodexRunSelection,
    ) -> Result<(), DaemonError> {
        let run = self.get_run_mut(provider_run_id)?;
        if let Some(model) = selection.model {
            if run.model() != model {
                run.set_model(model);
            }
        }
        if let Some(variant) = selection.variant {
            if run.variant() != Some(variant.as_str()) {
                run.set_variant(Some(variant));
            }
        }
        Ok(())
    }

    fn merge_opencode_run_selection(
        run: &RuntimeProviderRun,
        selection: OpenCodeRunSelection,
    ) -> OpenCodeRunSelection {
        OpenCodeRunSelection {
            model: selection.model.or_else(|| Some(run.model().to_string())),
            variant: selection
                .variant
                .or_else(|| run.variant().map(str::to_string)),
        }
    }
}

impl Default for ProviderProcessService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use crate::config::DaemonConfig;
    use crate::error::DaemonError;
    use crate::provider::opencode_binding::OpenCodeRunSelection;
    use crate::provider::{
        AgentEndpointMode, ProviderLaunchResult, ProviderResumeState, RuntimeProviderRun,
    };
    use crate::session::{CreateSessionRequest, SessionService, SessionStatus};

    use super::{
        LaunchProviderRequest, ProviderProcessService, ProviderRunLivenessReconciliation,
        ProviderRunState,
    };

    fn sessions() -> SessionService {
        SessionService::new(&DaemonConfig::for_tests())
    }

    fn launch_request(session_id: &str, model: &str) -> LaunchProviderRequest {
        LaunchProviderRequest::new(session_id, "dev-stub", "claude-code", "default", model)
    }

    fn launch_running_provider_run(
        providers: &mut ProviderProcessService,
        sessions: &mut SessionService,
        request: LaunchProviderRequest,
    ) -> RuntimeProviderRun {
        let outcome = providers
            .start_run_provider_only(request)
            .expect("provider-only start should succeed");
        let run = providers
            .mark_run_running(outcome.run().id())
            .expect("provider run should mark running");
        sessions
            .set_active_provider_run(run.session_id(), Some(run.id().to_string()))
            .expect("session active run should be set");
        run
    }

    #[test]
    fn launches_the_first_provider_run() {
        let mut sessions = sessions();
        let session = sessions
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let mut providers = ProviderProcessService::new();

        let run = launch_running_provider_run(
            &mut providers,
            &mut sessions,
            launch_request(session.id(), "sonnet"),
        );
        let session = sessions
            .get_session(session.id())
            .expect("session should exist");

        assert_eq!(run.id(), "provider-run-1");
        assert_eq!(run.state(), ProviderRunState::Running);
        assert_eq!(run.adapter_key(), "dev-stub");
        assert_eq!(session.active_provider_run_id(), Some(run.id()));
        assert_eq!(session.status(), SessionStatus::Active);
    }

    #[test]
    fn rejects_managed_io_when_adapter_cannot_enforce_writes() {
        let mut sessions = sessions();
        let session = sessions
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let mut providers = ProviderProcessService::new();

        let error = providers
            .start_run_provider_only(
                launch_request(session.id(), "sonnet").with_managed_io_required(),
            )
            .expect_err("dev-stub cannot enforce managed I/O writes");

        match error {
            DaemonError::ProviderManagedIoUnsupported { adapter_key, .. } => {
                assert_eq!(adapter_key, "dev-stub");
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn provider_only_start_run_returns_outcome_without_session_mutation() {
        let mut sessions = sessions();
        let session = sessions
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let mut providers = ProviderProcessService::new();

        let outcome = providers
            .start_run_provider_only(launch_request(session.id(), "sonnet"))
            .expect("provider-only start should succeed");

        assert_eq!(outcome.run().session_id(), session.id());
        assert_eq!(outcome.run().state(), ProviderRunState::Starting);
        assert_eq!(
            sessions
                .get_session(session.id())
                .expect("session should exist")
                .active_provider_run_id(),
            None
        );
    }

    #[test]
    fn liveness_reconciliation_without_process_observation_does_not_end_run() {
        let mut sessions = sessions();
        let session = sessions
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let mut providers = ProviderProcessService::new();
        let run = launch_running_provider_run(
            &mut providers,
            &mut sessions,
            launch_request(session.id(), "sonnet"),
        );

        let reconciliation = providers
            .reconcile_run_liveness_provider_only(session.id(), run.id(), None)
            .expect("liveness reconciliation should succeed");

        assert!(matches!(
            reconciliation,
            ProviderRunLivenessReconciliation::StillRunning(_)
        ));
        assert_eq!(
            sessions
                .get_session(session.id())
                .expect("session should exist")
                .active_provider_run_id(),
            Some(run.id())
        );
        assert_eq!(
            providers
                .get_run(run.id())
                .expect("run should still exist")
                .state(),
            ProviderRunState::Running
        );
    }

    #[test]
    fn provider_only_liveness_reconciliation_with_exited_process_marks_run_ended() {
        let mut sessions = sessions();
        let session = sessions
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let mut providers = ProviderProcessService::new();
        let run = launch_running_provider_run(
            &mut providers,
            &mut sessions,
            launch_request(session.id(), "sonnet"),
        );

        let reconciliation = providers
            .reconcile_run_liveness_provider_only(session.id(), run.id(), Some(false))
            .expect("liveness reconciliation should succeed");

        assert!(matches!(
            reconciliation,
            ProviderRunLivenessReconciliation::NewlyEnded(_)
        ));
        assert_eq!(
            sessions
                .get_session(session.id())
                .expect("session should exist")
                .active_provider_run_id(),
            Some(run.id())
        );
        assert_eq!(
            providers
                .get_run(run.id())
                .expect("run should still exist")
                .state(),
            ProviderRunState::Ended
        );
    }

    #[test]
    fn provider_only_liveness_reconciliation_handles_already_ended_without_session_mutation() {
        let mut sessions = sessions();
        let session = sessions
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let mut providers = ProviderProcessService::new();
        let run = launch_running_provider_run(
            &mut providers,
            &mut sessions,
            launch_request(session.id(), "sonnet"),
        );

        providers
            .reconcile_run_liveness_provider_only(session.id(), run.id(), Some(false))
            .expect("initial provider-only reconciliation should succeed");

        let reconciliation = providers
            .reconcile_run_liveness_provider_only(session.id(), run.id(), None)
            .expect("provider-only reconciliation should succeed");

        assert!(matches!(
            reconciliation,
            ProviderRunLivenessReconciliation::AlreadyEnded(_)
        ));
        assert_eq!(
            sessions
                .get_session(session.id())
                .expect("session should exist")
                .active_provider_run_id(),
            Some(run.id())
        );
        assert_eq!(
            providers
                .get_run(run.id())
                .expect("run should still exist")
                .state(),
            ProviderRunState::Ended
        );
    }

    #[test]
    fn provider_only_mark_run_ended_returns_outcome_without_session_mutation() {
        let mut sessions = sessions();
        let session = sessions
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let mut providers = ProviderProcessService::new();
        let run = launch_running_provider_run(
            &mut providers,
            &mut sessions,
            launch_request(session.id(), "sonnet"),
        );

        let outcome = providers
            .mark_run_ended_provider_only(session.id(), run.id())
            .expect("provider-only ending should succeed");

        assert!(!outcome.already_ended);
        assert_eq!(outcome.run().id(), run.id());
        assert_eq!(outcome.run().state(), ProviderRunState::Ended);
        assert_eq!(
            sessions
                .get_session(session.id())
                .expect("session should exist")
                .active_provider_run_id(),
            Some(run.id())
        );

        let outcome = providers
            .mark_run_ended_provider_only(session.id(), run.id())
            .expect("already-ended provider-only ending should succeed");
        assert!(outcome.already_ended);
        assert_eq!(outcome.run().id(), run.id());
    }

    #[test]
    fn provider_only_terminate_run_returns_outcome_without_session_mutation() {
        let mut sessions = sessions();
        let session = sessions
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let mut providers = ProviderProcessService::new();
        let run = launch_running_provider_run(
            &mut providers,
            &mut sessions,
            launch_request(session.id(), "sonnet"),
        );

        let outcome = providers
            .terminate_run_provider_only(session.id(), run.id())
            .expect("provider-only termination should succeed");

        assert!(!outcome.already_ended);
        assert_eq!(outcome.run().id(), run.id());
        assert_eq!(outcome.run().state(), ProviderRunState::Ended);
        assert_eq!(
            sessions
                .get_session(session.id())
                .expect("session should exist")
                .active_provider_run_id(),
            Some(run.id())
        );
    }

    #[test]
    fn provider_only_terminate_session_runs_returns_outcomes_without_session_mutation() {
        let mut sessions = sessions();
        let session = sessions
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let mut providers = ProviderProcessService::new();
        let first = launch_running_provider_run(
            &mut providers,
            &mut sessions,
            launch_request(session.id(), "sonnet"),
        );
        let second = launch_running_provider_run(
            &mut providers,
            &mut sessions,
            launch_request(session.id(), "opus"),
        );

        let outcome = providers
            .terminate_session_runs_provider_only(session.id())
            .expect("provider-only session termination should succeed");

        let terminated_run_ids = outcome
            .runs()
            .iter()
            .map(|outcome| outcome.run().id().to_string())
            .collect::<Vec<_>>();
        assert_eq!(terminated_run_ids, vec![first.id(), second.id()]);
        assert_eq!(
            providers
                .get_run(first.id())
                .expect("first run should remain recorded")
                .state(),
            ProviderRunState::Ended
        );
        assert_eq!(
            providers
                .get_run(second.id())
                .expect("second run should remain recorded")
                .state(),
            ProviderRunState::Ended
        );
        assert_eq!(
            sessions
                .get_session(session.id())
                .expect("session should exist")
                .active_provider_run_id(),
            Some(second.id())
        );
    }

    #[test]
    fn provider_only_park_run_returns_outcome_without_session_mutation() {
        let mut sessions = sessions();
        let session = sessions
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let mut providers = ProviderProcessService::new();
        let run = launch_running_provider_run(
            &mut providers,
            &mut sessions,
            launch_request(session.id(), "sonnet"),
        );

        let outcome = providers
            .park_run_provider_only(session.id(), run.id())
            .expect("provider-only park should succeed");

        assert_eq!(outcome.run().id(), run.id());
        assert_eq!(outcome.run().state(), ProviderRunState::Parked);
        assert_eq!(
            sessions
                .get_session(session.id())
                .expect("session should exist")
                .active_provider_run_id(),
            Some(run.id())
        );
    }

    #[test]
    fn provider_only_resume_run_returns_outcome_without_session_mutation() {
        let mut sessions = sessions();
        let session = sessions
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let mut providers = ProviderProcessService::new();
        let run = launch_running_provider_run(
            &mut providers,
            &mut sessions,
            launch_request(session.id(), "sonnet"),
        );
        providers
            .park_run_provider_only(session.id(), run.id())
            .expect("provider run should park");

        let outcome = providers
            .resume_run_provider_only(session.id(), run.id())
            .expect("provider-only resume should succeed");

        assert_eq!(outcome.run().id(), run.id());
        assert_eq!(outcome.run().state(), ProviderRunState::Running);
        assert_eq!(
            sessions
                .get_session(session.id())
                .expect("session should exist")
                .active_provider_run_id(),
            Some(run.id())
        );
    }

    #[test]
    fn parks_existing_run_when_new_run_becomes_active() {
        let mut sessions = sessions();
        let session = sessions
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let mut providers = ProviderProcessService::new();

        let first = launch_running_provider_run(
            &mut providers,
            &mut sessions,
            launch_request(session.id(), "sonnet"),
        );
        let outcome = providers
            .park_run_provider_only(session.id(), first.id())
            .expect("first run should park");
        sessions
            .set_active_provider_run(session.id(), None)
            .expect("session active run should clear");
        assert_eq!(outcome.run().state(), ProviderRunState::Parked);

        let second = launch_running_provider_run(
            &mut providers,
            &mut sessions,
            launch_request(session.id(), "opus"),
        );

        let first = providers
            .get_run(first.id())
            .expect("first run should still exist");
        let session = sessions
            .get_session(session.id())
            .expect("session should exist");

        assert_eq!(first.state(), ProviderRunState::Parked);
        assert_eq!(second.state(), ProviderRunState::Running);
        assert_eq!(session.active_provider_run_id(), Some(second.id()));
    }

    #[test]
    fn provider_only_start_allows_new_run_after_ended_run() {
        let mut sessions = sessions();
        let session = sessions
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let mut providers = ProviderProcessService::new();

        let first = launch_running_provider_run(
            &mut providers,
            &mut sessions,
            launch_request(session.id(), "sonnet"),
        );
        providers
            .get_run_mut(first.id())
            .expect("first run should exist")
            .mark_ended();

        let second = launch_running_provider_run(
            &mut providers,
            &mut sessions,
            launch_request(session.id(), "opus"),
        );
        let session = sessions
            .get_session(session.id())
            .expect("session should exist");
        let first = providers
            .get_run(first.id())
            .expect("first run should still exist");

        assert_eq!(first.state(), ProviderRunState::Ended);
        assert_eq!(second.state(), ProviderRunState::Running);
        assert_eq!(session.active_provider_run_id(), Some(second.id()));
    }

    #[test]
    fn launch_run_preserves_resume_state_from_the_request() {
        let mut sessions = sessions();
        let session = sessions
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let mut providers = ProviderProcessService::new();

        let run = launch_running_provider_run(
            &mut providers,
            &mut sessions,
            launch_request(session.id(), "sonnet")
                .with_resume_state(ProviderResumeState::from_codex_thread_id("thread-1")),
        );

        assert_eq!(run.resume_state().codex_thread_id(), Some("thread-1"));
    }

    #[test]
    fn merge_opencode_run_selection_keeps_the_existing_run_when_sync_has_no_metadata() {
        let request = LaunchProviderRequest::new(
            "session-1",
            "opencode",
            "opencode",
            "default",
            "anthropic/claude-sonnet-4",
        );
        let run = RuntimeProviderRun::new(
            "provider-run-1",
            &request,
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::External,
                process_label: "opencode:endpoint".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: std::collections::BTreeMap::new(),
                working_directory: None,
                structured_endpoint: Some("http://127.0.0.1:43112".to_string()),
            },
        );

        let merged = ProviderProcessService::merge_opencode_run_selection(
            &run,
            OpenCodeRunSelection::default(),
        );

        assert_eq!(merged.model.as_deref(), Some("anthropic/claude-sonnet-4"));
        assert_eq!(merged.variant.as_deref(), None);
    }
}
