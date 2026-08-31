use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{sleep, timeout, Duration, Instant};

use crate::error::DaemonError;
use crate::local::{
    CreateSliceBackupRequest, CreateSliceRequest, GetSliceLogsRequest, ListSliceAuditRequest,
    ListSlicesRequest, LocalDaemonResponse, SliceRefRequest, SliceStateResetRequest,
    SliceStateSaveMode, SliceStateSaveRequest, SliceStateSaveScope, SliceStateStatusRequest,
};
use crate::runtime::projection::DaemonConfigProjectionStore;
use crate::runtime::state::KernelRuntimeState;
use crate::transport::relay_client::RelayClientState;

use super::display_endpoint::revoke_display_tunnels_for_slice;
use super::provider_auth::merge_detected_provider_auth;
use super::worker_discovery::{
    discover_started_slice_worker, provision_and_prepare_worker_discovery,
};
use crate::runtime::cloud_api_client::{
    issue_cloud_slice_recovery_token, issue_cloud_slice_runtime_token,
};
use crate::runtime::cloud_relay_connection_executor::ensure_cloud_relay_connection;
use crate::runtime::cloud_relay_control::relay_token_expiry_ms;

const HOSTED_SLICE_RELAY_RECONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const HOSTED_SLICE_RELAY_RECONNECT_POLL_INTERVAL: Duration = Duration::from_millis(50);
const HOSTED_SLICE_RELAY_RECONNECT_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

pub(super) async fn execute_list_slices_request(
    runtime_state: &KernelRuntimeState,
    _request: ListSlicesRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let candidates = runtime_state
        .list_slices()
        .into_iter()
        .filter(|slice| {
            slice.backend == crate::slice::SliceBackendKind::LocalDocker
                && slice.status == crate::slice::SliceStatus::Running
        })
        .collect::<Vec<_>>();
    let detected = tokio::task::spawn_blocking(move || {
        candidates
            .into_iter()
            .filter_map(|slice| {
                crate::slice::inspect_local_docker_slice_provider_auth(&slice, "all", None)
                    .ok()
                    .map(|provider_auth| (slice, provider_auth))
            })
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "slice.auth.inspect",
        message: format!("slice provider auth inspection task failed: {error}"),
    })?;
    for (slice, provider_auth) in detected {
        let merged = merge_detected_provider_auth(slice.provider_auth.clone(), provider_auth);
        if merged != slice.provider_auth {
            runtime_state.set_slice_provider_auth(&slice.id, merged)?;
        }
    }
    let slices = runtime_state.list_slices();
    Ok(LocalDaemonResponse::SlicesListed { slices })
}

pub(super) async fn execute_create_slice_request(
    runtime_state: &KernelRuntimeState,
    request: CreateSliceRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let slice = runtime_state.create_slice(request).await?;
    Ok(LocalDaemonResponse::SliceCreated { slice })
}

pub(super) async fn execute_get_slice_request(
    runtime_state: &KernelRuntimeState,
    request: SliceRefRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let slice = runtime_state.resolve_slice(&request.slice_ref)?;
    Ok(LocalDaemonResponse::Slice { slice })
}

pub(super) async fn execute_get_slice_logs_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    request: GetSliceLogsRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let slice = runtime_state.resolve_slice(&request.slice_ref)?;
    let docker_options =
        crate::slice::LocalDockerSliceOptions::from_config(&config_projection.snapshot());
    let task_slice = slice.clone();
    let tail_lines = request.tail_lines;
    let entries = tokio::task::spawn_blocking(move || {
        crate::slice::collect_local_docker_slice_logs(&task_slice, &docker_options, tail_lines)
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "slice.logs",
        message: format!("slice log collection task failed: {error}"),
    })??;
    Ok(LocalDaemonResponse::SliceLogs { slice, entries })
}

pub(super) async fn execute_list_slice_audit_request(
    runtime_state: &KernelRuntimeState,
    request: ListSliceAuditRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let events = runtime_state.list_slice_audit_events(&request.slice_ref, request.limit)?;
    Ok(LocalDaemonResponse::SliceAuditListed { events })
}

