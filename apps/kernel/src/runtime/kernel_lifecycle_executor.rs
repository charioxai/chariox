use crate::error::DaemonError;
use crate::local::{DeleteKernelRequest, LocalDaemonResponse};
use crate::runtime::projection::DaemonConfigProjectionStore;
use crate::runtime::state::KernelRuntimeState;

pub(crate) async fn execute_delete_kernel_request(
    config_projection: &DaemonConfigProjectionStore,
    runtime_state: &KernelRuntimeState,
    _request: DeleteKernelRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let kernel_id = config_projection.snapshot().daemon_id;
    let deleted_sessions = runtime_state.delete_current_kernel_sessions().await?;
    Ok(LocalDaemonResponse::KernelDeleted {
        kernel_id,
        deleted_sessions,
    })
}
