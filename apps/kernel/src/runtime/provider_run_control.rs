use std::sync::Arc;

use tokio::sync::Mutex;

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::local::{
    GetProviderRunRequest, LocalDaemonResponse, LogoutProviderRequest,
    UpdateProviderRunSelectionRequest,
};
use crate::runtime::projection::ProviderCatalogProjectionStore;
use crate::runtime::provider_auth_control::execute_logout_provider_request as execute_provider_logout;

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
