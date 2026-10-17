//! MP-08 / MP-10 / MP-11: clients drain committed output; only the kernel
//! runtime pump executes providers and settles turns.
use crate::error::DaemonError;
use crate::local::{
    AppendNativeProviderOutputBatchRequest, AppendNativeProviderOutputRequest, LocalDaemonResponse,
    PumpTerminalOutputRequest,
};
use crate::runtime::projection::{AgentRuntimeProjectionStore, SessionStateProjectionStore};
use crate::runtime::state::KernelRuntimeState;

#[derive(Clone)]
pub(crate) struct TerminalOutputExecutor {
    state: KernelRuntimeState,
}

impl TerminalOutputExecutor {
    pub(crate) fn new(state: KernelRuntimeState) -> Self {
        Self { state }
    }

    pub(crate) async fn execute(
        &self,
        request: PumpTerminalOutputRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.read(&request)
    }

    pub(crate) fn projected_response(
        &self,
        request: &PumpTerminalOutputRequest,
    ) -> Option<Result<LocalDaemonResponse, DaemonError>> {
        Some(self.read(request))
    }

    fn read(
        &self,
        request: &PumpTerminalOutputRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::TerminalOutput {
            records: self
                .state
                .drain_committed_terminal_output(&request.session_id, &request.attachment_id)?,
        })
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
