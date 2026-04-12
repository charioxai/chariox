use std::collections::{BTreeMap, BTreeSet};
use std::thread;
use std::time::Duration;

use crate::error::DaemonError;
use crate::session::{PromptAttachment, SessionService};

use super::{
    codex_runtime::{
        abort_codex_turn, drain_codex_events, initialize_codex_runtime, submit_codex_prompt,
        CodexRuntimeBinding, CodexRuntimeState,
    },
    opencode_binding::{
        abort_opencode_session, initialize_opencode_runtime, runtime_is_healthy,
        submit_opencode_prompt, sync_opencode_run_selection, OpenCodeRunSelection,
        OpenCodeRuntimeBinding,
    },
    opencode_runtime::{drain_opencode_events, OpenCodeRuntimeState},
    LaunchProviderRequest, ProviderAssistantCompletion, ProviderPromptChunk,
    ProviderPromptSignalBatch, ProviderRegistry, ProviderRunActorMailbox,
    ProviderRunOperationLanes, ProviderRunState, RuntimeProviderRun,
};

pub struct ProviderProcessService {
    registry: ProviderRegistry,
    codex_runs: BTreeMap<String, CodexRuntimeState>,
    opencode_runs: BTreeMap<String, OpenCodeRuntimeState>,
    structured_prompt_submissions: BTreeSet<String>,
    run_actor_mailbox: ProviderRunActorMailbox,
    runs: BTreeMap<String, RuntimeProviderRun>,
    next_run_number: u64,
}

pub(crate) enum ProviderRuntimeBinding {
    Codex(CodexRuntimeBinding),
    OpenCode(OpenCodeRuntimeBinding),
}

pub(crate) struct ProviderPromptSubmitJob {
    run: RuntimeProviderRun,
    prompt: String,
    attachments: Vec<PromptAttachment>,
    inner: ProviderPromptSubmitJobInner,
}

enum ProviderPromptSubmitJobInner {
    Codex(CodexRuntimeState),
    OpenCode(OpenCodeRuntimeState),
    DevStubSlow { delay: Duration },
}

pub(crate) struct ProviderPromptSubmitCompletion {
    run_id: String,
    inner: ProviderPromptSubmitJobInner,
}

pub(crate) struct ProviderPromptAbortJob {
    run_id: String,
    inner: ProviderPromptAbortJobInner,
}

enum ProviderPromptAbortJobInner {
    Codex(CodexRuntimeState),
    OpenCode(OpenCodeRuntimeState),
    DevStubSlow { delay: Duration },
}

pub(crate) struct ProviderPromptAbortCompletion {
    run_id: String,
    inner: ProviderPromptAbortJobInner,
}

impl ProviderPromptSubmitJob {
    pub(crate) fn execute(self) -> (ProviderPromptSubmitCompletion, Result<(), DaemonError>) {
        let ProviderPromptSubmitJob {
            run,
            prompt,
            attachments,
            inner,
        } = self;
        let run_id = run.id().to_string();
        match inner {
            ProviderPromptSubmitJobInner::Codex(mut state) => {
                let result = submit_codex_prompt(&run, &mut state, &prompt, &attachments);
                (
                    ProviderPromptSubmitCompletion {
                        run_id,
                        inner: ProviderPromptSubmitJobInner::Codex(state),
                    },
                    result,
                )
            }
            ProviderPromptSubmitJobInner::OpenCode(state) => {
                let result = submit_opencode_prompt(&run, &state, &prompt, &attachments);
                (
                    ProviderPromptSubmitCompletion {
                        run_id,
                        inner: ProviderPromptSubmitJobInner::OpenCode(state),
                    },
                    result,
                )
            }
            ProviderPromptSubmitJobInner::DevStubSlow { delay } => {
                thread::sleep(delay);
                (
                    ProviderPromptSubmitCompletion {
                        run_id,
                        inner: ProviderPromptSubmitJobInner::DevStubSlow { delay },
                    },
                    Ok(()),
                )
            }
        }
    }
}

