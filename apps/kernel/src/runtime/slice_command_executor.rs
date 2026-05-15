use std::sync::Arc;

use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::local::{
    CreateSliceRequest, ImportSliceProviderAuthRequest, ListSlicesRequest, LocalDaemonRequest,
    LocalDaemonResponse, SliceRefRequest,
};
use crate::runtime::projection::DaemonConfigProjectionStore;

pub(crate) async fn execute_slice_request(
    app: &Arc<Mutex<DaemonApp>>,
    config_projection: &DaemonConfigProjectionStore,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::ListSlices(request) => execute_list_slices_request(app, request).await,
        LocalDaemonRequest::CreateSlice(request) => {
            execute_create_slice_request(app, request).await
        }
        LocalDaemonRequest::GetSlice(request) => execute_get_slice_request(app, request).await,
        LocalDaemonRequest::StartSlice(request) => {
            execute_start_slice_request(app, config_projection, request).await
        }
        LocalDaemonRequest::StopSlice(request) => execute_stop_slice_request(app, request).await,
        LocalDaemonRequest::DeleteSlice(request) => {
            execute_delete_slice_request(app, request).await
        }
        LocalDaemonRequest::ImportSliceProviderAuth(request) => {
            execute_import_slice_provider_auth_request(app, request).await
        }
        LocalDaemonRequest::GetSliceDisplayEndpoint(request) => {
            execute_get_slice_display_endpoint_request(app, request).await
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "slice request",
            message: "unsupported slice request".to_string(),
        }),
    }
}

pub(crate) async fn execute_list_slices_request(
    app: &Arc<Mutex<DaemonApp>>,
    _request: ListSlicesRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let slices = {
        let app = app.lock().await;
        app.slices().list()
    };
    Ok(LocalDaemonResponse::SlicesListed { slices })
}

pub(crate) async fn execute_create_slice_request(
    app: &Arc<Mutex<DaemonApp>>,
    request: CreateSliceRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let slice = {
        let app = app.lock().await;
        let slice = app.slices().create(
            &app.config().daemon_id,
            &app.config().host_machine_id,
            crate::slice::CreateSliceInput {
                name: request.name,
                backend: request.backend,
                os: request.os,
                workspace_mount: request.workspace_mount,
                worker_kernel_ref: request.worker_kernel_ref,
                display_url: request.display_url,
                now_ms: crate::session::unix_epoch_ms(),
            },
        )?;
        app.durable_state_store().append_event(
            "slice.created",
            Some(slice.id.clone()),
            serde_json::json!({ "slice": &slice }),
        )?;
        slice
    };
    Ok(LocalDaemonResponse::SliceCreated { slice })
}

pub(crate) async fn execute_get_slice_request(
    app: &Arc<Mutex<DaemonApp>>,
    request: SliceRefRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let slice = {
        let app = app.lock().await;
        app.slices().resolve(&request.slice_ref)?
    };
    Ok(LocalDaemonResponse::Slice { slice })
}

