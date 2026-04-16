use std::collections::BTreeSet;

use crate::error::DaemonError;
use crate::kernel::projection::{
    AgentRuntimeProjectionStore, ProviderRunProjectionStore, SessionStateProjectionStore,
};
use crate::kernel::runtime_state::KernelRuntimeState;
use crate::local::{LocalDaemonResponse, PumpTerminalOutputRequest};
use crate::provider::{ProviderRunOperationLanes, ProviderRunState};
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
        if session
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

        let provider_run_ids = self.provider_run_ids_for_pump(&session);
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