pub(super) async fn execute_save_slice_state_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    relay_state: Option<Arc<RwLock<RelayClientState>>>,
    request: SliceStateSaveRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let operation_guard =
        runtime_state.begin_slice_operation(&request.slice_ref, "slice.state.save")?;
    let slice = runtime_state.resolve_slice(&request.slice_ref)?;
    runtime_state.record_slice_audit_event(&slice, "state.save", "accepted", None, None)?;
    let slice = runtime_state
        .reconcile_slice_agent_attachments(&slice)
        .await?;
    let mode = match (request.mode, slice.agent_ids.is_empty()) {
        (Some(mode), _) => mode,
        (None, true) => SliceStateSaveMode::Shutdown,
        (None, false) => {
            let error = DaemonError::LocalTransport {
                operation: "slice.state.save",
                message: "slice save-state requires --restart-agents or --shutdown when agents are attached".to_string(),
            };
            let _ = runtime_state.mark_slice_state_save_failed(&request.slice_ref, &error);
            let _ = runtime_state.record_slice_audit_event(
                &slice,
                "state.save",
                "failed",
                None,
                Some(&error.to_string()),
            );
            return Err(error);
        }
    };
    let relaunch_manifests =
        match runtime_state.slice_agent_relaunch_manifests(&slice, "slice.state.save") {
            Ok(manifests) => manifests,
            Err(error) => {
                let _ = runtime_state.mark_slice_state_save_failed(&request.slice_ref, &error);
                let _ = runtime_state.record_slice_audit_event(
                    &slice,
                    "state.save",
                    "failed",
                    None,
                    Some(&error.to_string()),
                );
                return Err(error);
            }
        };
    if let Err(error) = runtime_state
        .park_slice_agent_provider_runs(&relaunch_manifests)
        .await
    {
        let _ = runtime_state.mark_slice_state_save_failed(&request.slice_ref, &error);
        let _ = runtime_state.record_slice_audit_event(
            &slice,
            "state.save",
            "failed",
            None,
            Some(&error.to_string()),
        );
        return Err(error);
    }
    let stopping_slice = runtime_state.mark_slice_stopping(&request.slice_ref)?;
    let docker_options =
        crate::slice::LocalDockerSliceOptions::from_config(&config_projection.snapshot());
    let save_docker_options = docker_options.clone();
    let task_slice = stopping_slice.clone();
    let save_result = tokio::task::spawn_blocking(move || {
        crate::slice::save_local_docker_slice_state(&task_slice, &save_docker_options)
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "slice.state.save",
        message: format!("slice state save task failed: {error}"),
    })?;
    let state = match save_result {
        Ok(state) => state,
        Err(error) => {
            let _ = runtime_state.mark_slice_state_save_failed(&request.slice_ref, &error);
            let _ = runtime_state.record_slice_audit_event(
                &slice,
                "state.save",
                "failed",
                None,
                Some(&error.to_string()),
            );
            return Err(error);
        }
    };
    let stopped_slice = runtime_state.mark_slice_stopped(&request.slice_ref)?;
    runtime_state
        .stop_slice_private_relay_home_connection(&stopped_slice.id)
        .await;
    let saved_slice = runtime_state.save_slice_state_record(&request.slice_ref, state.clone())?;
    if request.scope == Some(SliceStateSaveScope::FutureSlices) {
        crate::slice::set_local_docker_default_saved_state(&state, &docker_options)?;
    }
    runtime_state.record_slice_audit_event(&saved_slice, "state.save", "completed", None, None)?;
    drop(operation_guard);

    if mode == SliceStateSaveMode::RestartAgents {
        let started = execute_start_slice_request(
            runtime_state,
            config_projection,
            relay_state,
            SliceRefRequest {
                slice_ref: saved_slice.id.clone(),
            },
        )
        .await?;
        let LocalDaemonResponse::SliceStarted {
            slice: started_slice,
        } = started
        else {
            return Err(DaemonError::LocalTransport {
                operation: "slice.state.save",
                message: "slice restart returned an unexpected response".to_string(),
            });
        };
        Ok(LocalDaemonResponse::SliceStateSaved {
            slice: started_slice,
            state,
        })
    } else {
        Ok(LocalDaemonResponse::SliceStateSaved {
            slice: saved_slice,
            state,
        })
    }
}

pub(super) async fn execute_get_slice_state_status_request(
    runtime_state: &KernelRuntimeState,
    request: SliceStateStatusRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let slice = runtime_state.resolve_slice(&request.slice_ref)?;
    let state = runtime_state.active_saved_state_for_slice(&request.slice_ref)?;
    Ok(LocalDaemonResponse::SliceStateStatus { slice, state })
}

pub(super) async fn execute_reset_slice_state_request(
    runtime_state: &KernelRuntimeState,
    _config_projection: &DaemonConfigProjectionStore,
    request: SliceStateResetRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let _operation =
        runtime_state.begin_slice_operation(&request.slice_ref, "slice.state.reset")?;
    let slice = runtime_state.resolve_slice(&request.slice_ref)?;
    runtime_state.record_slice_audit_event(&slice, "state.reset", "accepted", None, None)?;
    if let Err(error) = ensure_slice_has_no_active_agents(&slice, "slice.state.reset") {
        let _ = runtime_state.record_slice_audit_event(
            &slice,
            "state.reset",
            "failed",
            None,
            Some(&error.to_string()),
        );
        return Err(error);
    }
    let (slice, removed_state) = runtime_state.reset_slice_state_record(&request.slice_ref)?;
    if let Some(state) = &removed_state {
        let state = state.clone();
        tokio::task::spawn_blocking(move || crate::slice::remove_local_docker_saved_state(&state))
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "slice.state.reset",
                message: format!("slice state reset cleanup task failed: {error}"),
            })??;
    }
    runtime_state.record_slice_audit_event(&slice, "state.reset", "completed", None, None)?;
    Ok(LocalDaemonResponse::SliceStateReset {
        slice,
        removed_state,
    })
}

pub(super) async fn execute_create_slice_backup_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    request: CreateSliceBackupRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let _operation =
        runtime_state.begin_slice_operation(&request.slice_ref, "slice.backup.create")?;
    let slice = runtime_state.resolve_slice(&request.slice_ref)?;
    runtime_state.record_slice_audit_event(&slice, "backup.create", "accepted", None, None)?;
    if let Err(error) = ensure_slice_has_no_active_agents(&slice, "slice.backup.create") {
        let _ = runtime_state.record_slice_audit_event(
            &slice,
            "backup.create",
            "failed",
            None,
            Some(&error.to_string()),
        );
        return Err(error);
    }
    let docker_options =
        crate::slice::LocalDockerSliceOptions::from_config(&config_projection.snapshot());
    let task_slice = slice.clone();
    let backup_name = request.name.clone();
    let backup = tokio::task::spawn_blocking(move || {
        crate::slice::create_local_docker_slice_backup(
            &task_slice,
            &docker_options,
            backup_name.as_deref(),
        )
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "slice.backup.create",
        message: format!("slice backup task failed: {error}"),
    })??;
    let backup = runtime_state.save_slice_backup_record(backup)?;
    let instructions = slice_backup_instructions(&backup);
    runtime_state.record_slice_audit_event(&slice, "backup.create", "completed", None, None)?;
    Ok(LocalDaemonResponse::SliceBackupCreated {
        slice,
        backup,
        instructions,
    })
}

