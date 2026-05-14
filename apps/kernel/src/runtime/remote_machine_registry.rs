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
use crate::runtime::projection::{DaemonConfigProjectionStore, ProviderCatalogProjectionStore};

pub(crate) async fn execute_remote_machine_registry_request(
    app: &Arc<Mutex<DaemonApp>>,
    config_projection: &DaemonConfigProjectionStore,
    provider_catalog_projection: &ProviderCatalogProjectionStore,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::ApproveRemoteMachine(request) => {
            execute_approve_remote_machine_request(
                app,
                config_projection,
                provider_catalog_projection,
                request,
            )
            .await
        }
        LocalDaemonRequest::ForgetRemoteMachine(request) => {
            execute_forget_remote_machine_request(
                app,
                config_projection,
                provider_catalog_projection,
                request,
            )
            .await
        }
        LocalDaemonRequest::RenameRemoteMachine(request) => {
            execute_rename_remote_machine_request(
                app,
                config_projection,
                provider_catalog_projection,
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
    request: ApproveRemoteMachineRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let config = config_projection.snapshot();
    let live = crate::transport::relay_discovery::list_live_machines(&config)
        .await
        .unwrap_or_default();
    let machine = resolve_machine_for_registry(&request.machine_ref, &live)?;
    crate::config::DaemonConfig::approve_remote_machine(
        machine.machine_id.clone(),
        machine.machine_alias.clone(),
    )?;
    invalidate_provider_catalog_caches(app, provider_catalog_projection).await;
    let machine = record_for_machine_id(machine.machine_id, live, &config.host_machine_id)?;
    Ok(LocalDaemonResponse::RemoteMachineApproved { machine })
}

pub(crate) async fn execute_forget_remote_machine_request(
    app: &Arc<Mutex<DaemonApp>>,
    config_projection: &DaemonConfigProjectionStore,
    provider_catalog_projection: &ProviderCatalogProjectionStore,
    request: ForgetRemoteMachineRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let config = config_projection.snapshot();
    let live = crate::transport::relay_discovery::list_live_machines(&config)
        .await
        .unwrap_or_default();
    let machine = resolve_machine_id_for_registry(&request.machine_ref, &live)?;
    let saved = crate::config::DaemonConfig::forget_remote_machine(machine.clone())?;
    invalidate_provider_catalog_caches(app, provider_catalog_projection).await;
    let machine = forgotten_machine_record(machine, saved.alias, live, &config.host_machine_id);
    Ok(LocalDaemonResponse::RemoteMachineForgotten { machine })
}

pub(crate) async fn execute_rename_remote_machine_request(
    app: &Arc<Mutex<DaemonApp>>,
    config_projection: &DaemonConfigProjectionStore,
    provider_catalog_projection: &ProviderCatalogProjectionStore,
    request: RenameRemoteMachineRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let config = config_projection.snapshot();
    let live = crate::transport::relay_discovery::list_live_machines(&config)
        .await
        .unwrap_or_default();
    let machine = resolve_machine_id_for_registry(&request.machine_ref, &live)?;
    crate::config::DaemonConfig::rename_remote_machine(machine.clone(), request.alias)?;
    invalidate_provider_catalog_caches(app, provider_catalog_projection).await;
    let machine = record_for_machine_id(machine, live, &config.host_machine_id)?;
    Ok(LocalDaemonResponse::RemoteMachineRenamed { machine })
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