impl ProviderPromptAbortJob {
    pub(crate) fn execute(self) -> (ProviderPromptAbortCompletion, Result<(), DaemonError>) {
        let ProviderPromptAbortJob { run_id, inner } = self;
        match inner {
            ProviderPromptAbortJobInner::Codex(mut state) => {
                let result = abort_codex_turn(&run_id, &mut state);
                (
                    ProviderPromptAbortCompletion {
                        run_id,
                        inner: ProviderPromptAbortJobInner::Codex(state),
                    },
                    result,
                )
            }
            ProviderPromptAbortJobInner::OpenCode(state) => {
                let result = abort_opencode_session(&run_id, &state);
                (
                    ProviderPromptAbortCompletion {
                        run_id,
                        inner: ProviderPromptAbortJobInner::OpenCode(state),
                    },
                    result,
                )
            }
            ProviderPromptAbortJobInner::DevStubSlow { delay } => {
                thread::sleep(delay);
                (
                    ProviderPromptAbortCompletion {
                        run_id,
                        inner: ProviderPromptAbortJobInner::DevStubSlow { delay },
                    },
                    Ok(()),
                )
            }
        }
    }
}

impl ProviderProcessService {
    pub fn new() -> Self {
        Self {
            registry: ProviderRegistry::new(),
            codex_runs: BTreeMap::new(),
            opencode_runs: BTreeMap::new(),
            structured_prompt_submissions: BTreeSet::new(),
            run_actor_mailbox: ProviderRunActorMailbox::default(),
            runs: BTreeMap::new(),
            next_run_number: 0,
        }
    }

    pub fn registry(&self) -> &ProviderRegistry {
        &self.registry
    }

    pub fn launch_run(
        &mut self,
        sessions: &mut SessionService,
        request: LaunchProviderRequest,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let run = self.start_run(sessions, request)?;
        let run = self.mark_run_running(run.id())?;
        sessions.set_active_provider_run(run.session_id(), Some(run.id().to_string()))?;
        Ok(run)
    }

    pub(crate) fn start_run(
        &mut self,
        sessions: &mut SessionService,
        request: LaunchProviderRequest,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let active_run_id = sessions
            .get_session(&request.session_id)?
            .active_provider_run_id()
            .map(str::to_owned);

        if let Some(active_run_id) = active_run_id.as_deref() {
            let active_run = self.get_run(active_run_id)?;
            match active_run.state() {
                ProviderRunState::Ended => {
                    sessions.set_active_provider_run(&request.session_id, None)?;
                    self.clear_runtime(active_run_id);
                }
                ProviderRunState::Starting => {
                    self.terminate_run(sessions, &request.session_id, active_run_id)?;
                }
                ProviderRunState::Running => {
                    self.park_run(sessions, &request.session_id, active_run_id)?;
                }
                ProviderRunState::Parked => {
                    sessions.set_active_provider_run(&request.session_id, None)?;
                }
            }
        }

        let adapter = self.registry.resolve(&request.adapter_key).ok_or_else(|| {
            DaemonError::ProviderAdapterNotFound {
                adapter_key: request.adapter_key.clone(),
            }
        })?;

        let run_id = self.next_run_id();
        let launch_result = adapter.connect(&request)?;
        let run = RuntimeProviderRun::new(run_id.clone(), &request, launch_result);

        self.runs.insert(run_id.clone(), run.clone());
        sessions.set_active_provider_run(&request.session_id, Some(run_id))?;

        Ok(run)
    }

    pub fn launch_run_detached(
        &mut self,
        request: LaunchProviderRequest,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let adapter = self.registry.resolve(&request.adapter_key).ok_or_else(|| {
            DaemonError::ProviderAdapterNotFound {
                adapter_key: request.adapter_key.clone(),
            }
        })?;

        let run_id = self.next_run_id();
        let launch_result = adapter.connect(&request)?;
        let mut run = RuntimeProviderRun::new(run_id.clone(), &request, launch_result);
        run.mark_running();

        self.runs.insert(run_id, run.clone());

        Ok(run)
    }

    pub fn park_run(
        &mut self,
        sessions: &mut SessionService,
        session_id: &str,
        run_id: &str,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let session = sessions.get_session(session_id)?;

        if session.active_provider_run_id() != Some(run_id) {
            return Err(DaemonError::InconsistentActiveProviderRun {
                session_id: session_id.to_string(),
                active_provider_run_id: session.active_provider_run_id().map(str::to_owned),
                requested_provider_run_id: run_id.to_string(),
            });
        }

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
        sessions.set_active_provider_run(session_id, None)?;

        Ok(run.clone())
    }