pub(super) async fn execute_start_slice_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    relay_state: Option<Arc<RwLock<RelayClientState>>>,
    request: SliceRefRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let _operation = runtime_state.begin_slice_operation(&request.slice_ref, "slice.start")?;
    let initial_record = runtime_state.resolve_slice(&request.slice_ref)?;
    let initial_record = runtime_state
        .reconcile_slice_agent_attachments(&initial_record)
        .await?;
    let materialization_state = runtime_state.clone();
    let materialization_slice = initial_record.clone();
    let initial_record = tokio::task::spawn_blocking(move || {
        materialization_state.materialize_slice_development_context(&materialization_slice)
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "slice.start",
        message: format!("slice development materialization task failed: {error}"),
    })??;
    let relaunch_manifests =
        runtime_state.slice_agent_relaunch_manifests(&initial_record, "slice.start")?;
    runtime_state
        .park_slice_agent_provider_runs(&relaunch_manifests)
        .await?;
    runtime_state.record_slice_audit_event(&initial_record, "start", "accepted", None, None)?;
    ensure_cloud_relay_connection(runtime_state, config_projection).await?;
    let relay = local_docker_slice_relay(config_projection, &initial_record).await?;
    let initial_slice = runtime_state.mark_slice_starting(
        &request.slice_ref,
        crate::slice::SliceRelayEndpoint {
            url: relay.relay_url.clone(),
            private: relay.uses_private_relay(),
        },
    )?;
    let supervisor_slice = initial_slice.clone();
    let supervisor_relay = Some(relay.clone());
    let mut docker_options =
        crate::slice::LocalDockerSliceOptions::from_config(&config_projection.snapshot());
    if let Some(state) = runtime_state.active_saved_state_for_slice(&initial_slice.id)? {
        docker_options = docker_options.with_saved_state(&state);
    }
    let discovery_config = match provision_and_prepare_worker_discovery(
        runtime_state,
        config_projection,
        &relay,
        Box::new(move || {
            crate::slice::run_local_docker_slice_action(
                &supervisor_slice,
                crate::slice::LocalDockerSliceAction::Provision,
                supervisor_relay,
                None,
                None,
                &docker_options,
            )
        }),
    )
    .await
    {
        Ok(config) => config,
        Err(error) => {
            let _ = runtime_state.mark_slice_operation_failed(&request.slice_ref, "start", &error);
            let _ = runtime_state.record_slice_audit_event(
                &initial_record,
                "start",
                "failed",
                None,
                Some(&error.to_string()),
            );
            return Err(error);
        }
    };
    if relay.uses_private_relay() {
        if let Err(error) = runtime_state
            .ensure_slice_private_relay_home_connection(
                &initial_slice.id,
                relay.relay_url.clone(),
                relay.relay_token.clone(),
            )
            .await
        {
            let _ = runtime_state.mark_slice_operation_failed(&request.slice_ref, "start", &error);
            let _ = runtime_state.record_slice_audit_event(
                &initial_record,
                "start",
                "failed",
                None,
                Some(&error.to_string()),
            );
            return Err(error);
        }
    }
    let discovered = match discover_started_slice_worker(&discovery_config, &initial_slice).await {
        Ok(worker) => Some(worker),
        Err(error) => {
            runtime_state
                .stop_slice_private_relay_home_connection(&initial_slice.id)
                .await;
            let _ = runtime_state.mark_slice_operation_failed(&request.slice_ref, "start", &error);
            let _ = runtime_state.record_slice_audit_event(
                &initial_record,
                "start",
                "failed",
                None,
                Some(&error.to_string()),
            );
            return Err(error);
        }
    };
    if relay.uses_shared_relay() {
        let activation_result = match relay_state.as_ref() {
            Some(relay_state) => {
                activate_hosted_slice_relay_token(
                    config_projection,
                    relay_state,
                    &initial_slice,
                    discovered.as_ref().expect("slice worker was discovered"),
                )
                .await
            }
            None => Err(DaemonError::LocalTransport {
                operation: "activate hosted slice relay token",
                message: "home relay connection state is unavailable".to_string(),
            }),
        };
        if let Err(error) = activation_result {
            let _ = runtime_state.mark_slice_operation_failed(&request.slice_ref, "start", &error);
            let _ = runtime_state.record_slice_audit_event(
                &initial_record,
                "start",
                "failed",
                None,
                Some(&error.to_string()),
            );
            return Err(error);
        }
    }
    let slice = runtime_state.mark_slice_running(&request.slice_ref, discovered)?;
    let mut slice = slice;
    if !relaunch_manifests.is_empty() {
        let worker = relay_presence_from_started_slice(&slice, "slice.start")?;
        runtime_state
            .rebind_and_relaunch_slice_agents(relaunch_manifests, &worker)
            .await?;
        slice = runtime_state.resolve_slice(&request.slice_ref)?;
    }
    runtime_state.record_slice_audit_event(&slice, "start", "completed", None, None)?;
    Ok(LocalDaemonResponse::SliceStarted { slice })
}