pub(crate) async fn execute_start_slice_request(
    app: &Arc<Mutex<DaemonApp>>,
    config_projection: &DaemonConfigProjectionStore,
    request: SliceRefRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let initial_record = {
        let app = app.lock().await;
        app.slices().resolve(&request.slice_ref)?
    };
    let relay = local_docker_slice_relay(config_projection, &initial_record);
    let initial_slice = {
        let app = app.lock().await;
        let relay_endpoint = crate::slice::SliceRelayEndpoint {
            url: relay.relay_url.clone(),
            private: relay.container_relay_url.is_none(),
        };
        app.slices().set_relay_endpoint(
            &request.slice_ref,
            Some(relay_endpoint),
            crate::session::unix_epoch_ms(),
        )?;
        let slice = app.slices().set_status(
            &request.slice_ref,
            crate::slice::SliceStatus::Starting,
            crate::session::unix_epoch_ms(),
        )?;
        app.durable_state_store().append_event(
            "slice.updated",
            Some(slice.id.clone()),
            serde_json::json!({ "slice": &slice }),
        )?;
        slice
    };
    let supervisor_slice = initial_slice.clone();
    let supervisor_relay = Some(relay.clone());
    let docker_options = {
        let app = app.lock().await;
        crate::slice::LocalDockerSliceOptions::from_config(app.config())
    };
    let supervisor_result = tokio::task::spawn_blocking(move || {
        crate::slice::run_local_docker_slice_action(
            &supervisor_slice,
            crate::slice::LocalDockerSliceAction::Provision,
            supervisor_relay,
            &docker_options,
        )
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "slice.start",
        message: format!("slice supervisor task failed: {error}"),
    })?;
    if let Err(error) = supervisor_result {
        let app = app.lock().await;
        let _ = app.slices().set_status(
            &request.slice_ref,
            crate::slice::SliceStatus::Unhealthy,
            crate::session::unix_epoch_ms(),
        );
        if let Ok(slice) = app.slices().resolve(&request.slice_ref) {
            let _ = app.durable_state_store().append_event(
                "slice.updated",
                Some(slice.id.clone()),
                serde_json::json!({ "slice": &slice }),
            );
        }
        return Err(error);
    }
    let discovered = discover_started_slice_worker(config_projection, &initial_slice, &relay)
        .await
        .ok();
    let slice = {
        let app = app.lock().await;
        let slice = app.slices().set_status(
            &request.slice_ref,
            crate::slice::SliceStatus::Running,
            crate::session::unix_epoch_ms(),
        )?;
        let slice = if let Some(worker) = discovered {
            app.slices().set_worker_presence(
                &request.slice_ref,
                Some(worker.kernel_id),
                Some(worker.machine_id),
                worker.available_providers,
                crate::session::unix_epoch_ms(),
            )?
        } else {
            slice
        };
        app.durable_state_store().append_event(
            "slice.updated",
            Some(slice.id.clone()),
            serde_json::json!({ "slice": &slice }),
        )?;
        slice
    };
    Ok(LocalDaemonResponse::SliceStarted { slice })
}

pub(crate) async fn execute_stop_slice_request(
    app: &Arc<Mutex<DaemonApp>>,
    request: SliceRefRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let resolved_slice = {
        let app = app.lock().await;
        app.slices().resolve(&request.slice_ref)?
    };
    let docker_options = {
        let app = app.lock().await;
        crate::slice::LocalDockerSliceOptions::from_config(app.config())
    };
    let supervisor_result = tokio::task::spawn_blocking(move || {
        crate::slice::run_local_docker_slice_action(
            &resolved_slice,
            crate::slice::LocalDockerSliceAction::Stop,
            None,
            &docker_options,
        )
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "slice.stop",
        message: format!("slice supervisor task failed: {error}"),
    })?;
    supervisor_result?;
    let slice = {
        let app = app.lock().await;
        let slice = app.slices().set_status(
            &request.slice_ref,
            crate::slice::SliceStatus::Stopped,
            crate::session::unix_epoch_ms(),
        )?;
        app.durable_state_store().append_event(
            "slice.updated",
            Some(slice.id.clone()),
            serde_json::json!({ "slice": &slice }),
        )?;
        slice
    };
    Ok(LocalDaemonResponse::SliceStopped { slice })
}

