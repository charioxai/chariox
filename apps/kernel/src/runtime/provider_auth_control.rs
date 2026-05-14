use crate::error::DaemonError;
use crate::local::provider_requests::{
    logout_provider_response, provider_auth_status_response, start_provider_login_response,
};
use crate::local::{
    GetProviderAuthStatusRequest, LocalDaemonRequest, LocalDaemonResponse, LogoutProviderRequest,
    StartProviderLoginRequest,
};

pub(crate) async fn execute_provider_auth_request(
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::GetProviderAuthStatus(request) => {
            execute_get_provider_auth_status_request(request).await
        }
        LocalDaemonRequest::StartProviderLogin(request) => {
            execute_start_provider_login_request(request).await
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "provider auth request",
            message: "unsupported provider auth request".to_string(),
        }),
    }
}

pub(crate) async fn execute_get_provider_auth_status_request(
    request: GetProviderAuthStatusRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    tokio::task::spawn_blocking(move || provider_auth_status_response(request))
        .await
        .map_err(|error| provider_auth_task_error("get provider auth status", error))?
}

pub(crate) async fn execute_start_provider_login_request(
    request: StartProviderLoginRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    tokio::task::spawn_blocking(move || start_provider_login_response(request))
        .await
        .map_err(|error| provider_auth_task_error("start provider login", error))?
}

pub(crate) async fn execute_logout_provider_request(
    request: LogoutProviderRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    tokio::task::spawn_blocking(move || logout_provider_response(request))
        .await
        .map_err(|error| provider_auth_task_error("logout provider", error))?
}

fn provider_auth_task_error(operation: &'static str, error: tokio::task::JoinError) -> DaemonError {
    DaemonError::LocalTransport {
        operation,
        message: error.to_string(),
    }
}