pub(super) async fn execute_stop_slice_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    relay_state: Option<Arc<RwLock<RelayClientState>>>,
    request: SliceRefRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let _operation = runtime_state.begin_slice_operation(&request.slice_ref, "slice.stop")?;
    let resolved_slice = runtime_state.resolve_slice(&request.slice_ref)?;
    runtime_state.record_slice_audit_event(&resolved_slice, "stop", "accepted", None, None)?;
    if slice_stop_is_already_complete(&resolved_slice) {
        let slice = runtime_state.mark_slice_stopped(&request.slice_ref)?;
        runtime_state
            .stop_slice_private_relay_home_connection(&slice.id)
            .await;
        revoke_display_tunnels_for_slice(relay_state, &slice.id).await;
        runtime_state.record_slice_audit_event(&slice, "stop", "completed", None, None)?;
        return Ok(LocalDaemonResponse::SliceStopped { slice });
    }
    if let Err(error) = ensure_slice_has_no_active_agents(&resolved_slice, "slice.stop") {
        let _ = runtime_state.mark_slice_operation_rejected(&request.slice_ref, "stop", &error);
        let _ = runtime_state.record_slice_audit_event(
            &resolved_slice,
            "stop",
            "failed",
            None,
            Some(&error.to_string()),
        );
        return Err(error);
    }
    let stopping_slice = runtime_state.mark_slice_stopping(&request.slice_ref)?;
    let docker_options =
        crate::slice::LocalDockerSliceOptions::from_config(&config_projection.snapshot());
    let task_slice = stopping_slice.clone();
    let supervisor_result = tokio::task::spawn_blocking(move || {
        crate::slice::run_local_docker_slice_action(
            &task_slice,
            crate::slice::LocalDockerSliceAction::Stop,
            None,
            None,
            None,
            &docker_options,
        )
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "slice.stop",
        message: format!("slice supervisor task failed: {error}"),
    })?;
    if let Err(error) = supervisor_result {
        let _ = runtime_state.mark_slice_operation_failed(&request.slice_ref, "stop", &error);
        let _ = runtime_state.record_slice_audit_event(
            &stopping_slice,
            "stop",
            "failed",
            None,
            Some(&error.to_string()),
        );
        return Err(error);
    }
    let slice = runtime_state.mark_slice_stopped(&request.slice_ref)?;
    runtime_state
        .stop_slice_private_relay_home_connection(&slice.id)
        .await;
    revoke_display_tunnels_for_slice(relay_state, &slice.id).await;
    runtime_state.record_slice_audit_event(&slice, "stop", "completed", None, None)?;
    Ok(LocalDaemonResponse::SliceStopped { slice })
}

pub(super) async fn execute_delete_slice_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    relay_state: Option<Arc<RwLock<RelayClientState>>>,
    request: SliceRefRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let _operation = runtime_state.begin_slice_operation(&request.slice_ref, "slice.delete")?;
    let resolved_slice = runtime_state.resolve_slice(&request.slice_ref)?;
    runtime_state.record_slice_audit_event(&resolved_slice, "delete", "accepted", None, None)?;
    if let Err(error) = ensure_slice_has_no_active_agents(&resolved_slice, "slice.delete") {
        let _ = runtime_state.mark_slice_delete_failed(&request.slice_ref, &error);
        let _ = runtime_state.record_slice_audit_event(
            &resolved_slice,
            "delete",
            "failed",
            None,
            Some(&error.to_string()),
        );
        return Err(error);
    }
    let deleting_slice = runtime_state.mark_slice_delete_in_progress(&request.slice_ref)?;
    if resolved_slice.backend == crate::slice::SliceBackendKind::LocalDocker {
        let docker_options =
            crate::slice::LocalDockerSliceOptions::from_config(&config_projection.snapshot());
        let task_slice = deleting_slice.clone();
        let supervisor_result = tokio::task::spawn_blocking(move || {
            crate::slice::run_local_docker_slice_action(
                &task_slice,
                crate::slice::LocalDockerSliceAction::Destroy,
                None,
                None,
                None,
                &docker_options,
            )
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "slice.delete",
            message: format!("slice supervisor task failed: {error}"),
        })?;
        if let Err(error) = supervisor_result {
            let _ = runtime_state.mark_slice_delete_failed(&request.slice_ref, &error);
            let _ = runtime_state.record_slice_audit_event(
                &deleting_slice,
                "delete",
                "failed",
                None,
                Some(&error.to_string()),
            );
            return Err(error);
        }
    }
    let cleanup_state = runtime_state.clone();
    let cleanup_slice = deleting_slice.clone();
    let cleanup_result = tokio::task::spawn_blocking(move || {
        cleanup_state.cleanup_slice_development_context(&cleanup_slice)
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "slice.delete",
        message: format!("slice development cleanup task failed: {error}"),
    })?;
    if let Err(error) = cleanup_result {
        let _ = runtime_state.mark_slice_delete_failed(&request.slice_ref, &error);
        let _ = runtime_state.record_slice_audit_event(
            &deleting_slice,
            "delete",
            "failed",
            None,
            Some(&error.to_string()),
        );
        return Err(error);
    }
    let slice = runtime_state.delete_slice(&request.slice_ref)?;
    runtime_state
        .stop_slice_private_relay_home_connection(&slice.id)
        .await;
    revoke_display_tunnels_for_slice(relay_state, &slice.id).await;
    runtime_state.record_slice_audit_event(&slice, "delete", "completed", None, None)?;
    Ok(LocalDaemonResponse::SliceDeleted { slice })
}

