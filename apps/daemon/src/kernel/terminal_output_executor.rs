use std::sync::Arc;

use tokio::sync::Mutex;

use crate::app::provider_output::{
    pump_terminal_output_for_attachment, ProviderOutputPump, ProviderOutputPumpRequest,
};
use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::kernel::projection::{
    AgentRuntimeProjectionStore, ProviderRunProjectionStore, SessionStateProjectionStore,
};
use crate::local::{LocalDaemonResponse, PumpTerminalOutputRequest};
use crate::provider::{ProviderRunOperationLanes, ProviderRunState};
use crate::terminal::TerminalStreamStore;

#[derive(Clone)]
pub(crate) struct TerminalOutputExecutor {
    app: Arc<Mutex<DaemonApp>>,
    provider_runtime_lanes: ProviderRunOperationLanes,
    session_projection: SessionStateProjectionStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
    provider_run_projection: ProviderRunProjectionStore,
    terminal_stream: TerminalStreamStore,
}

impl TerminalOutputExecutor {
    pub(crate) fn new(
        app: Arc<Mutex<DaemonApp>>,
        provider_runtime_lanes: ProviderRunOperationLanes,
        session_projection: SessionStateProjectionStore,
        agent_runtime_projection: AgentRuntimeProjectionStore,
        provider_run_projection: ProviderRunProjectionStore,
        terminal_stream: TerminalStreamStore,
    ) -> Self {
        Self {
            app,
            provider_runtime_lanes,
            session_projection,
            agent_runtime_projection,
            provider_run_projection,
            terminal_stream,
        }
    }

    pub(crate) async fn execute(
        &self,
        request: PumpTerminalOutputRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let Some(session) = self.session_projection.get(&request.session_id) else {
            let mut app = self.app.lock().await;
            let records = pump_terminal_output_for_attachment(
                &mut app,
                &request.session_id,
                &request.attachment_id,
            )?;
            if let Ok(session) = app.local_api_session_snapshot(&request.session_id) {
                self.agent_runtime_projection.update_session(&session);
                self.session_projection.update(session);
            }
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
        {
            let mut app = self.app.lock().await;
            let _ = ProviderOutputPump::new(&mut app).pump_provider_output(
                ProviderOutputPumpRequest {
                    session_id: &request.session_id,
                    provider_run_id: &provider_run_id,
                    recipient_attachment_ids,
                },
            )?;
            if let Ok(session) = app.local_api_session_snapshot(&request.session_id) {
                self.agent_runtime_projection.update_session(&session);
                self.session_projection.update(session);
            }
        }
        Ok(LocalDaemonResponse::TerminalOutput {
            records: self
                .terminal_stream
                .drain_output_records(&request.session_id, &request.attachment_id),
        })
    }
}