    pub fn resume_run(
        &mut self,
        sessions: &mut SessionService,
        session_id: &str,
        run_id: &str,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let active_run_id = sessions
            .get_session(session_id)?
            .active_provider_run_id()
            .map(str::to_owned);

        if let Some(active_run_id) = active_run_id.as_deref() {
            if active_run_id != run_id {
                self.park_run(sessions, session_id, active_run_id)?;
            }
        }

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
        sessions.set_active_provider_run(session_id, Some(run_id.to_string()))?;

        Ok(run.clone())
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

    pub fn terminate_run(
        &mut self,
        sessions: &mut SessionService,
        session_id: &str,
        run_id: &str,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let active_run_id = sessions
            .get_session(session_id)?
            .active_provider_run_id()
            .map(str::to_owned);
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

        let _ = self.abort_structured_runtime(run_id);
        let adapter = self.adapter_for(run_snapshot.adapter_key())?;
        adapter.terminate(&run_snapshot);

        let run = self.get_run_mut(run_id)?;
        run.mark_ended();
        let run = run.clone();

        if active_run_id.as_deref() == Some(run_id) {
            sessions.set_active_provider_run(session_id, None)?;
        }
        self.clear_runtime(run_id);

        Ok(run)
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

    pub fn terminate_session_runs(
        &mut self,
        sessions: &mut SessionService,
        session_id: &str,
    ) -> Result<Vec<RuntimeProviderRun>, DaemonError> {
        let run_ids: Vec<String> = self
            .runs
            .values()
            .filter(|run| run.session_id() == session_id && run.state() != ProviderRunState::Ended)
            .map(|run| run.id().to_string())
            .collect();

        let mut terminated_runs = Vec::with_capacity(run_ids.len());

        for run_id in run_ids {
            terminated_runs.push(self.terminate_run(sessions, session_id, &run_id)?);
        }

        Ok(terminated_runs)
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
                self.codex_runs.insert(run_id.to_string(), binding.state);
                let run_mut = self.get_run_mut(run_id)?;
                run_mut.set_resume_state(binding.resume_state.clone());
                run_mut.set_provider_session_id(
                    binding.resume_state.codex_thread_id().map(str::to_string),
                );
                self.apply_codex_run_selection(run_id, binding.selection)?;
            }
            ProviderRuntimeBinding::OpenCode(binding) => {
                self.opencode_runs.insert(run_id.to_string(), binding.state);
                let run_mut = self.get_run_mut(run_id)?;
                run_mut.set_resume_state(binding.resume_state.clone());
                run_mut.set_provider_session_id(
                    binding
                        .resume_state
                        .opencode_session_id()
                        .map(str::to_string),
                );
                self.apply_opencode_run_selection(run_id, binding.selection)?;
                self.sync_run_selection(run_id)?;
            }
        }
        Ok(())
    }

    pub fn runtime_is_healthy(&self, run_id: &str) -> bool {
        if self.codex_runs.contains_key(run_id) {
            return true;
        }
        let Some(state) = self.opencode_runs.get(run_id) else {
            return false;
        };
        runtime_is_healthy(run_id, state)
    }

    pub(crate) fn run_uses_structured_prompt_io(&self, run: &RuntimeProviderRun) -> bool {
        run.adapter_key() == "codex"
            || run.adapter_key() == "opencode"
            || (run.adapter_key() == "dev-stub" && run.provider() == "slow-structured")
    }

    pub(crate) fn structured_prompt_io_in_flight(&self, provider_run_id: &str) -> bool {
        self.structured_prompt_submissions.contains(provider_run_id)
    }

    pub(crate) fn run_operation_lanes(&self) -> ProviderRunOperationLanes {
        self.run_actor_mailbox.operation_lanes()
    }

    pub fn sync_run_selection(&mut self, provider_run_id: &str) -> Result<(), DaemonError> {
        if self.codex_runs.contains_key(provider_run_id) {
            return Ok(());
        }
        let Some(state) = self.opencode_runs.get(provider_run_id) else {
            return Ok(());
        };
        let current_run = self.get_run(provider_run_id)?;
        let selection = sync_opencode_run_selection(provider_run_id, state)?;
        self.apply_opencode_run_selection(
            provider_run_id,
            Self::merge_opencode_run_selection(&current_run, selection),
        )
    }

