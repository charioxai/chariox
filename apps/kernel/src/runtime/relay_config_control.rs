use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::local::{ConfigureRelayRequest, LocalDaemonResponse, RelayStatus};
use crate::runtime::projection::{DaemonConfigProjectionStore, ProviderCatalogProjectionStore};
use crate::runtime::remote_relay_inventory::projected_relay_status;
use crate::transport::relay_client::RelayClientState;

pub(crate) async fn projected_relay_status_response(
    relay_state: Arc<RwLock<RelayClientState>>,
    config_projection: DaemonConfigProjectionStore,
) -> Result<LocalDaemonResponse, DaemonError> {
    Ok(LocalDaemonResponse::RelayStatus {
        status: projected_relay_status_view(relay_state, config_projection).await,
    })
}

pub(crate) async fn projected_relay_status_view(
    relay_state: Arc<RwLock<RelayClientState>>,
    config_projection: DaemonConfigProjectionStore,
) -> RelayStatus {
    projected_relay_status(relay_state, config_projection).await
}

pub(crate) async fn execute_configure_relay_request(
    app: &Arc<Mutex<DaemonApp>>,
    relay_state: Arc<RwLock<RelayClientState>>,
    config_projection: &DaemonConfigProjectionStore,
    provider_catalog_projection: &ProviderCatalogProjectionStore,
    request: ConfigureRelayRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    {
        let mut app = app.lock().await;
        app.configure_relay(request.relay_url, request.relay_token)?;
        app.invalidate_provider_catalog_cache();
        config_projection.update(app.config().clone());
    }
    provider_catalog_projection.invalidate();
    Ok(LocalDaemonResponse::RelayConfigured {
        status: projected_relay_status_view(relay_state, config_projection.clone()).await,
    })
}
