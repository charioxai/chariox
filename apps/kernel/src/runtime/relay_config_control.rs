use std::sync::Arc;

use tokio::sync::RwLock;

use crate::error::DaemonError;
use crate::local::{ConfigureRelayRequest, LocalDaemonRequest, LocalDaemonResponse, RelayStatus};
use crate::runtime::projection::{DaemonConfigProjectionStore, ProviderCatalogProjectionStore};
use crate::runtime::remote_relay_inventory::projected_relay_status;
use crate::runtime::state::KernelRuntimeState;
use crate::transport::relay_client::RelayClientState;

async fn projected_relay_status_response(
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
    runtime_state: &KernelRuntimeState,
    relay_state: Arc<RwLock<RelayClientState>>,
    config_projection: &DaemonConfigProjectionStore,
    provider_catalog_projection: &ProviderCatalogProjectionStore,
    request: ConfigureRelayRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    runtime_state
        .configure_relay(request.relay_url, request.relay_token, true)
        .await?;
    provider_catalog_projection.invalidate();
    Ok(LocalDaemonResponse::RelayConfigured {
        status: projected_relay_status_view(relay_state, config_projection.clone()).await,
    })
}

pub(crate) async fn execute_relay_config_request(
    runtime_state: &KernelRuntimeState,
    relay_state: Arc<RwLock<RelayClientState>>,
    config_projection: &DaemonConfigProjectionStore,
    provider_catalog_projection: &ProviderCatalogProjectionStore,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::RelayStatus(_) => {
            projected_relay_status_response(relay_state, config_projection.clone()).await
        }
        LocalDaemonRequest::ConfigureRelay(request) => {
            execute_configure_relay_request(
                runtime_state,
                relay_state,
                config_projection,
                provider_catalog_projection,
                request,
            )
            .await
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "relay config request",
            message: "unsupported relay config request".to_string(),
        }),
    }
}
