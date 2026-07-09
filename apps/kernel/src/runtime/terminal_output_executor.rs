use std::collections::BTreeSet;

use crate::error::DaemonError;
use crate::local::{
    AppendNativeProviderOutputBatchRequest, AppendNativeProviderOutputRequest, LocalDaemonResponse,
    PumpTerminalOutputRequest,
};
use crate::provider::{ProviderRunOperationLanes, ProviderRunState};
use crate::runtime::projection::{
    publish_session_runtime_projection, AgentRuntimeProjectionStore, ProviderRunProjectionStore,
    SessionStateProjectionStore,
};
use crate::runtime::session_read_control::projected_session_or_absence;
use crate::runtime::state::KernelRuntimeState;
use crate::terminal::TerminalStreamStore;

#[derive(Clone)]
pub(crate) struct TerminalOutputExecutor {
    terminal_output_store: TerminalOutputStore,
    provider_runtime_lanes: ProviderRunOperationLanes,
    session_projection: SessionStateProjectionStore,
    provider_run_projection: ProviderRunProjectionStore,
    terminal_stream: TerminalStreamStore,
}

#[derive(Clone)]
struct TerminalOutputStore {
    state: KernelRuntimeState,
    session_projection: SessionStateProjectionStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
}

impl TerminalOutputExecutor {
    pub(crate) fn new(
        state: KernelRuntimeState,
        provider_runtime_lanes: ProviderRunOperationLanes,
        session_projection: SessionStateProjectionStore,
        agent_runtime_projection: AgentRuntimeProjectionStore,
        provider_run_projection: ProviderRunProjectionStore,
        terminal_stream: TerminalStreamStore,
    ) -> Self {
        let terminal_output_store =
            TerminalOutputStore::new(state, session_projection.clone(), agent_runtime_projection);
        Self {
            terminal_output_store,
            provider_runtime_lanes,
            session_projection,
            provider_run_projection,
            terminal_stream,
        }
    }

    pub(crate) async fn execute(
        &self,
        request: PumpTerminalOutputRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let Some(session) = self.session_projection.get(&request.session_id) else {
            let records = self
                .terminal_output_store
                .pump_terminal_output_with_compat_snapshot(
                    &request.session_id,
                    &request.attachment_id,
                )
                .await?;
            return Ok(LocalDaemonResponse::TerminalOutput { records });
        };
        if !session.has_attachment(&request.attachment_id) {
            return Err(DaemonError::AttachmentNotInSession {
                session_id: request.session_id,
                attachment_id: request.attachment_id,
            });
        }
        self.terminal_output_store
            .state
            .record_terminal_attachment_heartbeat(
                &request.session_id,
                &request.attachment_id,
                crate::session::unix_epoch_ms(),
            )
            .await?;
        let provider_run_ids = self.provider_run_ids_for_pump(&session);
        if provider_run_ids.is_empty()
            && session
                .active_provider_run_id()
                .is_none_or(|provider_run_id| {
                    self.projected_provider_run_is_idle(&request, provider_run_id)
                })
            && !session.has_any_prompt_work()
        {
            return Ok(LocalDaemonResponse::TerminalOutput {
                records: self
                    .terminal_stream
                    .drain_output_records(&request.session_id, &request.attachment_id),
            });
        }
        if self
            .terminal_stream
            .has_pending_output_records(&request.session_id, &request.attachment_id)
        {
            return Ok(LocalDaemonResponse::TerminalOutput {
                records: self
                    .terminal_stream
                    .drain_output_records(&request.session_id, &request.attachment_id),
            });
        }

        let mut permits = Vec::new();
        for provider_run_id in &provider_run_ids {
            permits.push(self.provider_runtime_lanes.acquire(provider_run_id).await);
        }
        let records = self
            .terminal_output_store
            .pump_terminal_output_with_compat_snapshot(&request.session_id, &request.attachment_id)
            .await?;
        drop(permits);
        Ok(LocalDaemonResponse::TerminalOutput { records })
    }

    pub(crate) fn projected_response(
        &self,
        request: &PumpTerminalOutputRequest,
    ) -> Option<Result<LocalDaemonResponse, DaemonError>> {
        let session =
            match projected_session_or_absence(&self.session_projection, &request.session_id)? {
                Ok(session) => session,
                Err(error) => return Some(Err(error)),
            };
        if !session.has_attachment(&request.attachment_id) {
            return Some(Err(DaemonError::AttachmentNotInSession {
                session_id: request.session_id.clone(),
                attachment_id: request.attachment_id.clone(),
            }));
        }
        let active_provider_run_id = session.active_provider_run_id();
        if !session.has_any_prompt_work()
            && (active_provider_run_id.is_none()
                || active_provider_run_id.is_some_and(|provider_run_id| {
                    self.provider_run_projection
                        .get(provider_run_id)
                        .is_some_and(|run| {
                            run.session_id() == request.session_id
                                && matches!(
                                    run.state(),
                                    ProviderRunState::Ended | ProviderRunState::Parked
                                )
                        })
                }))
        {
            return Some(Ok(LocalDaemonResponse::TerminalOutput {
                records: self
                    .terminal_stream
                    .drain_output_records(&request.session_id, &request.attachment_id),
            }));
        }
        None
    }

    fn projected_provider_run_is_idle(
        &self,
        request: &PumpTerminalOutputRequest,
        provider_run_id: &str,
    ) -> bool {
        self.provider_run_projection
            .get(provider_run_id)
            .is_some_and(|run| {
                run.session_id() == request.session_id
                    && matches!(
                        run.state(),
                        ProviderRunState::Ended | ProviderRunState::Parked
                    )
            })
    }

