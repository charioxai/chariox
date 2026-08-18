use crate::error::DaemonError;
use crate::local::provider_requests::{
    logout_provider_response, provider_auth_status_response, start_provider_login_response,
};
use crate::local::{
    GetProviderAuthStatusRequest, LocalDaemonRequest, LocalDaemonResponse, LogoutProviderRequest,
    StartProviderLoginRequest,
};
use crate::runtime::state::KernelRuntimeState;

pub(crate) async fn execute_provider_auth_request(
    runtime_state: &KernelRuntimeState,
    owner_user_id: &str,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::GetProviderAuthStatus(request) => {
            execute_get_provider_auth_status_request(runtime_state, owner_user_id, request).await
        }
        LocalDaemonRequest::StartProviderLogin(request) => {
            execute_start_provider_login_request(runtime_state, owner_user_id, request).await
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "provider auth request",
            message: "unsupported provider auth request".to_string(),
        }),
    }
}

pub(crate) async fn execute_get_provider_auth_status_request(
    runtime_state: &KernelRuntimeState,
    owner_user_id: &str,
    request: GetProviderAuthStatusRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry = runtime_state.provider_account_profile_registry().clone();
    let owner_user_id = owner_user_id.to_string();
    tokio::task::spawn_blocking(move || {
        provider_auth_status_response(&registry, &owner_user_id, request)
    })
    .await
    .map_err(|error| provider_auth_task_error("get provider auth status", error))?
}

pub(crate) async fn execute_start_provider_login_request(
    runtime_state: &KernelRuntimeState,
    owner_user_id: &str,
    request: StartProviderLoginRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry = runtime_state.provider_account_profile_registry().clone();
    let owner_user_id = owner_user_id.to_string();
    tokio::task::spawn_blocking(move || {
        start_provider_login_response(&registry, &owner_user_id, request)
    })
    .await
    .map_err(|error| provider_auth_task_error("start provider login", error))?
}

pub(crate) async fn execute_logout_provider_request(
    runtime_state: &KernelRuntimeState,
    owner_user_id: &str,
    request: LogoutProviderRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry = runtime_state.provider_account_profile_registry().clone();
    crate::runtime::provider_account_control::ensure_profile_idle(
        runtime_state,
        &registry,
        owner_user_id,
        &request.provider,
        &request.account_profile,
    )?;
    let owner_user_id = owner_user_id.to_string();
    tokio::task::spawn_blocking(move || {
        logout_provider_response(&registry, &owner_user_id, request)
    })
    .await
    .map_err(|error| provider_auth_task_error("logout provider", error))?
}

fn provider_auth_task_error(operation: &'static str, error: tokio::task::JoinError) -> DaemonError {
    DaemonError::LocalTransport {
        operation,
        message: error.to_string(),
    }
}
