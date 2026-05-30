use tokio::time::{sleep, Duration};

use crate::error::DaemonError;
use crate::local::{
    CreateSliceRequest, ImportSliceProviderAuthRequest, ListSlicesRequest, LocalDaemonRequest,
    LocalDaemonResponse, SetSliceProviderAuthAliasRequest, SliceRefRequest,
    StartSliceProviderLoginRequest,
};
use crate::runtime::projection::DaemonConfigProjectionStore;
use crate::runtime::state::KernelRuntimeState;

pub(crate) async fn execute_slice_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::ListSlices(request) => {
            execute_list_slices_request(runtime_state, request).await
        }
        LocalDaemonRequest::CreateSlice(request) => {
            execute_create_slice_request(runtime_state, request).await
        }
        LocalDaemonRequest::GetSlice(request) => {
            execute_get_slice_request(runtime_state, request).await
        }
        LocalDaemonRequest::StartSlice(request) => {
            execute_start_slice_request(runtime_state, config_projection, request).await
        }
        LocalDaemonRequest::StopSlice(request) => {
            execute_stop_slice_request(runtime_state, config_projection, request).await
        }
        LocalDaemonRequest::DeleteSlice(request) => {
            execute_delete_slice_request(runtime_state, config_projection, request).await
        }
        LocalDaemonRequest::ImportSliceProviderAuth(request) => {
            execute_import_slice_provider_auth_request(runtime_state, config_projection, request)
                .await
        }
        LocalDaemonRequest::StartSliceProviderLogin(request) => {
            execute_start_slice_provider_login_request(runtime_state, config_projection, request)
                .await
        }
        LocalDaemonRequest::SetSliceProviderAuthAlias(request) => {
            execute_set_slice_provider_auth_alias_request(runtime_state, request).await
        }
        LocalDaemonRequest::GetSliceDisplayEndpoint(request) => {
            execute_get_slice_display_endpoint_request(runtime_state, request).await
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "slice request",
            message: "unsupported slice request".to_string(),
        }),
    }
}

pub(crate) async fn execute_list_slices_request(
    runtime_state: &KernelRuntimeState,
    _request: ListSlicesRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let slices = runtime_state.list_slices();
    Ok(LocalDaemonResponse::SlicesListed { slices })
}

pub(crate) async fn execute_create_slice_request(
    runtime_state: &KernelRuntimeState,
    request: CreateSliceRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let slice = runtime_state.create_slice(request).await?;
    Ok(LocalDaemonResponse::SliceCreated { slice })
}

pub(crate) async fn execute_get_slice_request(
    runtime_state: &KernelRuntimeState,
    request: SliceRefRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let slice = runtime_state.resolve_slice(&request.slice_ref)?;
    Ok(LocalDaemonResponse::Slice { slice })
}

pub(crate) async fn execute_start_slice_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    request: SliceRefRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let initial_record = runtime_state.resolve_slice(&request.slice_ref)?;
    let relay = local_docker_slice_relay(config_projection, &initial_record);
    let initial_slice = runtime_state.mark_slice_starting(
        &request.slice_ref,
        crate::slice::SliceRelayEndpoint {
            url: relay.relay_url.clone(),
            private: relay.container_relay_url.is_none(),
        },
    )?;
    let supervisor_slice = initial_slice.clone();
    let supervisor_relay = Some(relay.clone());
    let docker_options =
        crate::slice::LocalDockerSliceOptions::from_config(&config_projection.snapshot());
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
        let _ = runtime_state
            .set_slice_status(&request.slice_ref, crate::slice::SliceStatus::Unhealthy);
        return Err(error);
    }
    let discovered = discover_started_slice_worker(config_projection, &initial_slice, &relay)
        .await
        .ok();
    let slice = runtime_state.mark_slice_running(&request.slice_ref, discovered)?;
    Ok(LocalDaemonResponse::SliceStarted { slice })
}

pub(crate) async fn execute_stop_slice_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    request: SliceRefRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let resolved_slice = runtime_state.resolve_slice(&request.slice_ref)?;
    let docker_options =
        crate::slice::LocalDockerSliceOptions::from_config(&config_projection.snapshot());
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
    let slice =
        runtime_state.set_slice_status(&request.slice_ref, crate::slice::SliceStatus::Stopped)?;
    Ok(LocalDaemonResponse::SliceStopped { slice })
}

pub(crate) async fn execute_delete_slice_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    request: SliceRefRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let resolved_slice = runtime_state.resolve_slice(&request.slice_ref)?;
    if !resolved_slice.agent_ids.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "slice.delete",
            message: format!(
                "slice `{}` still has {} active agent(s)",
                resolved_slice.name,
                resolved_slice.agent_ids.len()
            ),
        });
    }
    if resolved_slice.backend == crate::slice::SliceBackendKind::LocalDocker {
        let docker_options =
            crate::slice::LocalDockerSliceOptions::from_config(&config_projection.snapshot());
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
    let slice = runtime_state.delete_slice(&request.slice_ref)?;
    Ok(LocalDaemonResponse::SliceDeleted { slice })
}

pub(crate) async fn execute_import_slice_provider_auth_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    request: ImportSliceProviderAuthRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let slice = runtime_state.resolve_slice(&request.slice_ref)?;
    if slice.backend == crate::slice::SliceBackendKind::LocalDocker {
        let docker_options =
            crate::slice::LocalDockerSliceOptions::from_config(&config_projection.snapshot());
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
        let provider_auth = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .map(|home| crate::slice_provider_auth::inspect_home_provider_auth(&home))
            .unwrap_or_default();
        let slice = runtime_state.set_slice_provider_auth(&request.slice_ref, provider_auth)?;
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
    runtime_state: &KernelRuntimeState,
    request: SliceRefRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let endpoint = runtime_state.slice_display_endpoint(&request.slice_ref)?;
    Ok(LocalDaemonResponse::SliceDisplayEndpoint { endpoint })
}

pub(crate) async fn execute_start_slice_provider_login_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    request: StartSliceProviderLoginRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let slice = runtime_state.resolve_slice(&request.slice_ref)?;
    if slice.backend != crate::slice::SliceBackendKind::LocalDocker {
        return Err(DaemonError::LocalTransport {
            operation: "slice.auth.login",
            message: format!(
                "slice provider login is only implemented for local Docker slices, got `{:?}`",
                slice.backend
            ),
        });
    }
    let docker_options =
        crate::slice::LocalDockerSliceOptions::from_config(&config_projection.snapshot());
    let resolved_slice = slice.clone();
    let provider = request.provider.clone();
    let login = tokio::task::spawn_blocking(move || {
        crate::slice::start_local_docker_slice_provider_login(
            &resolved_slice,
            &provider,
            &docker_options,
        )
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "slice.auth.login",
        message: format!("slice provider login task failed: {error}"),
    })??;
    Ok(LocalDaemonResponse::SliceProviderLoginStarted { slice, login })
}

pub(crate) async fn execute_set_slice_provider_auth_alias_request(
    runtime_state: &KernelRuntimeState,
    request: SetSliceProviderAuthAliasRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let slice = runtime_state.set_slice_provider_auth_alias(
        &request.slice_ref,
        &request.provider,
        request.alias.as_deref(),
    )?;
    Ok(LocalDaemonResponse::SliceProviderAuthAliasSet {
        slice,
        provider: request.provider,
        alias: request.alias,
    })
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
    _config_projection: &DaemonConfigProjectionStore,
    slice: &crate::slice::SliceRecord,
) -> crate::slice::LocalDockerSliceRelay {
    crate::slice::local_docker_private_relay(slice)
}