fn slice_backup_instructions(backup: &crate::slice::SliceBackupRecord) -> String {
    let backup_dir = std::path::Path::new(&backup.manifest_path)
        .parent()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| backup.manifest_path.clone());
    format!(
        "Backup saved:\n  {backup_dir}\n\nTo use it manually, stop Chariox slice operations, then swap this backup directory with the active state directory for the slice.\n\nThe Docker image tag for this backup is:\n  {}",
        backup.image_ref
    )
}

fn ensure_slice_has_no_active_agents(
    slice: &crate::slice::SliceRecord,
    operation: &'static str,
) -> Result<(), DaemonError> {
    if slice.agent_ids.is_empty() {
        return Ok(());
    }
    Err(DaemonError::LocalTransport {
        operation,
        message: format!(
            "slice `{}` still has {} active agent(s)",
            slice.name,
            slice.agent_ids.len()
        ),
    })
}

fn slice_stop_is_already_complete(slice: &crate::slice::SliceRecord) -> bool {
    slice.status == crate::slice::SliceStatus::Stopped
}

fn relay_presence_from_started_slice(
    slice: &crate::slice::SliceRecord,
    operation: &'static str,
) -> Result<chariox_relay::protocol::RelayKernelPresence, DaemonError> {
    let Some(worker_kernel_id) = slice.worker_kernel_id.clone() else {
        return Err(DaemonError::LocalTransport {
            operation,
            message: format!("started slice `{}` has no worker kernel id", slice.name),
        });
    };
    let Some(worker_machine_id) = slice.worker_machine_id.clone() else {
        return Err(DaemonError::LocalTransport {
            operation,
            message: format!("started slice `{}` has no worker machine id", slice.name),
        });
    };
    Ok(chariox_relay::protocol::RelayKernelPresence {
        kernel_id: worker_kernel_id,
        machine_id: worker_machine_id,
        machine_alias: None,
        relay_alias: None,
        kernel_alias: None,
        available_providers: slice.providers.clone(),
        provider_accounts: Vec::new(),
        capabilities: Vec::new(),
        accepting_remote_leases: true,
        leased_agent_count: 0,
        local_session_count: 0,
        public_key: String::new(),
    })
}

async fn local_docker_slice_relay(
    config_projection: &DaemonConfigProjectionStore,
    slice: &crate::slice::SliceRecord,
) -> Result<crate::slice::LocalDockerSliceRelay, DaemonError> {
    let config = config_projection.snapshot();
    let mut hosted_relay_token = None;
    if let (Some(relay_url), Some(relay_token)) =
        (config.relay_url.clone(), config.relay_token.clone())
    {
        if configured_relay_is_container_reachable(&relay_url) {
            hosted_relay_token =
                Some(hosted_cloud_slice_relay_token(&config, slice, relay_token).await?);
        }
    }
    Ok(local_docker_slice_relay_for_config(
        &config,
        slice,
        hosted_relay_token,
    ))
}

fn local_docker_slice_relay_for_config(
    config: &crate::config::DaemonConfig,
    slice: &crate::slice::SliceRecord,
    hosted_relay_token: Option<String>,
) -> crate::slice::LocalDockerSliceRelay {
    if let (Some(relay_url), Some(relay_token)) =
        (config.relay_url.clone(), config.relay_token.clone())
    {
        if configured_relay_is_container_reachable(&relay_url) {
            let relay_token = hosted_relay_token.unwrap_or(relay_token);
            crate::logging::info_with_fields(
                "daemon.slice",
                "selected hosted relay for local docker slice",
                serde_json::json!({
                    "slice_id": slice.id,
                    "relay_url": relay_url,
                    "cloud_profile_present": config.cloud_relay.is_some(),
                    "cloud_relay_config_present": false,
                }),
            );
            return crate::slice::LocalDockerSliceRelay {
                relay_url: relay_url.clone(),
                container_relay_url: Some(relay_url),
                relay_token,
                // The home kernel's Cloud profile contains machine credentials and
                // session tokens. It must never be copied into a provider-visible slice.
                cloud_relay_config_json: None,
            };
        }
    }
    crate::slice::local_docker_private_relay(slice)
}

async fn hosted_cloud_slice_relay_token(
    config: &crate::config::DaemonConfig,
    slice: &crate::slice::SliceRecord,
    fallback_relay_token: String,
) -> Result<String, DaemonError> {
    let Some(profile) = config.cloud_relay.clone() else {
        return Ok(fallback_relay_token);
    };
    let issued = issue_cloud_slice_runtime_token(
        &profile,
        &slice.worker_kernel_ref,
        &slice.owner_kernel_id,
        None,
    )
    .await?;
    Ok(issued.token)
}

