use crate::error::DaemonError;
use crate::kernel::projection::{
    AgentRuntimeProjectionStore, ProviderRunProjectionStore, SessionStateProjectionStore,
};
use crate::kernel::runtime_state::CompatibilityRuntimeState;
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
    state: CompatibilityRuntimeState,
    session_projection: SessionStateProjectionStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
}

impl TerminalOutputExecutor {
    pub(crate) fn new(
        state: CompatibilityRuntimeState,
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
        let Some(provider_run_id) = session.active_provider_run_id().map(str::to_string) else {
            return Ok(LocalDaemonResponse::TerminalOutput {
                records: self
                    .terminal_stream
                    .drain_output_records(&request.session_id, &request.attachment_id),
            });
        };
        if self
            .provider_run_projection
            .get(&provider_run_id)
            .is_some_and(|run| {
                run.session_id() == request.session_id
                    && matches!(
                        run.state(),
                        ProviderRunState::Ended | ProviderRunState::Parked
                    )
            })
        {
            return Ok(LocalDaemonResponse::TerminalOutput {
                records: self
                    .terminal_stream
                    .drain_output_records(&request.session_id, &request.attachment_id),
            });
        }

        let recipient_attachment_ids = session.attachment_ids().iter().cloned().collect();
        let _permit = self.provider_runtime_lanes.acquire(&provider_run_id).await;
        self.terminal_output_store
            .pump_active_provider_output(
                &request.session_id,
                &provider_run_id,
                recipient_attachment_ids,
            )
            .await?;
        Ok(LocalDaemonResponse::TerminalOutput {
            records: self
                .terminal_stream
                .drain_output_records(&request.session_id, &request.attachment_id),
        })
    }
}

impl TerminalOutputStore {
    fn new(
        state: CompatibilityRuntimeState,
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

    async fn pump_active_provider_output(
        &self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
    ) -> Result<(), DaemonError> {
        let session = self
            .state
            .pump_active_provider_output_with_snapshot(
                session_id,
                provider_run_id,
                recipient_attachment_ids,
            )
            .await?;
        self.refresh_session_projection(session);
        Ok(())
    }

    fn refresh_session_projection(&self, session: Option<crate::session::RuntimeSession>) {
        if let Some(session) = session {
            self.agent_runtime_projection.update_session(&session);
            self.session_projection.update(session);
        }
    }
}
