use std::sync::Arc;

use tokio::sync::Mutex;

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::local::{
    GetProviderRunRequest, LocalDaemonRequest, LocalDaemonResponse, LogoutProviderRequest,
    UpdateProviderRunSelectionRequest,
};
use crate::runtime::projection::{ProviderCatalogProjectionStore, ProviderRunProjectionStore};
use crate::runtime::provider_auth_control::execute_logout_provider_request as execute_provider_logout;

pub(crate) async fn execute_provider_run_request(
    app: &Arc<Mutex<DaemonApp>>,
    provider_catalog_projection: &ProviderCatalogProjectionStore,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::GetProviderRun(request) => {
            execute_get_provider_run_request(app, request).await
        }
        LocalDaemonRequest::UpdateProviderRunSelection(request) => {
            execute_update_provider_run_selection_request(app, request).await
        }
        LocalDaemonRequest::LogoutProvider(request) => {
            execute_logout_provider_and_invalidate_catalog_request(
                app,
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
    app: &Arc<Mutex<DaemonApp>>,
    request: GetProviderRunRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let mut app = app.lock().await;
    crate::local::provider_requests::get_provider_run_response(&mut app, request)
}

pub(crate) async fn execute_update_provider_run_selection_request(
    app: &Arc<Mutex<DaemonApp>>,
    request: UpdateProviderRunSelectionRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let mut app = app.lock().await;
    crate::local::provider_requests::update_provider_run_selection_response(&mut app, request)
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
    if provider_run.adapter_key() == "opencode" {
        return Ok(None);
    }
    Ok(Some(LocalDaemonResponse::ProviderRun { provider_run }))
}

pub(crate) async fn execute_logout_provider_and_invalidate_catalog_request(
    app: &Arc<Mutex<DaemonApp>>,
    provider_catalog_projection: &ProviderCatalogProjectionStore,
    request: LogoutProviderRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let response = execute_provider_logout(request).await?;
    invalidate_provider_catalog_caches(app, provider_catalog_projection).await;
    Ok(response)
}

pub(crate) async fn invalidate_provider_catalog_caches(
    app: &Arc<Mutex<DaemonApp>>,
    provider_catalog_projection: &ProviderCatalogProjectionStore,
) {
    provider_catalog_projection.invalidate();
    if let Ok(mut app) = app.try_lock() {
        app.invalidate_provider_catalog_cache();
    }
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
