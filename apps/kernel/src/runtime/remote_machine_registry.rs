use std::sync::Arc;

use tokio::sync::Mutex;

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::local::provider_requests::{
    forgotten_machine_record, record_for_machine_id, resolve_machine_for_registry,
    resolve_machine_id_for_registry,
};
use crate::local::{
    ApproveRemoteMachineRequest, ForgetRemoteMachineRequest, LocalDaemonRequest,
    LocalDaemonResponse, RenameRemoteMachineRequest,
};
use crate::runtime::projection::RemoteRelayInventoryProjectionStore;
use crate::runtime::projection::{DaemonConfigProjectionStore, ProviderCatalogProjectionStore};
use crate::runtime::state::KernelRuntimeState;

pub(crate) async fn execute_remote_machine_registry_request(
    runtime_state: &KernelRuntimeState,
    app: &Arc<Mutex<DaemonApp>>,
    config_projection: &DaemonConfigProjectionStore,
    provider_catalog_projection: &ProviderCatalogProjectionStore,
    remote_relay_inventory_projection: &RemoteRelayInventoryProjectionStore,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::ApproveRemoteMachine(request) => {
            execute_approve_remote_machine_request(
                app,
                config_projection,
                provider_catalog_projection,
                remote_relay_inventory_projection,
                request,
            )
            .await
        }
        LocalDaemonRequest::ForgetRemoteMachine(request) => {
            execute_forget_remote_machine_request(
                runtime_state,
                app,
                config_projection,
                provider_catalog_projection,
                remote_relay_inventory_projection,
                request,
            )
            .await
        }
        LocalDaemonRequest::RenameRemoteMachine(request) => {
            execute_rename_remote_machine_request(
                app,
                config_projection,
                provider_catalog_projection,
                remote_relay_inventory_projection,
                request,
            )
            .await
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "remote machine registry request",
            message: "unsupported remote machine registry request".to_string(),
        }),
    }
}

pub(crate) async fn execute_approve_remote_machine_request(
    app: &Arc<Mutex<DaemonApp>>,
    config_projection: &DaemonConfigProjectionStore,
    provider_catalog_projection: &ProviderCatalogProjectionStore,
    remote_relay_inventory_projection: &RemoteRelayInventoryProjectionStore,
    request: ApproveRemoteMachineRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let config = config_projection.snapshot();
    let live = live_registry_machines(&config).await.unwrap_or_default();
    let machine = resolve_machine_for_registry(&request.machine_ref, &live)?;
    crate::config::DaemonConfig::approve_remote_machine(
        machine.machine_id.clone(),
        machine.machine_alias.clone(),
    )?;
    invalidate_provider_catalog_caches(app, provider_catalog_projection).await;
    let machine = record_for_machine_id(machine.machine_id, live, &config.host_machine_id)?;
    remote_relay_inventory_projection.update_machine(machine.clone());
    Ok(LocalDaemonResponse::RemoteMachineApproved { machine })
}

pub(crate) async fn execute_forget_remote_machine_request(
    runtime_state: &KernelRuntimeState,
    app: &Arc<Mutex<DaemonApp>>,
    config_projection: &DaemonConfigProjectionStore,
    provider_catalog_projection: &ProviderCatalogProjectionStore,
    remote_relay_inventory_projection: &RemoteRelayInventoryProjectionStore,
    request: ForgetRemoteMachineRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let config = config_projection.snapshot();
    let live = live_registry_machines(&config).await.unwrap_or_default();
    let machine =
        resolve_machine_id_for_registry(&request.machine_ref, &live).or_else(|error| {
            runtime_state
                .resolve_leased_worker_machine_ref(&request.machine_ref)
                .ok_or(error)
        })?;
    let saved = crate::config::DaemonConfig::forget_remote_machine(machine.clone())?;
    invalidate_provider_catalog_caches(app, provider_catalog_projection).await;
    runtime_state
        .end_worker_leases_of_forgotten_machine(&machine)
        .await;
    let machine = forgotten_machine_record(machine, saved.alias, live, &config.host_machine_id);
    remote_relay_inventory_projection.remove_machine(&machine.machine_id);
    Ok(LocalDaemonResponse::RemoteMachineForgotten { machine })
}

pub(crate) async fn execute_rename_remote_machine_request(
    app: &Arc<Mutex<DaemonApp>>,
    config_projection: &DaemonConfigProjectionStore,
    provider_catalog_projection: &ProviderCatalogProjectionStore,
    remote_relay_inventory_projection: &RemoteRelayInventoryProjectionStore,
    request: RenameRemoteMachineRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let config = config_projection.snapshot();
    let live = live_registry_machines(&config).await.unwrap_or_default();
    let machine = resolve_machine_id_for_registry(&request.machine_ref, &live)?;
    crate::config::DaemonConfig::rename_remote_machine(machine.clone(), request.alias)?;
    invalidate_provider_catalog_caches(app, provider_catalog_projection).await;
    let machine = record_for_machine_id(machine, live, &config.host_machine_id)?;
    remote_relay_inventory_projection.update_machine(machine.clone());
    Ok(LocalDaemonResponse::RemoteMachineRenamed { machine })
}

// MP-08 / MP-10 / MP-11: hosted metadata reads need a client-scoped token;
// the kernel's registration/peer token cannot authorize machine discovery.
pub(crate) async fn live_registry_machines(
    config: &crate::config::DaemonConfig,
) -> Result<Vec<chariox_relay::protocol::RelayMachinePresence>, DaemonError> {
    let discovery_config =
        crate::runtime::cloud_api_client::scoped_metadata_config(config, true).await?;
    crate::transport::relay_discovery::list_live_machines(&discovery_config).await
}

async fn invalidate_provider_catalog_caches(
    app: &Arc<Mutex<DaemonApp>>,
    provider_catalog_projection: &ProviderCatalogProjectionStore,
) {
    provider_catalog_projection.invalidate();
    if let Ok(mut app) = app.try_lock() {
        app.invalidate_provider_catalog_cache();
    }
}