pub(crate) async fn execute_delete_slice_request(
    app: &Arc<Mutex<DaemonApp>>,
    request: SliceRefRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let resolved_slice = {
        let app = app.lock().await;
        app.slices().resolve(&request.slice_ref)?
    };
    if resolved_slice.backend == crate::slice::SliceBackendKind::LocalDocker {
        let docker_options = {
            let app = app.lock().await;
            crate::slice::LocalDockerSliceOptions::from_config(app.config())
        };
        let supervisor_result = tokio::task::spawn_blocking(move || {
            crate::slice::run_local_docker_slice_action(
                &resolved_slice,
                crate::slice::LocalDockerSliceAction::Destroy,
                None,
                &docker_options,
            )
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "slice.delete",
            message: format!("slice supervisor task failed: {error}"),
        })?;
        supervisor_result?;
    }
    let slice = {
        let app = app.lock().await;
        let slice = app.slices().delete(&request.slice_ref)?;
        app.durable_state_store().append_event(
            "slice.deleted",
            Some(slice.id.clone()),
            serde_json::json!({ "slice": &slice }),
        )?;
        slice
    };
    Ok(LocalDaemonResponse::SliceDeleted { slice })
}

pub(crate) async fn execute_import_slice_provider_auth_request(
    app: &Arc<Mutex<DaemonApp>>,
    request: ImportSliceProviderAuthRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let slice = {
        let app = app.lock().await;
        app.slices().resolve(&request.slice_ref)?
    };
    if slice.backend == crate::slice::SliceBackendKind::LocalDocker {
        let docker_options = {
            let app = app.lock().await;
            crate::slice::LocalDockerSliceOptions::from_config(app.config())
        };
        let resolved_slice = slice.clone();
        tokio::task::spawn_blocking(move || {
            crate::slice::run_local_docker_slice_action(
                &resolved_slice,
                crate::slice::LocalDockerSliceAction::ImportProviderAuth,
                None,
                &docker_options,
            )
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "slice.auth.import",
            message: format!("slice auth import task failed: {error}"),
        })??;
        return Ok(LocalDaemonResponse::SliceProviderAuthImported {
            slice,
            provider: request.provider,
            status: "imported".to_string(),
        });
    }
    Ok(LocalDaemonResponse::SliceProviderAuthImported {
        slice,
        provider: request.provider,
        status: "not_implemented".to_string(),
    })
}

pub(crate) async fn execute_get_slice_display_endpoint_request(
    app: &Arc<Mutex<DaemonApp>>,
    request: SliceRefRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let endpoint = {
        let app = app.lock().await;
        app.slices().display_endpoint(&request.slice_ref)?
    };
    Ok(LocalDaemonResponse::SliceDisplayEndpoint { endpoint })
}

async fn discover_started_slice_worker(
    config_projection: &DaemonConfigProjectionStore,
    slice: &crate::slice::SliceRecord,
    relay: &crate::slice::LocalDockerSliceRelay,
) -> Result<arroba_relay::protocol::RelayKernelPresence, DaemonError> {
    let mut config = config_projection.snapshot();
    config.relay_url = Some(relay.relay_url.clone());
    config.relay_token = Some(relay.relay_token.clone());
    config.cloud_relay = None;
    let worker_ref = slice.worker_kernel_ref.clone();
    let mut last_error = None;
    for _ in 0..20 {
        match crate::transport::relay_discovery::get_live_kernel(&config, &worker_ref).await {
            Ok(kernel) => return Ok(kernel),
            Err(error) => {
                last_error = Some(error);
                sleep(Duration::from_millis(250)).await;
            }
        }
    }
    Err(last_error.unwrap_or_else(|| DaemonError::LocalTransport {
        operation: "slice.discover_worker",
        message: format!("slice `{}` worker did not appear", slice.name),
    }))
}

fn local_docker_slice_relay(
    config_projection: &DaemonConfigProjectionStore,
    slice: &crate::slice::SliceRecord,
) -> crate::slice::LocalDockerSliceRelay {
    let config = config_projection.snapshot();
    match (config.relay_url, config.relay_token) {
        (Some(relay_url), Some(relay_token)) => crate::slice::LocalDockerSliceRelay {
            relay_url: relay_url.clone(),
            container_relay_url: Some(relay_url),
            relay_token,
        },
        _ => crate::slice::local_docker_private_relay(slice),
    }
}
