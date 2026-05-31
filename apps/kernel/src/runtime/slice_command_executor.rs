mod provider_auth;

use tokio::time::{sleep, Duration};

use crate::error::DaemonError;
use crate::local::{
    CreateSliceRequest, GetSliceLogsRequest, ImportSliceProviderAuthRequest, ListSlicesRequest,
    LocalDaemonRequest, LocalDaemonResponse, RemoveSliceProviderAuthRequest,
    SetSliceProviderAuthAliasRequest, SliceRefRequest, StartSliceProviderLoginRequest,
};
use crate::runtime::projection::DaemonConfigProjectionStore;
use crate::runtime::state::KernelRuntimeState;

use provider_auth::{
    merge_scoped_provider_auth, normalized_slice_provider, scoped_provider_auth_summaries,
    slice_auth_summary_matches_provider,
};

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
        LocalDaemonRequest::RemoveSliceProviderAuth(request) => {
            execute_remove_slice_provider_auth_request(runtime_state, config_projection, request)
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
        LocalDaemonRequest::GetSliceLogs(request) => {
            execute_get_slice_logs_request(runtime_state, config_projection, request).await
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

pub(crate) async fn execute_get_slice_logs_request(
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

pub(crate) async fn execute_start_slice_request(
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
        let _ = runtime_state
            .set_slice_status(&request.slice_ref, crate::slice::SliceStatus::Unhealthy);
        let _ = runtime_state.record_slice_audit_event(
            &initial_record,
            "start",
            "failed",
            None,
            Some(&error.to_string()),
        );
        return Err(error);
    }
    let discovered =
        match discover_started_slice_worker(config_projection, &initial_slice, &relay).await {
            Ok(worker) => Some(worker),
            Err(error) => {
                let _ = runtime_state
                    .set_slice_status(&request.slice_ref, crate::slice::SliceStatus::Unhealthy);
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

pub(crate) async fn execute_stop_slice_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    request: SliceRefRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let _operation = runtime_state.begin_slice_operation(&request.slice_ref, "slice.stop")?;
    let resolved_slice = runtime_state.resolve_slice(&request.slice_ref)?;
    runtime_state.record_slice_audit_event(&resolved_slice, "stop", "accepted", None, None)?;
    ensure_slice_has_no_active_agents(&resolved_slice, "slice.stop")?;
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
        let _ = runtime_state
            .set_slice_status(&request.slice_ref, crate::slice::SliceStatus::Unhealthy);
        let _ = runtime_state.record_slice_audit_event(
            &stopping_slice,
            "stop",
            "failed",
            None,
            Some(&error.to_string()),
        );
        return Err(error);
    }
    let slice =
        runtime_state.set_slice_status(&request.slice_ref, crate::slice::SliceStatus::Stopped)?;
    runtime_state.record_slice_audit_event(&slice, "stop", "completed", None, None)?;
    Ok(LocalDaemonResponse::SliceStopped { slice })
}

pub(crate) async fn execute_delete_slice_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    request: SliceRefRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let _operation = runtime_state.begin_slice_operation(&request.slice_ref, "slice.delete")?;
    let resolved_slice = runtime_state.resolve_slice(&request.slice_ref)?;
    runtime_state.record_slice_audit_event(&resolved_slice, "delete", "accepted", None, None)?;
    ensure_slice_has_no_active_agents(&resolved_slice, "slice.delete")?;
    if resolved_slice.backend == crate::slice::SliceBackendKind::LocalDocker {
        let docker_options =
            crate::slice::LocalDockerSliceOptions::from_config(&config_projection.snapshot());
        let task_slice = resolved_slice.clone();
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
            let _ = runtime_state.record_slice_audit_event(
                &resolved_slice,
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

pub(crate) async fn execute_import_slice_provider_auth_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    request: ImportSliceProviderAuthRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let _operation =
        runtime_state.begin_slice_operation(&request.slice_ref, "slice.auth.import")?;
    let slice = runtime_state.resolve_slice(&request.slice_ref)?;
    let provider = normalized_slice_provider(&request.provider)?;
    runtime_state.record_slice_audit_event(
        &slice,
        "auth.import",
        "accepted",
        Some(&provider),
        None,
    )?;
    if slice.backend == crate::slice::SliceBackendKind::LocalDocker {
        let docker_options =
            crate::slice::LocalDockerSliceOptions::from_config(&config_projection.snapshot());
        let resolved_slice = slice.clone();
        let provider_for_action = provider.clone();
        let import_result = tokio::task::spawn_blocking(move || {
            crate::slice::run_local_docker_slice_action(
                &resolved_slice,
                crate::slice::LocalDockerSliceAction::ImportProviderAuth,
                None,
                Some(&provider_for_action),
                &docker_options,
            )
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "slice.auth.import",
            message: format!("slice auth import task failed: {error}"),
        })?;
        if let Err(error) = import_result {
            let _ = runtime_state.record_slice_audit_event(
                &slice,
                "auth.import",
                "failed",
                Some(&provider),
                Some(&error.to_string()),
            );
            return Err(error);
        }
        let imported_provider_auth = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .map(|home| crate::slice_provider_auth::inspect_home_provider_auth(&home))
            .map(|summaries| scoped_provider_auth_summaries(&provider, summaries))
            .unwrap_or_default();
        let provider_auth =
            merge_scoped_provider_auth(slice.provider_auth, &provider, imported_provider_auth);
        let slice = runtime_state.set_slice_provider_auth(&request.slice_ref, provider_auth)?;
        runtime_state.record_slice_audit_event(
            &slice,
            "auth.import",
            "completed",
            Some(&provider),
            None,
        )?;
        return Ok(LocalDaemonResponse::SliceProviderAuthImported {
            slice,
            provider,
            status: "imported".to_string(),
        });
    }
    runtime_state.record_slice_audit_event(
        &slice,
        "auth.import",
        "not_implemented",
        Some(&provider),
        Some("slice auth import is not implemented for this backend"),
    )?;
    Ok(LocalDaemonResponse::SliceProviderAuthImported {
        slice,
        provider,
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

pub(crate) async fn execute_remove_slice_provider_auth_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    request: RemoveSliceProviderAuthRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let _operation =
        runtime_state.begin_slice_operation(&request.slice_ref, "slice.auth.remove")?;
    let slice = runtime_state.resolve_slice(&request.slice_ref)?;
    let provider = normalized_slice_provider(&request.provider)?;
    runtime_state.record_slice_audit_event(
        &slice,
        "auth.remove",
        "accepted",
        Some(&provider),
        None,
    )?;
    if slice.backend == crate::slice::SliceBackendKind::LocalDocker {
        let docker_options =
            crate::slice::LocalDockerSliceOptions::from_config(&config_projection.snapshot());
        let resolved_slice = slice.clone();
        let provider_for_action = provider.clone();
        let remove_result = tokio::task::spawn_blocking(move || {
            crate::slice::run_local_docker_slice_action(
                &resolved_slice,
                crate::slice::LocalDockerSliceAction::RemoveProviderAuth,
                None,
                Some(&provider_for_action),
                &docker_options,
            )
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "slice.auth.remove",
            message: format!("slice auth remove task failed: {error}"),
        })?;
        if let Err(error) = remove_result {
            let _ = runtime_state.record_slice_audit_event(
                &slice,
                "auth.remove",
                "failed",
                Some(&provider),
                Some(&error.to_string()),
            );
            return Err(error);
        }
        let provider_auth = slice
            .provider_auth
            .into_iter()
            .filter(|summary| !slice_auth_summary_matches_provider(&summary.provider, &provider))
            .collect::<Vec<_>>();
        let slice = runtime_state.set_slice_provider_auth(&request.slice_ref, provider_auth)?;
        runtime_state.record_slice_audit_event(
            &slice,
            "auth.remove",
            "completed",
            Some(&provider),
            None,
        )?;
        return Ok(LocalDaemonResponse::SliceProviderAuthRemoved {
            slice,
            provider,
            status: "removed".to_string(),
        });
    }
    runtime_state.record_slice_audit_event(
        &slice,
        "auth.remove",
        "not_implemented",
        Some(&provider),
        Some("slice auth removal is not implemented for this backend"),
    )?;
    Ok(LocalDaemonResponse::SliceProviderAuthRemoved {
        slice,
        provider,
        status: "not_implemented".to_string(),
    })
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

pub(crate) async fn execute_start_slice_provider_login_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    request: StartSliceProviderLoginRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let _operation = runtime_state.begin_slice_operation(&request.slice_ref, "slice.auth.login")?;
    let slice = runtime_state.resolve_slice(&request.slice_ref)?;
    runtime_state.record_slice_audit_event(
        &slice,
        "auth.login",
        "accepted",
        Some(&request.provider),
        None,
    )?;
    if slice.backend != crate::slice::SliceBackendKind::LocalDocker {
        let message = format!(
            "slice provider login is only implemented for local Docker slices, got `{:?}`",
            slice.backend
        );
        runtime_state.record_slice_audit_event(
            &slice,
            "auth.login",
            "failed",
            Some(&request.provider),
            Some(&message),
        )?;
        return Err(DaemonError::LocalTransport {
            operation: "slice.auth.login",
            message,
        });
    }
    let docker_options =
        crate::slice::LocalDockerSliceOptions::from_config(&config_projection.snapshot());
    let resolved_slice = slice.clone();
    let provider = request.provider.clone();
    let login_result = tokio::task::spawn_blocking(move || {
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
    })?;
    let login = match login_result {
        Ok(login) => login,
        Err(error) => {
            let _ = runtime_state.record_slice_audit_event(
                &slice,
                "auth.login",
                "failed",
                Some(&request.provider),
                Some(&error.to_string()),
            );
            return Err(error);
        }
    };
    runtime_state.record_slice_audit_event(
        &slice,
        "auth.login",
        "completed",
        Some(&request.provider),
        None,
    )?;
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
    runtime_state.record_slice_audit_event(
        &slice,
        "auth.alias",
        "completed",
        Some(&request.provider),
        None,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn auth(
        provider: &str,
        account: &str,
        alias: Option<&str>,
    ) -> crate::slice_provider_auth::SliceProviderAuthSummary {
        crate::slice_provider_auth::SliceProviderAuthSummary {
            provider: provider.to_string(),
            state: crate::slice_provider_auth::SliceProviderAuthState::Configured,
            auth_type: Some("test".to_string()),
            account_id: Some(account.to_string()),
            email: None,
            organization_id: None,
            organization_name: None,
            subscription_type: None,
            alias: alias.map(str::to_string),
            source: "test".to_string(),
        }
    }

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

    #[test]
    fn scoped_provider_auth_import_filters_requested_provider() {
        let summaries = vec![
            auth("codex", "codex-1", None),
            auth("opencode:openai", "openai-1", None),
            auth("opencode:opencode", "opencode-1", None),
            auth("claude", "claude-1", None),
        ];

        let codex = scoped_provider_auth_summaries("codex", summaries.clone());
        assert_eq!(
            codex
                .iter()
                .map(|auth| auth.provider.as_str())
                .collect::<Vec<_>>(),
            vec!["codex"]
        );

        let opencode = scoped_provider_auth_summaries("opencode", summaries);
        assert_eq!(
            opencode
                .iter()
                .map(|auth| auth.provider.as_str())
                .collect::<Vec<_>>(),
            vec!["opencode:openai", "opencode:opencode"]
        );

        let all = scoped_provider_auth_summaries(
            "all",
            vec![
                auth("codex", "codex-1", None),
                auth("opencode:openai", "openai-1", None),
                auth("claude", "claude-1", None),
            ],
        );
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn scoped_provider_auth_merge_preserves_other_providers_and_aliases() {
        let existing = vec![
            auth("codex", "old-codex", Some("Work")),
            auth("claude", "claude-1", Some("Claude")),
        ];
        let imported = vec![auth("codex", "new-codex", None)];

        let merged = merge_scoped_provider_auth(existing, "codex", imported);

        assert_eq!(merged.len(), 2);
        let codex = merged.iter().find(|auth| auth.provider == "codex").unwrap();
        let claude = merged
            .iter()
            .find(|auth| auth.provider == "claude")
            .unwrap();
        assert_eq!(codex.account_id.as_deref(), Some("new-codex"));
        assert_eq!(codex.alias.as_deref(), Some("Work"));
        assert_eq!(claude.account_id.as_deref(), Some("claude-1"));
    }

    #[test]
    fn scoped_provider_auth_remove_matches_provider_families() {
        let existing = vec![
            auth("codex", "codex-1", None),
            auth("opencode:openai", "openai-1", None),
            auth("opencode:opencode", "opencode-1", None),
            auth("claude", "claude-1", None),
        ];

        let remaining = existing
            .into_iter()
            .filter(|summary| !slice_auth_summary_matches_provider(&summary.provider, "opencode"))
            .map(|summary| summary.provider)
            .collect::<Vec<_>>();

        assert_eq!(remaining, vec!["codex", "claude"]);
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
}
