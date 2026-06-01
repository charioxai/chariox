use tokio::time::{sleep, Duration};

use crate::error::DaemonError;
use crate::local::{
    CreateSliceRequest, GetSliceLogsRequest, ListSlicesRequest, LocalDaemonResponse,
    SliceRefRequest,
};
use crate::runtime::projection::DaemonConfigProjectionStore;
use crate::runtime::state::KernelRuntimeState;

pub(super) async fn execute_list_slices_request(
    runtime_state: &KernelRuntimeState,
    _request: ListSlicesRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
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

pub(super) async fn execute_start_slice_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    request: SliceRefRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let _operation = runtime_state.begin_slice_operation(&request.slice_ref, "slice.start")?;
    let initial_record = runtime_state.resolve_slice(&request.slice_ref)?;
    runtime_state.record_slice_audit_event(&initial_record, "start", "accepted", None, None)?;
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
            None,
            &docker_options,
        )
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "slice.start",
        message: format!("slice supervisor task failed: {error}"),
    })?;
    if let Err(error) = supervisor_result {
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
    let discovered = match discover_started_slice_worker(config_projection, &initial_slice, &relay)
        .await
    {
        Ok(worker) => Some(worker),
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
    let slice = runtime_state.mark_slice_running(&request.slice_ref, discovered)?;
    runtime_state.record_slice_audit_event(&slice, "start", "completed", None, None)?;
    Ok(LocalDaemonResponse::SliceStarted { slice })
}

pub(super) async fn execute_stop_slice_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    request: SliceRefRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let _operation = runtime_state.begin_slice_operation(&request.slice_ref, "slice.stop")?;
    let resolved_slice = runtime_state.resolve_slice(&request.slice_ref)?;
    runtime_state.record_slice_audit_event(&resolved_slice, "stop", "accepted", None, None)?;
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
    runtime_state.record_slice_audit_event(&slice, "stop", "completed", None, None)?;
    Ok(LocalDaemonResponse::SliceStopped { slice })
}

pub(super) async fn execute_delete_slice_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
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
    let slice = runtime_state.delete_slice(&request.slice_ref)?;
    runtime_state.record_slice_audit_event(&slice, "delete", "completed", None, None)?;
    Ok(LocalDaemonResponse::SliceDeleted { slice })
}

pub(super) async fn execute_get_slice_display_endpoint_request(
    runtime_state: &KernelRuntimeState,
    request: SliceRefRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let endpoint = runtime_state.slice_display_endpoint(&request.slice_ref)?;
    Ok(LocalDaemonResponse::SliceDisplayEndpoint { endpoint })
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
    if let (Some(relay_url), Some(relay_token)) =
        (config.relay_url.clone(), config.relay_token.clone())
    {
        if configured_relay_is_container_reachable(&relay_url) {
            let cloud_relay_config_json =
                hosted_cloud_relay_config_json(&config, &relay_url, &relay_token);
            return crate::slice::LocalDockerSliceRelay {
                relay_url: relay_url.clone(),
                container_relay_url: Some(relay_url),
                relay_token,
                cloud_relay_config_json,
            };
        }
    }
    crate::slice::local_docker_private_relay(slice)
}

fn hosted_cloud_relay_config_json(
    config: &crate::config::DaemonConfig,
    relay_url: &str,
    relay_token: &str,
) -> Option<String> {
    if !relay_url.starts_with("wss://") {
        return None;
    }
    let profile = config.cloud_relay.as_ref()?;
    serde_json::to_string(&serde_json::json!({
        "relay_url": relay_url,
        "relay_token": relay_token,
        "cloud_relay": profile,
    }))
    .ok()
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
            worker_kernel_ref: "slice:dev".to_string(),
            worker_kernel_id: Some("worker-1".to_string()),
            worker_machine_id: Some("machine-slice-1".to_string()),
            relay_endpoint: None,
            local_docker_ports: None,
            providers: Vec::new(),
            provider_auth: Vec::new(),
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
    fn hosted_cloud_slices_use_shared_relay_for_worker_projection() {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.relay_url = Some("wss://relay.example.test".to_string());
        config.relay_token = Some("shared-token".to_string());
        let projection = DaemonConfigProjectionStore::new(config);

        let relay = local_docker_slice_relay(&projection, &slice(Vec::new()));

        assert_eq!(relay.relay_url, "wss://relay.example.test");
        assert_eq!(
            relay.container_relay_url.as_deref(),
            Some("wss://relay.example.test")
        );
        assert_eq!(relay.relay_token, "shared-token");
        assert_eq!(relay.cloud_relay_config_json, None);
    }

    #[test]
    fn hosted_cloud_slices_pass_refreshable_relay_profile_to_worker() {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.relay_url = Some("wss://relay.example.test".to_string());
        config.relay_token = Some("shared-token".to_string());
        config.cloud_relay = Some(cloud_profile());
        let projection = DaemonConfigProjectionStore::new(config);

        let relay = local_docker_slice_relay(&projection, &slice(Vec::new()));

        let profile_json = relay
            .cloud_relay_config_json
            .expect("hosted cloud relay should pass refreshable config");
        let payload: serde_json::Value = serde_json::from_str(&profile_json).unwrap();
        assert_eq!(payload["relay_url"], "wss://relay.example.test");
        assert_eq!(payload["relay_token"], "shared-token");
        assert_eq!(
            payload["cloud_relay"]["machine_credential"],
            "machine-secret"
        );
    }

    #[test]
    fn self_hosted_slices_use_configured_non_loopback_ws_relay() {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.relay_url = Some("ws://relay.lan:49100".to_string());
        config.relay_token = Some("self-host-token".to_string());
        let projection = DaemonConfigProjectionStore::new(config);

        let relay = local_docker_slice_relay(&projection, &slice(Vec::new()));

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

        let relay = local_docker_slice_relay(&projection, &slice(Vec::new()));

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

        let relay = local_docker_slice_relay(&projection, &slice(Vec::new()));

        assert!(relay.relay_url.starts_with("ws://127.0.0.1:"));
        assert_eq!(relay.container_relay_url, None);
        assert_eq!(relay.relay_token, "slice-local-kernel-1-slice-1");
        assert_eq!(relay.cloud_relay_config_json, None);
    }
}