async fn activate_hosted_slice_relay_token(
    config_projection: &DaemonConfigProjectionStore,
    relay_state: &Arc<RwLock<RelayClientState>>,
    slice: &crate::slice::SliceRecord,
    worker: &chariox_relay::protocol::RelayKernelPresence,
) -> Result<(), DaemonError> {
    let config = config_projection.snapshot();
    let Some(profile) = config.cloud_relay.as_ref() else {
        return Ok(());
    };
    let issued = issue_cloud_slice_runtime_token(
        profile,
        &slice.worker_kernel_ref,
        &slice.owner_kernel_id,
        Some(&worker.public_key),
    )
    .await?;
    let expires_at_ms =
        relay_token_expiry_ms(&issued.token).ok_or_else(|| DaemonError::LocalTransport {
            operation: "activate hosted slice relay token",
            message: "Cloud returned a relay token without an expiry".to_string(),
        })?;
    let recovery = issue_cloud_slice_recovery_token(
        profile,
        &slice.worker_kernel_ref,
        &slice.owner_kernel_id,
        &worker.public_key,
    )
    .await?;
    let recovery_expires_at_ms =
        relay_token_expiry_ms(&recovery.token).ok_or_else(|| DaemonError::LocalTransport {
            operation: "activate hosted slice relay token",
            message: "Cloud returned a slice recovery token without an expiry".to_string(),
        })?;
    let activation_nonce = managed_slice_relay_activation_nonce();
    relay_state
        .write()
        .await
        .begin_managed_slice_relay_activation(
            slice.id.clone(),
            worker.kernel_id.clone(),
            slice.worker_kernel_ref.clone(),
            worker.public_key.clone(),
            activation_nonce.clone(),
        );
    let result = async {
        let response = crate::transport::relay_client::send_peer_request_via_temporary_connection(
            &config,
            chariox_relay::protocol::ClientTarget {
                daemon_id: Some(worker.kernel_id.clone()),
                daemon_alias: None,
            },
            crate::transport::relay_peer::RelayPeerRequest::InstallManagedSliceRelayToken {
                slice_id: slice.id.clone(),
                owner_kernel_id: slice.owner_kernel_id.clone(),
                owner_machine_id: slice.owner_machine_id.clone(),
                activation_nonce: activation_nonce.clone(),
                relay_token: crate::transport::relay_peer::RelayManagedSliceToken::new(
                    issued.token,
                ),
                expires_at_ms,
                relay_recovery_token: crate::transport::relay_peer::RelayManagedSliceToken::new(
                    recovery.token,
                ),
                recovery_expires_at_ms,
            },
        )
        .await?;
        match response {
            crate::transport::relay_peer::RelayPeerResponse::ManagedSliceRelayTokenInstalled {
                slice_id,
                activation_nonce: installed_nonce,
                relay_peer_protocol_version,
            } if slice_id == slice.id
                && installed_nonce == activation_nonce
                && relay_peer_protocol_version
                    >= crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION =>
            {
                wait_for_hosted_slice_relay_activation(
                    config_projection,
                    relay_state,
                    slice,
                    worker,
                    &activation_nonce,
                )
                .await
            }
            crate::transport::relay_peer::RelayPeerResponse::ManagedSliceRelayTokenFailed {
                code,
                retryable,
            } => Err(DaemonError::LocalTransport {
                operation: "activate hosted slice relay token",
                message: format!(
                    "slice worker rejected key-bound relay token ({code}, retryable={retryable})"
                ),
            }),
            _ => Err(DaemonError::LocalTransport {
                operation: "activate hosted slice relay token",
                message: "slice worker returned an incompatible relay-token response".to_string(),
            }),
        }
    }
    .await;
    let mut state = relay_state.write().await;
    if result.is_ok() {
        state.retain_completed_managed_slice_relay_activation(&slice.id, &activation_nonce);
    } else {
        state.discard_managed_slice_relay_activation(&slice.id, &activation_nonce);
    }
    result
}

