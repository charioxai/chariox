use crate::error::DaemonError;
use crate::local::{DeleteKernelRequest, LocalDaemonRequest, LocalDaemonResponse};
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

pub(crate) async fn execute_kernel_lifecycle_request(
    config_projection: &DaemonConfigProjectionStore,
    runtime_state: &KernelRuntimeState,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::DeleteKernel(request) => {
            execute_delete_kernel_request(config_projection, runtime_state, request).await
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "kernel lifecycle request",
            message: "unsupported kernel lifecycle request".to_string(),
        }),
    }
}
