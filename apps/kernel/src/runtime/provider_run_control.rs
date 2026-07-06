use crate::error::DaemonError;
use crate::local::{
    GetProviderRunRequest, LocalDaemonRequest, LocalDaemonResponse, LogoutProviderRequest,
    UpdateProviderRunSelectionRequest,
};
use crate::runtime::projection::{
    ProviderCatalogProjectionStore, ProviderProcessProjectionStore, ProviderRunProjectionStore,
};
use crate::runtime::provider_auth_control::execute_logout_provider_request as execute_provider_logout;
use crate::runtime::state::KernelRuntimeState;

pub(crate) async fn execute_provider_run_request(
    runtime_state: &KernelRuntimeState,
    provider_catalog_projection: &ProviderCatalogProjectionStore,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::GetProviderRun(request) => {
            execute_get_provider_run_request(runtime_state, request).await
        }
        LocalDaemonRequest::UpdateProviderRunSelection(request) => {
            execute_update_provider_run_selection_request(runtime_state, request).await
        }
        LocalDaemonRequest::LogoutProvider(request) => {
            execute_logout_provider_and_invalidate_catalog_request(
                provider_catalog_projection,
                request,
            )
            .await
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "provider run request",
            message: "unsupported provider run request".to_string(),
        }),
    }
}

pub(crate) async fn execute_get_provider_run_request(
    runtime_state: &KernelRuntimeState,
    request: GetProviderRunRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    runtime_state.provider_run_response(request)
}

pub(crate) async fn execute_update_provider_run_selection_request(
    runtime_state: &KernelRuntimeState,
    request: UpdateProviderRunSelectionRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    runtime_state.update_provider_run_selection_response(request)
}

pub(crate) fn projected_provider_run_response(
    provider_run_projection: &ProviderRunProjectionStore,
    request: &GetProviderRunRequest,
    caller_user_id: &str,
) -> Result<Option<LocalDaemonResponse>, DaemonError> {
    let Some(provider_run) = provider_run_projection.get(&request.provider_run_id) else {
        return Ok(None);
    };
    ensure_provider_run_visible_to_user(&provider_run, caller_user_id)?;
    if crate::provider::provider_run_refreshes_selection_on_read(&provider_run) {
        return Ok(None);
    }
    Ok(Some(LocalDaemonResponse::ProviderRun { provider_run }))
}

pub(crate) fn refresh_provider_run_projection_from_response(
    provider_run_projection: &ProviderRunProjectionStore,
    provider_process_projection: &ProviderProcessProjectionStore,
    result: &Result<LocalDaemonResponse, DaemonError>,
) {
    match result {
        Ok(LocalDaemonResponse::ProviderRun { provider_run })
        | Ok(LocalDaemonResponse::ProviderRunLaunched { provider_run })
        | Ok(LocalDaemonResponse::ProviderRunLaunchAccepted { provider_run }) => {
            provider_run_projection.update(provider_run.clone());
            provider_process_projection.invalidate();
        }
        _ => {}
    }
}

pub(crate) async fn execute_logout_provider_and_invalidate_catalog_request(
    provider_catalog_projection: &ProviderCatalogProjectionStore,
    request: LogoutProviderRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let response = execute_provider_logout(request).await?;
    invalidate_provider_catalog_caches(provider_catalog_projection);
    Ok(response)
}

pub(crate) fn invalidate_provider_catalog_caches(
    provider_catalog_projection: &ProviderCatalogProjectionStore,
) {
    provider_catalog_projection.invalidate();
}

pub(crate) fn ensure_provider_run_visible_to_user(
    provider_run: &crate::provider::RuntimeProviderRun,
    caller_user_id: &str,
) -> Result<(), DaemonError> {
    if provider_run.owned_by(caller_user_id) {
        Ok(())
    } else {
        Err(DaemonError::OwnershipAccessDenied {
            user_id: caller_user_id.to_string(),
            owner_user_id: provider_run.owner_user_id().to_string(),
            resource: format!("provider run `{}`", provider_run.id()),
            operation: "read provider run",
        })
    }
}