async fn wait_for_hosted_slice_relay_activation(
    config_projection: &DaemonConfigProjectionStore,
    relay_state: &Arc<RwLock<RelayClientState>>,
    slice: &crate::slice::SliceRecord,
    worker: &chariox_relay::protocol::RelayKernelPresence,
    activation_nonce: &str,
) -> Result<(), DaemonError> {
    let deadline = Instant::now() + HOSTED_SLICE_RELAY_RECONNECT_TIMEOUT;
    let mut last_probe_error = None;
    let ping_value = format!(
        "managed-slice-relay-activation:{}:{}",
        slice.id, activation_nonce
    );

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            let detail = last_probe_error
                .map(|error: DaemonError| format!("; last probe failed: {error}"))
                .unwrap_or_default();
            return Err(DaemonError::LocalTransport {
                operation: "activate hosted slice relay token",
                message: format!(
                    "slice worker did not confirm key-bound relay-token activation within 15s{detail}"
                ),
            });
        }

        if !relay_state
            .read()
            .await
            .managed_slice_relay_activation_confirmed(&slice.id, activation_nonce)
        {
            sleep(remaining.min(HOSTED_SLICE_RELAY_RECONNECT_POLL_INTERVAL)).await;
            continue;
        }

        let config = config_projection.snapshot();
        let observation = timeout(
            remaining.min(HOSTED_SLICE_RELAY_RECONNECT_PROBE_TIMEOUT),
            crate::transport::relay_discovery::find_live_kernel_once(&config, &worker.kernel_id),
        )
        .await;
        let presence = match observation {
            Ok(Ok(presence)) => presence,
            Ok(Err(error)) => {
                last_probe_error = Some(error);
                sleep(
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(HOSTED_SLICE_RELAY_RECONNECT_POLL_INTERVAL),
                )
                .await;
                continue;
            }
            Err(_) => {
                last_probe_error = Some(DaemonError::LocalTransport {
                    operation: "activate hosted slice relay token",
                    message: "relay presence probe timed out".to_string(),
                });
                continue;
            }
        };

        if hosted_slice_activation_presence_ready(presence.as_ref(), &worker.public_key)? {
            let ping_timeout = deadline
                .saturating_duration_since(Instant::now())
                .min(HOSTED_SLICE_RELAY_RECONNECT_PROBE_TIMEOUT);
            if ping_timeout.is_zero() {
                continue;
            }
            let ping = crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
                &config,
                chariox_relay::protocol::ClientTarget {
                    daemon_id: Some(worker.kernel_id.clone()),
                    daemon_alias: None,
                },
                crate::transport::relay_peer::RelayPeerRequest::Ping {
                    value: ping_value.clone(),
                },
                ping_timeout,
            );
            match timeout(ping_timeout, ping).await {
                Ok(Ok(crate::transport::relay_peer::RelayPeerResponse::Pong {
                    value,
                    daemon_id,
                })) if value == ping_value && daemon_id == worker.kernel_id => return Ok(()),
                Ok(Ok(_)) => {
                    last_probe_error = Some(DaemonError::LocalTransport {
                        operation: "activate hosted slice relay token",
                        message: "reconnected slice returned an incompatible ping response"
                            .to_string(),
                    });
                }
                Ok(Err(error)) => last_probe_error = Some(error),
                Err(_) => {
                    last_probe_error = Some(DaemonError::LocalTransport {
                        operation: "activate hosted slice relay token",
                        message: "reconnected slice ping timed out".to_string(),
                    });
                }
            }
        }

        sleep(
            deadline
                .saturating_duration_since(Instant::now())
                .min(HOSTED_SLICE_RELAY_RECONNECT_POLL_INTERVAL),
        )
        .await;
    }
}

fn hosted_slice_activation_presence_ready(
    presence: Option<&chariox_relay::protocol::RelayKernelPresence>,
    expected_public_key: &str,
) -> Result<bool, DaemonError> {
    let Some(presence) = presence else {
        return Ok(false);
    };
    if presence.public_key != expected_public_key {
        return Err(DaemonError::LocalTransport {
            operation: "activate hosted slice relay token",
            message: "slice worker relay key changed during credential activation".to_string(),
        });
    }
    Ok(true)
}

fn managed_slice_relay_activation_nonce() -> String {
    format!(
        "{:032x}{:032x}",
        rand::random::<u128>(),
        rand::random::<u128>()
    )
}