    pub fn clear_runtime(&mut self, provider_run_id: &str) {
        self.structured_prompt_submissions.remove(provider_run_id);
        self.codex_runs.remove(provider_run_id);
        if let Some(state) = self.opencode_runs.remove(provider_run_id) {
            state.stop();
        }
    }

    pub fn abort_structured_runtime(&mut self, provider_run_id: &str) -> Result<bool, DaemonError> {
        let run = self.get_run(provider_run_id)?;
        if run.adapter_key() == "codex" {
            let state = self.codex_runs.get_mut(provider_run_id).ok_or_else(|| {
                DaemonError::ProviderProtocol {
                    provider_run_id: provider_run_id.to_string(),
                    operation: "codex_thread_missing",
                    message: "no Codex thread is bound to this provider run".to_string(),
                }
            })?;
            abort_codex_turn(provider_run_id, state)?;
            return Ok(true);
        }
        if run.adapter_key() != "opencode" {
            return Ok(false);
        }

        let state = self.opencode_runs.get(provider_run_id).ok_or_else(|| {
            DaemonError::ProviderProtocol {
                provider_run_id: provider_run_id.to_string(),
                operation: "opencode_session_missing",
                message: "no OpenCode session is bound to this provider run".to_string(),
            }
        })?;
        abort_opencode_session(provider_run_id, state)?;
        Ok(true)
    }

    fn take_structured_prompt_abort_job(
        &mut self,
        provider_run_id: &str,
    ) -> Result<Option<ProviderPromptAbortJob>, DaemonError> {
        let run = self.get_run(provider_run_id)?;
        if run.adapter_key() == "dev-stub" && run.provider() == "slow-structured" {
            self.structured_prompt_submissions
                .insert(provider_run_id.to_string());
            return Ok(Some(ProviderPromptAbortJob {
                run_id: provider_run_id.to_string(),
                inner: ProviderPromptAbortJobInner::DevStubSlow {
                    delay: Duration::from_millis(750),
                },
            }));
        }
        if run.adapter_key() == "codex" {
            let state = self.codex_runs.remove(provider_run_id).ok_or_else(|| {
                DaemonError::ProviderProtocol {
                    provider_run_id: provider_run_id.to_string(),
                    operation: "codex_thread_missing",
                    message: "no Codex thread is bound to this provider run".to_string(),
                }
            })?;
            self.structured_prompt_submissions
                .insert(provider_run_id.to_string());
            return Ok(Some(ProviderPromptAbortJob {
                run_id: provider_run_id.to_string(),
                inner: ProviderPromptAbortJobInner::Codex(state),
            }));
        }
        if run.adapter_key() != "opencode" {
            return Ok(None);
        }

        let state = self.opencode_runs.remove(provider_run_id).ok_or_else(|| {
            DaemonError::ProviderProtocol {
                provider_run_id: provider_run_id.to_string(),
                operation: "opencode_session_missing",
                message: "no OpenCode session is bound to this provider run".to_string(),
            }
        })?;
        self.structured_prompt_submissions
            .insert(provider_run_id.to_string());
        Ok(Some(ProviderPromptAbortJob {
            run_id: provider_run_id.to_string(),
            inner: ProviderPromptAbortJobInner::OpenCode(state),
        }))
    }

    pub(crate) fn finish_structured_prompt_abort_job(
        &mut self,
        completion: ProviderPromptAbortCompletion,
    ) {
        self.structured_prompt_submissions
            .remove(&completion.run_id);
        match completion.inner {
            ProviderPromptAbortJobInner::Codex(state) => {
                self.codex_runs.insert(completion.run_id, state);
            }
            ProviderPromptAbortJobInner::OpenCode(state) => {
                self.opencode_runs.insert(completion.run_id, state);
            }
            ProviderPromptAbortJobInner::DevStubSlow { .. } => {}
        }
    }

