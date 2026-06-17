use std::sync::Arc;

use tokio::sync::RwLock;

use crate::error::DaemonError;
use crate::local::{
    ListRemoteMachineKernelsRequest, ListRemoteMachinesRequest, LocalDaemonRequest,
    LocalDaemonResponse, RelayStatus,
};
use crate::runtime::projection::{
    DaemonConfigProjectionStore, RemoteRelayInventoryProjectionStore,
};
use crate::transport::relay_client::{refresh_remote_inventory_projection, RelayClientState};

const REMOTE_RELAY_INVENTORY_REFRESH_COOLDOWN_MS: u64 = 1_000;

pub(crate) async fn projected_remote_machines_response(
    relay_state: Arc<RwLock<RelayClientState>>,
    config_projection: DaemonConfigProjectionStore,
    remote_relay_inventory_projection: RemoteRelayInventoryProjectionStore,
) -> Result<LocalDaemonResponse, DaemonError> {
    request_remote_relay_inventory_projection_refresh(
        relay_state,
        config_projection,
        remote_relay_inventory_projection.clone(),
    )
    .await;
    let (machines, _) = remote_relay_inventory_projection.snapshot();
    Ok(LocalDaemonResponse::RemoteMachinesListed { machines })
}

pub(crate) async fn execute_list_remote_machines_request(
    relay_state: Arc<RwLock<RelayClientState>>,
    config_projection: DaemonConfigProjectionStore,
    remote_relay_inventory_projection: RemoteRelayInventoryProjectionStore,
    _request: ListRemoteMachinesRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    projected_remote_machines_response(
        relay_state,
        config_projection,
        remote_relay_inventory_projection,
    )
    .await
}

pub(crate) async fn projected_remote_machine_kernels_response(
    relay_state: Arc<RwLock<RelayClientState>>,
    config_projection: DaemonConfigProjectionStore,
    remote_relay_inventory_projection: RemoteRelayInventoryProjectionStore,
    machine_ref: String,
) -> Result<LocalDaemonResponse, DaemonError> {
    request_remote_relay_inventory_projection_refresh(
        relay_state,
        config_projection,
        remote_relay_inventory_projection.clone(),
    )
    .await;
    let machine_ref =
        crate::local::provider_requests::resolve_registered_or_raw_machine_ref(&machine_ref);
    let (_, kernels) = remote_relay_inventory_projection.snapshot();
    let kernels = kernels
        .into_iter()
        .filter(|kernel| {
            kernel.machine_id == machine_ref
                || kernel.machine_alias.as_deref() == Some(machine_ref.as_str())
                || kernel.relay_alias.as_deref() == Some(machine_ref.as_str())
                || kernel.kernel_alias.as_deref() == Some(machine_ref.as_str())
        })
        .collect();
    Ok(LocalDaemonResponse::RemoteMachineKernelsListed {
        machine_ref,
        kernels,
    })
}

pub(crate) async fn execute_list_remote_machine_kernels_request(
    relay_state: Arc<RwLock<RelayClientState>>,
    config_projection: DaemonConfigProjectionStore,
    remote_relay_inventory_projection: RemoteRelayInventoryProjectionStore,
    request: ListRemoteMachineKernelsRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    projected_remote_machine_kernels_response(
        relay_state,
        config_projection,
        remote_relay_inventory_projection,
        request.machine_ref,
    )
    .await
}

pub(crate) async fn execute_remote_relay_inventory_request(
    relay_state: Arc<RwLock<RelayClientState>>,
    config_projection: DaemonConfigProjectionStore,
    remote_relay_inventory_projection: RemoteRelayInventoryProjectionStore,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::ListRemoteMachines(request) => {
            execute_list_remote_machines_request(
                relay_state,
                config_projection,
                remote_relay_inventory_projection,
                request,
            )
            .await
        }
        LocalDaemonRequest::ListRemoteMachineKernels(request) => {
            execute_list_remote_machine_kernels_request(
                relay_state,
                config_projection,
                remote_relay_inventory_projection,
                request,
            )
            .await
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "remote relay inventory request",
            message: "unsupported remote relay inventory request".to_string(),
        }),
    }
}

pub(crate) async fn projected_relay_status(
    relay_state: Arc<RwLock<RelayClientState>>,
    config_projection: DaemonConfigProjectionStore,
) -> RelayStatus {
    let config = config_projection.snapshot();
    let connected = relay_state.read().await.connected();
    RelayStatus {
        configured: config.relay_url.is_some() && config.relay_token.is_some(),
        connected,
        relay_url: config.relay_url,
        relay_token_configured: config.relay_token.is_some(),
        daemon_id: config.daemon_id,
        daemon_alias: config.daemon_alias,
        machine_id: config.host_machine_id,
        machine_alias: config.host_machine_alias,
    }
}

async fn request_remote_relay_inventory_projection_refresh(
    relay_state: Arc<RwLock<RelayClientState>>,
    config_projection: DaemonConfigProjectionStore,
    remote_relay_inventory_projection: RemoteRelayInventoryProjectionStore,
) {
    let connected = relay_state.read().await.connected();
    if !connected {
        return;
    }
    let config = config_projection.snapshot();
    let now_ms = crate::session::unix_epoch_ms();
    let stale_after_ms = (config.relay_heartbeat_ms.saturating_mul(2)).max(1_000);
    if !remote_relay_inventory_projection.should_request_refresh(
        now_ms,
        stale_after_ms,
        REMOTE_RELAY_INVENTORY_REFRESH_COOLDOWN_MS,
    ) {
        return;
    }
    tokio::spawn(async move {
        if let Err(error) = refresh_remote_inventory_projection(
            config_projection,
            remote_relay_inventory_projection,
        )
        .await
        {
            crate::logging::warn_with_fields(
                "daemon.router",
                "remote relay inventory refresh on demand failed",
                serde_json::json!({
                    "error": error.to_string(),
                    "stale_after_ms": stale_after_ms,
                    "cooldown_ms": REMOTE_RELAY_INVENTORY_REFRESH_COOLDOWN_MS,
                }),
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DaemonConfig;

    #[tokio::test]
    async fn relay_status_projects_config_and_connection_state() {
        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some("wss://relay.example".to_string());
        config.relay_token = Some("token".to_string());
        config.daemon_id = "daemon-1".to_string();
        config.host_machine_id = "machine-1".to_string();
        config.host_machine_alias = Some("laptop".to_string());

        let status = projected_relay_status(
            Arc::new(RwLock::new(RelayClientState::default())),
            DaemonConfigProjectionStore::new(config),
        )
        .await;

        assert!(status.configured);
        assert!(!status.connected);
        assert_eq!(status.relay_url.as_deref(), Some("wss://relay.example"));
        assert!(status.relay_token_configured);
        assert_eq!(status.daemon_id, "daemon-1");
        assert_eq!(status.machine_id, "machine-1");
        assert_eq!(status.machine_alias.as_deref(), Some("laptop"));
    }
}
