use std::collections::BTreeSet;

use crate::error::DaemonError;
use crate::local::{
    AppendNativeProviderOutputRequest, LocalDaemonResponse, PumpTerminalOutputRequest,
};
use crate::provider::{ProviderRunOperationLanes, ProviderRunState};
use crate::runtime::projection::{
    AgentRuntimeProjectionStore, ProviderRunProjectionStore, SessionStateProjectionStore,
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
        let mut provider_run_ids = BTreeSet::new();
        if let Some(provider_run_id) = session.active_provider_run_id() {
            if !self.projected_run_is_idle_for_session(session.id(), provider_run_id) {
                provider_run_ids.insert(provider_run_id.to_string());
            }
        }
        for (agent_id, prompt_state) in session.prompt_states() {
            if prompt_state.active_prompt().is_none() {
                continue;
            }
            if let Some(run) = self
                .provider_run_projection
                .get_for_agent(session.id(), agent_id)
            {
                if !matches!(
                    run.state(),
                    ProviderRunState::Ended | ProviderRunState::Parked
                ) {
                    provider_run_ids.insert(run.id().to_string());
                }
            }
        }
        for run in self.provider_run_projection.list_for_session(session.id()) {
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

    fn projected_run_is_idle_for_session(&self, session_id: &str, provider_run_id: &str) -> bool {
        self.provider_run_projection
            .get(provider_run_id)
            .is_some_and(|run| {
                run.session_id() == session_id
                    && matches!(
                        run.state(),
                        ProviderRunState::Ended | ProviderRunState::Parked
                    )
            })
    }
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
            self.agent_runtime_projection.update_session(&session);
            self.session_projection.update(session);
        }
    }
}

pub(crate) async fn execute_append_native_provider_output_request(
    runtime_state: &KernelRuntimeState,
    session_projection: &SessionStateProjectionStore,
    agent_runtime_projection: &AgentRuntimeProjectionStore,
    request: AppendNativeProviderOutputRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let (records, session) = runtime_state.append_native_provider_output(request)?;
    agent_runtime_projection.update_session(&session);
    session_projection.update(session);
    Ok(LocalDaemonResponse::TerminalOutput { records })
}