    pub fn submit_structured_prompt(
        &mut self,
        run: &RuntimeProviderRun,
        prompt: &str,
        attachments: &[PromptAttachment],
    ) -> Result<bool, DaemonError> {
        let _ = self.record_run_activity(run.id());
        if run.adapter_key() == "codex" {
            let state =
                self.codex_runs
                    .get_mut(run.id())
                    .ok_or_else(|| DaemonError::ProviderProtocol {
                        provider_run_id: run.id().to_string(),
                        operation: "codex_thread_missing",
                        message: "no Codex thread is bound to this provider run".to_string(),
                    })?;
            submit_codex_prompt(run, state, prompt, attachments)?;
            return Ok(true);
        }
        if run.adapter_key() != "opencode" {
            return Ok(false);
        }

        let state =
            self.opencode_runs
                .get(run.id())
                .ok_or_else(|| DaemonError::ProviderProtocol {
                    provider_run_id: run.id().to_string(),
                    operation: "opencode_session_missing",
                    message: "no OpenCode session is bound to this provider run".to_string(),
                })?;
        submit_opencode_prompt(run, state, prompt, attachments)?;
        Ok(true)
    }

    fn take_structured_prompt_submit_job(
        &mut self,
        run: &RuntimeProviderRun,
        prompt: &str,
        attachments: &[PromptAttachment],
    ) -> Result<Option<ProviderPromptSubmitJob>, DaemonError> {
        let _ = self.record_run_activity(run.id());
        if run.adapter_key() == "dev-stub" && run.provider() == "slow-structured" {
            self.structured_prompt_submissions
                .insert(run.id().to_string());
            return Ok(Some(ProviderPromptSubmitJob {
                run: run.clone(),
                prompt: prompt.to_string(),
                attachments: attachments.to_vec(),
                inner: ProviderPromptSubmitJobInner::DevStubSlow {
                    delay: Duration::from_millis(750),
                },
            }));
        }
        if run.adapter_key() == "codex" {
            let state =
                self.codex_runs
                    .remove(run.id())
                    .ok_or_else(|| DaemonError::ProviderProtocol {
                        provider_run_id: run.id().to_string(),
                        operation: "codex_thread_missing",
                        message: "no Codex thread is bound to this provider run".to_string(),
                    })?;
            self.structured_prompt_submissions
                .insert(run.id().to_string());
            return Ok(Some(ProviderPromptSubmitJob {
                run: run.clone(),
                prompt: prompt.to_string(),
                attachments: attachments.to_vec(),
                inner: ProviderPromptSubmitJobInner::Codex(state),
            }));
        }
        if run.adapter_key() != "opencode" {
            return Ok(None);
        }

        let state =
            self.opencode_runs
                .remove(run.id())
                .ok_or_else(|| DaemonError::ProviderProtocol {
                    provider_run_id: run.id().to_string(),
                    operation: "opencode_session_missing",
                    message: "no OpenCode session is bound to this provider run".to_string(),
                })?;
        self.structured_prompt_submissions
            .insert(run.id().to_string());
        Ok(Some(ProviderPromptSubmitJob {
            run: run.clone(),
            prompt: prompt.to_string(),
            attachments: attachments.to_vec(),
            inner: ProviderPromptSubmitJobInner::OpenCode(state),
        }))
    }

    pub(crate) fn finish_structured_prompt_submit_job(
        &mut self,
        completion: ProviderPromptSubmitCompletion,
    ) {
        self.structured_prompt_submissions
            .remove(&completion.run_id);
        match completion.inner {
            ProviderPromptSubmitJobInner::Codex(state) => {
                self.codex_runs.insert(completion.run_id, state);
            }
            ProviderPromptSubmitJobInner::OpenCode(state) => {
                self.opencode_runs.insert(completion.run_id, state);
            }
            ProviderPromptSubmitJobInner::DevStubSlow { .. } => {}
        }
    }

    fn spawn_structured_prompt_submit_job(
        &mut self,
        session_id: String,
        provider_run_id: String,
        agent_id: String,
        job: ProviderPromptSubmitJob,
    ) {
        self.run_actor_mailbox
            .spawn_submit(session_id, provider_run_id, agent_id, job);
    }