    fn provider_run_ids_for_pump(
        &self,
        session: &crate::session::RuntimeSession,
    ) -> BTreeSet<String> {
        provider_run_ids_for_pump_from_projections(
            session,
            &self.terminal_output_store.agent_runtime_projection,
            &self.provider_run_projection,
        )
    }
}

fn provider_run_ids_for_pump_from_projections(
    session: &crate::session::RuntimeSession,
    agent_runtime_projection: &AgentRuntimeProjectionStore,
    provider_run_projection: &ProviderRunProjectionStore,
) -> BTreeSet<String> {
    let mut provider_run_ids = BTreeSet::new();
    if let Some(provider_run_id) = session.active_provider_run_id() {
        if !projected_run_is_idle_for_session(
            provider_run_projection,
            session.id(),
            provider_run_id,
        ) {
            provider_run_ids.insert(provider_run_id.to_string());
        }
    }
    for projection in agent_runtime_projection.list_for_session(session.id()) {
        if projection.active_prompt.is_none() {
            continue;
        }
        if let Some(run) = provider_run_projection.get_for_agent(session.id(), &projection.agent_id)
        {
            if !matches!(
                run.state(),
                ProviderRunState::Ended | ProviderRunState::Parked
            ) {
                provider_run_ids.insert(run.id().to_string());
            }
        }
    }
    for run in provider_run_projection.list_for_session(session.id()) {
        if run.client_interface().is_arroba() {
            continue;
        }
        if matches!(
            run.state(),
            ProviderRunState::Starting | ProviderRunState::Running
        ) {
            provider_run_ids.insert(run.id().to_string());
        }
    }
    provider_run_ids
}

fn projected_run_is_idle_for_session(
    provider_run_projection: &ProviderRunProjectionStore,
    session_id: &str,
    provider_run_id: &str,
) -> bool {
    provider_run_projection
        .get(provider_run_id)
        .is_some_and(|run| {
            run.session_id() == session_id
                && matches!(
                    run.state(),
                    ProviderRunState::Ended | ProviderRunState::Parked
                )
        })
}

impl TerminalOutputStore {
    fn new(
        state: KernelRuntimeState,
        session_projection: SessionStateProjectionStore,
        agent_runtime_projection: AgentRuntimeProjectionStore,
    ) -> Self {
        Self {
            state,
            session_projection,
            agent_runtime_projection,
        }
    }

    async fn pump_terminal_output_with_compat_snapshot(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<Vec<crate::terminal::TerminalOutputRecord>, DaemonError> {
        let (records, session) = self
            .state
            .pump_terminal_output_with_snapshot(session_id, attachment_id)
            .await?;
        self.refresh_session_projection(session);
        Ok(records)
    }

    fn refresh_session_projection(&self, session: Option<crate::session::RuntimeSession>) {
        if let Some(session) = session {
            publish_session_runtime_projection(
                &self.session_projection,
                &self.agent_runtime_projection,
                &session,
            );
        }
    }
}

pub(crate) async fn execute_append_native_provider_output_request(
    runtime_state: &KernelRuntimeState,
    _session_projection: &SessionStateProjectionStore,
    _agent_runtime_projection: &AgentRuntimeProjectionStore,
    request: AppendNativeProviderOutputRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let records = runtime_state.append_native_provider_output(request)?;
    Ok(LocalDaemonResponse::TerminalOutput { records })
}

pub(crate) async fn execute_append_native_provider_output_batch_request(
    runtime_state: &KernelRuntimeState,
    _session_projection: &SessionStateProjectionStore,
    _agent_runtime_projection: &AgentRuntimeProjectionStore,
    request: AppendNativeProviderOutputBatchRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let records = runtime_state.append_native_provider_output_batch(request)?;
    Ok(LocalDaemonResponse::TerminalOutput { records })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn provider_run_ids_for_pump_uses_agent_runtime_projection_when_session_mirror_is_stale() {
        let session = crate::session::RuntimeSession::new(
            "session-terminal-pump",
            None,
            "workspace",
            "worktree",
            "machine",
            "daemon",
        );
        assert!(
            session.prompt_states().is_empty(),
            "session mirror starts without prompt state"
        );
        let agent_runtime_projection = AgentRuntimeProjectionStore::default();
        let provider_run_projection = ProviderRunProjectionStore::default();
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "codex",
            "codex",
            "default",
            "gpt-test",
        )
        .with_agent_id("agent-1");
        let launch = crate::provider::ProviderLaunchResult {
            process_label: "codex:test".to_string(),
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("codex:test".to_string()),
        };
        let mut run = crate::provider::RuntimeProviderRun::new(
            "provider-run-terminal-pump",
            &request,
            launch,
        );
        run.mark_running();
        provider_run_projection.update(run);
        let active_prompt = crate::session::PromptQueueItem::external_observed_running(
            "codex",
            "codex-thread-terminal-pump",
            "codex-turn-terminal-pump",
            "agent-1",
            "external prompt still running",
        );
        agent_runtime_projection.update_agent_prompt_state(
            session.id(),
            "agent-1",
            Some(active_prompt),
            None,
            0,
        );

        let provider_run_ids = provider_run_ids_for_pump_from_projections(
            &session,
            &agent_runtime_projection,
            &provider_run_projection,
        );

        assert!(
            provider_run_ids.contains("provider-run-terminal-pump"),
            "terminal pump should use projected active prompt state, not stale session mirror"
        );
    }
}
