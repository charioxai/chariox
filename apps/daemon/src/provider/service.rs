use std::collections::BTreeMap;

use crate::error::DaemonError;
use crate::session::{PromptAttachment, SessionService};

use super::{
    codex_runtime::{
        abort_codex_turn, drain_codex_events, initialize_codex_runtime, submit_codex_prompt,
        CodexPollResult, CodexRuntimeState,
    },
    opencode_binding::{
        abort_opencode_session, initialize_opencode_runtime, runtime_is_healthy,
        submit_opencode_prompt, sync_opencode_run_selection, OpenCodeRunSelection,
    },
    opencode_runtime::{drain_opencode_events, OpenCodePollResult, OpenCodeRuntimeState},
    LaunchProviderRequest, ProviderRegistry, ProviderRunState, RuntimeProviderRun,
};

#[derive(Debug)]
pub struct ProviderProcessService {
    registry: ProviderRegistry,
    codex_runs: BTreeMap<String, CodexRuntimeState>,
    opencode_runs: BTreeMap<String, OpenCodeRuntimeState>,
    runs: BTreeMap<String, RuntimeProviderRun>,
    next_run_number: u64,
}

impl ProviderProcessService {
    pub fn new() -> Self {
        Self {
            registry: ProviderRegistry::new(),
            codex_runs: BTreeMap::new(),
            opencode_runs: BTreeMap::new(),
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
        let active_run_id = sessions
            .get_session(&request.session_id)?
            .active_provider_run_id()
            .map(str::to_owned);

        if let Some(active_run_id) = active_run_id.as_deref() {
            let active_run = self.get_run(active_run_id)?;
            if active_run.state() == ProviderRunState::Ended {
                sessions.set_active_provider_run(&request.session_id, None)?;
                self.clear_runtime(active_run_id);
            } else {
                self.park_run(sessions, &request.session_id, active_run_id)?;
            }
        }

        let adapter = self.registry.resolve(&request.adapter_key).ok_or_else(|| {
            DaemonError::ProviderAdapterNotFound {
                adapter_key: request.adapter_key.clone(),
            }
        })?;

        let run_id = self.next_run_id();
        let launch_result = adapter.connect(&request)?;
        let mut run = RuntimeProviderRun::new(run_id.clone(), &request, launch_result);
        run.mark_running();

        self.runs.insert(run_id.clone(), run.clone());
        sessions.set_active_provider_run(&request.session_id, Some(run_id))?;

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
        if run.adapter_key() == "codex" {
            let (state, selection) = initialize_codex_runtime(run)?;
            self.codex_runs.insert(run.id().to_string(), state);
            self.apply_codex_run_selection(run.id(), selection)?;
            return Ok(());
        }
        if run.adapter_key() != "opencode" {
            return Ok(());
        }

        let binding = initialize_opencode_runtime(run)?;
        self.opencode_runs
            .insert(run.id().to_string(), binding.state);
        self.apply_opencode_run_selection(run.id(), binding.selection)?;
        self.sync_run_selection(run.id())?;
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

    pub fn sync_run_selection(&mut self, provider_run_id: &str) -> Result<(), DaemonError> {
        if self.codex_runs.contains_key(provider_run_id) {
            return Ok(());
        }
        let Some(state) = self.opencode_runs.get(provider_run_id) else {
            return Ok(());
        };
        let selection = sync_opencode_run_selection(provider_run_id, state)?;
        self.apply_opencode_run_selection(provider_run_id, selection)
    }

    pub fn clear_runtime(&mut self, provider_run_id: &str) {
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

    pub fn submit_structured_prompt(
        &mut self,
        run: &RuntimeProviderRun,
        prompt: &str,
        attachments: &[PromptAttachment],
    ) -> Result<bool, DaemonError> {
        if run.adapter_key() == "codex" {
            let state = self
                .codex_runs
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

    pub fn poll_structured_output(
        &mut self,
        provider_run_id: &str,
    ) -> Result<Option<StructuredPollResult>, DaemonError> {
        let run = self.get_run(provider_run_id)?;
        if run.adapter_key() == "codex" {
            let state = self.codex_runs.get_mut(provider_run_id).ok_or_else(|| {
                DaemonError::ProviderProtocol {
                    provider_run_id: provider_run_id.to_string(),
                    operation: "codex_thread_missing",
                    message: "no Codex thread is bound to this provider run".to_string(),
                }
            })?;
            let poll = drain_codex_events(provider_run_id, state)?;
            return Ok(Some(StructuredPollResult::Codex(poll)));
        }
        if run.adapter_key() != "opencode" {
            return Ok(None);
        }

        let drain = self.drain_opencode_events(provider_run_id)?;

        Ok(Some(StructuredPollResult::OpenCode(OpenCodePollResult {
            chunks: drain.chunks,
            completions: drain.completions,
            prompt_completed: drain.prompt_completed,
            provider_idle: drain.provider_idle,
            notices: drain.notices,
        })))
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

pub enum StructuredPollResult {
    OpenCode(OpenCodePollResult),
    Codex(CodexPollResult),
}

impl Default for ProviderProcessService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use crate::config::DaemonConfig;
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
}