    fn spawn_structured_prompt_abort_job(
        &mut self,
        session_id: String,
        provider_run_id: String,
        job: ProviderPromptAbortJob,
    ) {
        self.run_actor_mailbox
            .spawn_abort(session_id, provider_run_id, job);
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
        let job = self
            .take_structured_prompt_submit_job(run, prompt, attachments)?
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "enqueue structured prompt dispatch",
                message: format!(
                    "provider run `{provider_run_id}` does not use structured prompt I/O"
                ),
            })?;
        self.spawn_structured_prompt_submit_job(session_id, provider_run_id, agent_id, job);
        Ok(())
    }

    pub(crate) fn enqueue_structured_prompt_abort(
        &mut self,
        session_id: String,
        provider_run_id: String,
    ) -> Result<(), DaemonError> {
        let job = self
            .take_structured_prompt_abort_job(&provider_run_id)?
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "enqueue structured prompt abort",
                message: format!(
                    "provider run `{provider_run_id}` does not use structured prompt I/O"
                ),
            })?;
        self.spawn_structured_prompt_abort_job(session_id, provider_run_id, job);
        Ok(())
    }

    pub(crate) fn drain_finished_structured_prompt_submits(
        &mut self,
    ) -> Vec<super::FinishedProviderPromptSubmitJob> {
        self.run_actor_mailbox.drain_finished_submits()
    }

    pub(crate) fn drain_finished_structured_prompt_aborts(
        &mut self,
    ) -> Vec<super::FinishedProviderPromptAbortJob> {
        self.run_actor_mailbox.drain_finished_aborts()
    }

    pub fn poll_structured_output(
        &mut self,
        provider_run_id: &str,
    ) -> Result<Option<ProviderPromptSignalBatch>, DaemonError> {
        let _ = self.record_run_activity(provider_run_id);
        let run = self.get_run(provider_run_id)?;
        if run.adapter_key() == "codex" {
            if self.structured_prompt_submissions.contains(provider_run_id)
                && !self.codex_runs.contains_key(provider_run_id)
            {
                return Ok(None);
            }
            let state = self.codex_runs.get_mut(provider_run_id).ok_or_else(|| {
                DaemonError::ProviderProtocol {
                    provider_run_id: provider_run_id.to_string(),
                    operation: "codex_thread_missing",
                    message: "no Codex thread is bound to this provider run".to_string(),
                }
            })?;
            let poll = drain_codex_events(provider_run_id, state)?;
            return Ok(Some(ProviderPromptSignalBatch {
                chunks: poll
                    .chunks
                    .into_iter()
                    .map(|chunk| ProviderPromptChunk {
                        kind: chunk.kind,
                        merge_key: chunk.merge_key,
                        bytes: chunk.bytes,
                    })
                    .collect(),
                completions: poll
                    .completions
                    .into_iter()
                    .map(|completion| ProviderAssistantCompletion {
                        message_id: completion.message_id,
                        completed_at_ms: completion.completed_at_ms,
                    })
                    .collect(),
                prompt_completed: poll.prompt_completed,
                notices: poll.notices,
            }));
        }
        if run.adapter_key() != "opencode" {
            return Ok(None);
        }
        if self.structured_prompt_submissions.contains(provider_run_id)
            && !self.opencode_runs.contains_key(provider_run_id)
        {
            return Ok(None);
        }

        let drain = self.drain_opencode_events(provider_run_id)?;

        Ok(Some(ProviderPromptSignalBatch {
            chunks: drain
                .chunks
                .into_iter()
                .map(|chunk| ProviderPromptChunk {
                    kind: chunk.kind,
                    merge_key: chunk.merge_key,
                    bytes: chunk.bytes,
                })
                .collect(),
            completions: drain
                .completions
                .into_iter()
                .map(|completion| ProviderAssistantCompletion {
                    message_id: completion.message_id,
                    completed_at_ms: completion.completed_at_ms,
                })
                .collect(),
            prompt_completed: drain.prompt_completed,
            notices: drain.notices,
        }))
    }

    pub fn mark_run_ended(
        &mut self,
        sessions: &mut SessionService,
        session_id: &str,
        run_id: &str,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let active_run_id = sessions
            .get_session(session_id)?
            .active_provider_run_id()
            .map(str::to_owned);
        let run_snapshot = self.get_run(run_id)?;

        if run_snapshot.session_id() != session_id {
            return Err(DaemonError::ProviderRunNotInSession {
                session_id: session_id.to_string(),
                provider_run_id: run_id.to_string(),
            });
        }

        if run_snapshot.state() == ProviderRunState::Ended {
            self.clear_runtime(run_id);
            return Ok(run_snapshot);
        }

        let run = self.get_run_mut(run_id)?;
        run.mark_ended();
        let run = run.clone();

        if active_run_id.as_deref() == Some(run_id) {
            sessions.set_active_provider_run(session_id, None)?;
        }
        self.clear_runtime(run_id);

        Ok(run)
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

    fn drain_opencode_events(
        &mut self,
        provider_run_id: &str,
    ) -> Result<super::opencode_runtime::OpenCodeEventDrainResult, DaemonError> {
        let drain = {
            let state = self.opencode_runs.get_mut(provider_run_id).ok_or_else(|| {
                DaemonError::ProviderProtocol {
                    provider_run_id: provider_run_id.to_string(),
                    operation: "opencode_session_missing",
                    message: "no OpenCode session is bound to this provider run".to_string(),
                }
            })?;
            drain_opencode_events(state, provider_run_id)?
        };

        if drain.resolved_model.is_some()
            || drain.resolved_variant.is_some()
            || drain.resolved_usage_tokens_total.is_some()
        {
            let run = self.get_run_mut(provider_run_id)?;
            if let Some(model) = drain.resolved_model.as_deref() {
                crate::logging::debug_with_fields(
                    "daemon.provider.opencode",
                    "resolved provider run model from opencode metadata",
                    serde_json::json!({
                        "provider_run_id": provider_run_id,
                        "previous_model": run.model(),
                        "resolved_model": model,
                        "source": drain.resolved_model_source,
                    }),
                );
                if run.model() != model {
                    run.set_model(model.to_string());
                }
            }
            if let Some(variant) = drain.resolved_variant.as_deref() {
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
            if let Some(total_tokens) = drain.resolved_usage_tokens_total {
                if run.usage_tokens_total() != Some(total_tokens) {
                    run.set_usage_tokens_total(Some(total_tokens));
                }
            }
        }

        Ok(drain)
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
    use crate::provider::opencode_binding::OpenCodeRunSelection;
    use crate::provider::{
        AgentEndpointMode, ProviderLaunchResult, ProviderResumeState, RuntimeProviderRun,
    };
    use crate::session::{CreateSessionRequest, SessionService, SessionStatus};

    use super::{LaunchProviderRequest, ProviderProcessService, ProviderRunState};

    fn sessions() -> SessionService {
        SessionService::new(&DaemonConfig::for_tests())
    }

    fn launch_request(session_id: &str, model: &str) -> LaunchProviderRequest {
        LaunchProviderRequest::new(session_id, "dev-stub", "claude-code", "default", model)
    }

    #[test]
    fn launches_the_first_provider_run() {
        let mut sessions = sessions();
        let session = sessions
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let mut providers = ProviderProcessService::new();

        let run = providers
            .launch_run(&mut sessions, launch_request(session.id(), "sonnet"))
            .expect("provider run should launch");
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
    fn parks_existing_run_when_new_run_becomes_active() {
        let mut sessions = sessions();
        let session = sessions
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let mut providers = ProviderProcessService::new();

        let first = providers
            .launch_run(&mut sessions, launch_request(session.id(), "sonnet"))
            .expect("first run should launch");
        let second = providers
            .launch_run(&mut sessions, launch_request(session.id(), "opus"))
            .expect("second run should launch");

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
    fn rejects_inconsistent_active_run_state() {
        let mut sessions = sessions();
        let session = sessions
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let mut providers = ProviderProcessService::new();

        sessions
            .set_active_provider_run(session.id(), Some("missing-run".to_string()))
            .expect("session active run can be set for this invariant test");

        let error = providers
            .launch_run(&mut sessions, launch_request(session.id(), "sonnet"))
            .expect_err("launch should reject inconsistent active run state");

        match error {
            crate::DaemonError::ProviderRunNotFound { provider_run_id } => {
                assert_eq!(provider_run_id, "missing-run");
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn launches_new_run_when_session_points_at_ended_active_run() {
        let mut sessions = sessions();
        let session = sessions
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let mut providers = ProviderProcessService::new();

        let first = providers
            .launch_run(&mut sessions, launch_request(session.id(), "sonnet"))
            .expect("first run should launch");
        providers
            .get_run_mut(first.id())
            .expect("first run should exist")
            .mark_ended();

        let second = providers
            .launch_run(&mut sessions, launch_request(session.id(), "opus"))
            .expect("second run should launch even if active run is stale and ended");
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

        let run = providers
            .launch_run(
                &mut sessions,
                launch_request(session.id(), "sonnet")
                    .with_resume_state(ProviderResumeState::from_codex_thread_id("thread-1")),
            )
            .expect("provider run should launch");

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