fn configured_relay_is_container_reachable(relay_url: &str) -> bool {
    let Ok(url) = url::Url::parse(relay_url) else {
        return false;
    };
    match url.scheme() {
        "ws" | "wss" => {}
        _ => return false,
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    !matches!(host, "127.0.0.1" | "::1" | "localhost")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slice(agent_ids: Vec<String>) -> crate::slice::SliceRecord {
        crate::slice::SliceRecord {
            id: "slice-1".to_string(),
            name: "dev".to_string(),
            owner_kernel_id: "kernel-1".to_string(),
            owner_machine_id: "machine-1".to_string(),
            session_id: None,
            session_ids: Vec::new(),
            agent_ids,
            backend: crate::slice::SliceBackendKind::LocalDocker,
            os: "linux".to_string(),
            display_mode: crate::slice::SliceDisplayMode::Headless,
            status: crate::slice::SliceStatus::Running,
            last_operation: None,
            last_operation_status: None,
            last_error: None,
            last_operation_at_ms: None,
            workspace_id: Some("workspace".to_string()),
            worktree_id: Some("worktree".to_string()),
            workspace_mount: Some("worktree".to_string()),
            development: None,
            development_storage_root: None,
            development_publication: None,
            worker_kernel_ref: "slice:dev".to_string(),
            worker_kernel_id: Some("worker-1".to_string()),
            worker_machine_id: Some("machine-slice-1".to_string()),
            relay_endpoint: None,
            local_docker_ports: None,
            providers: Vec::new(),
            provider_auth: Vec::new(),
            saved_state_ref: None,
            saved_state_status: None,
            saved_state_updated_at_ms: None,
            display_endpoint: None,
            created_at_ms: 0,
            updated_at_ms: 0,
        }
    }

    fn cloud_profile() -> crate::config::PersistedCloudRelayProfile {
        crate::config::PersistedCloudRelayProfile {
            api_url: "https://cloud.example.test".to_string(),
            email: "user@example.test".to_string(),
            account_id: "account-1".to_string(),
            user_id: "user-1".to_string(),
            account_slug: "acct".to_string(),
            realm_id: "realm-1".to_string(),
            relay_url: "wss://relay.example.test".to_string(),
            issuer_id: "issuer-1".to_string(),
            client_id: Some("client-1".to_string()),
            client_alias: None,
            machine_id: Some("machine-1".to_string()),
            machine_alias: None,
            machine_credential: Some("machine-secret".to_string()),
            cloud_session_token: None,
            cloud_session_expires_at_ms: None,
            token_expires_at_ms: Some(200_000),
        }
    }

    #[test]
    fn stop_and_delete_guard_rejects_active_agents() {
        let error =
            ensure_slice_has_no_active_agents(&slice(vec!["agent-1".to_string()]), "slice.stop")
                .expect_err("active slice should reject stop/delete");
        assert!(error.to_string().contains("active agent"));
        ensure_slice_has_no_active_agents(&slice(Vec::new()), "slice.stop")
            .expect("idle slice should pass guard");
    }

    #[test]
    fn stop_is_idempotent_after_shutdown_save_with_attached_agents() {
        let mut stopped = slice(vec!["agent-1".to_string()]);
        stopped.status = crate::slice::SliceStatus::Stopped;
        assert!(slice_stop_is_already_complete(&stopped));
    }

    #[test]
    fn hosted_slice_activation_requires_live_same_key_after_confirmation() {
        let worker = chariox_relay::protocol::RelayKernelPresence {
            kernel_id: "worker-1".to_string(),
            machine_id: "machine-slice-1".to_string(),
            machine_alias: None,
            relay_alias: None,
            kernel_alias: Some("slice:dev".to_string()),
            available_providers: Vec::new(),
            provider_accounts: Vec::new(),
            capabilities: Vec::new(),
            accepting_remote_leases: false,
            leased_agent_count: 0,
            local_session_count: 0,
            public_key: "worker-key".to_string(),
        };
        assert!(!hosted_slice_activation_presence_ready(None, "worker-key")
            .expect("offline worker should remain pending"));
        assert!(
            hosted_slice_activation_presence_ready(Some(&worker), "worker-key")
                .expect("same-key worker should be ready after authenticated confirmation")
        );

        let mut replacement = worker;
        replacement.public_key = "replacement-key".to_string();
        assert!(hosted_slice_activation_presence_ready(Some(&replacement), "worker-key").is_err());
    }

    #[test]
    fn managed_slice_activation_nonce_is_fresh() {
        let first = managed_slice_relay_activation_nonce();
        let second = managed_slice_relay_activation_nonce();
        assert_eq!(first.len(), 64);
        assert_ne!(first, second);
    }

    #[test]
    fn hosted_cloud_slices_use_shared_relay_for_worker_projection() {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.relay_url = Some("wss://relay.example.test".to_string());
        config.relay_token = Some("shared-token".to_string());
        let projection = DaemonConfigProjectionStore::new(config);

        let config = projection.snapshot();
        let relay = local_docker_slice_relay_for_config(&config, &slice(Vec::new()), None);

        assert_eq!(relay.relay_url, "wss://relay.example.test");
        assert_eq!(
            relay.container_relay_url.as_deref(),
            Some("wss://relay.example.test")
        );
        assert_eq!(relay.relay_token, "shared-token");
        assert_eq!(relay.cloud_relay_config_json, None);
    }

    #[test]
    fn hosted_cloud_slices_never_pass_the_home_cloud_profile_to_worker() {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.relay_url = Some("wss://relay.example.test".to_string());
        config.relay_token = Some("shared-token".to_string());
        config.cloud_relay = Some(cloud_profile());
        let projection = DaemonConfigProjectionStore::new(config);

        let config = projection.snapshot();
        let relay = local_docker_slice_relay_for_config(
            &config,
            &slice(Vec::new()),
            Some("fresh-worker-token".to_string()),
        );

        assert_eq!(relay.cloud_relay_config_json, None);
        assert_eq!(relay.relay_token, "fresh-worker-token");
    }

    #[test]
    fn self_hosted_slices_use_configured_non_loopback_ws_relay() {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.relay_url = Some("ws://relay.lan:49100".to_string());
        config.relay_token = Some("self-host-token".to_string());
        let projection = DaemonConfigProjectionStore::new(config);

        let config = projection.snapshot();
        let relay = local_docker_slice_relay_for_config(&config, &slice(Vec::new()), None);

        assert_eq!(relay.relay_url, "ws://relay.lan:49100");
        assert_eq!(
            relay.container_relay_url.as_deref(),
            Some("ws://relay.lan:49100")
        );
        assert_eq!(relay.relay_token, "self-host-token");
        assert_eq!(relay.cloud_relay_config_json, None);
    }

    #[test]
    fn local_slices_keep_private_relay_for_loopback_relay_setups() {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.relay_url = Some("ws://127.0.0.1:49100".to_string());
        config.relay_token = Some("local-token".to_string());
        let projection = DaemonConfigProjectionStore::new(config);

        let config = projection.snapshot();
        let relay = local_docker_slice_relay_for_config(&config, &slice(Vec::new()), None);

        assert!(relay.relay_url.starts_with("ws://127.0.0.1:"));
        assert_eq!(relay.container_relay_url, None);
        assert_eq!(relay.relay_token, "slice-local-kernel-1-slice-1");
        assert_eq!(relay.cloud_relay_config_json, None);
    }

    #[test]
    fn local_slices_keep_private_relay_when_configured_relay_lacks_token() {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.relay_url = Some("wss://relay.example.test".to_string());
        config.relay_token = None;
        let projection = DaemonConfigProjectionStore::new(config);

        let config = projection.snapshot();
        let relay = local_docker_slice_relay_for_config(&config, &slice(Vec::new()), None);

        assert!(relay.relay_url.starts_with("ws://127.0.0.1:"));
        assert_eq!(relay.container_relay_url, None);
        assert_eq!(relay.relay_token, "slice-local-kernel-1-slice-1");
        assert_eq!(relay.cloud_relay_config_json, None);
    }
}
