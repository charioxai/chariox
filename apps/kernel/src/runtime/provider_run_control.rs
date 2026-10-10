use crate::error::DaemonError;
use crate::local::{
    GetProviderRunRequest, LocalDaemonRequest, LocalDaemonResponse, LogoutProviderRequest,
    UpdateProviderRunSelectionRequest,
};
use crate::runtime::projection::{
    ProviderCatalogProjectionStore, ProviderProcessProjectionStore, ProviderRunProjectionStore,
};
use crate::runtime::provider_auth_control::execute_logout_provider_request as execute_provider_logout;
use crate::runtime::state::KernelRuntimeState;

pub(crate) async fn execute_provider_run_request(
    runtime_state: &KernelRuntimeState,
    provider_catalog_projection: &ProviderCatalogProjectionStore,
    caller_user_id: &str,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::GetProviderRun(request) => {
            execute_get_provider_run_request(runtime_state, request).await
        }
        LocalDaemonRequest::UpdateProviderRunSelection(request) => {
            execute_update_provider_run_selection_request(runtime_state, request).await
        }
        LocalDaemonRequest::LogoutProvider(request) => {
            execute_logout_provider_and_invalidate_catalog_request(
                runtime_state,
                provider_catalog_projection,
                caller_user_id,
                request,
            )
            .await
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "provider run request",
            message: "unsupported provider run request".to_string(),
        }),
    }
}

pub(crate) async fn execute_get_provider_run_request(
    runtime_state: &KernelRuntimeState,
    request: GetProviderRunRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    runtime_state.provider_run_response(request)
}

pub(crate) async fn execute_update_provider_run_selection_request(
    runtime_state: &KernelRuntimeState,
    request: UpdateProviderRunSelectionRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    runtime_state.update_provider_run_selection_response(request)
}

pub(crate) fn projected_provider_run_response(
    provider_run_projection: &ProviderRunProjectionStore,
    request: &GetProviderRunRequest,
    caller_user_id: &str,
) -> Result<Option<LocalDaemonResponse>, DaemonError> {
    let Some(provider_run) = provider_run_projection.get(&request.provider_run_id) else {
        return Ok(None);
    };
    ensure_provider_run_visible_to_user(&provider_run, caller_user_id)?;
    // Leased runs have no local provider registry entry to refresh.
    // Their selection is owned by the worker and delivered in this projection.
    let leased_run = provider_run.id().starts_with("leased:");
    if crate::provider::provider_run_refreshes_selection_on_read(&provider_run) && !leased_run {
        return Ok(None);
    }
    Ok(Some(LocalDaemonResponse::ProviderRun {
        provider_run: provider_run.into(),
    }))
}

pub(crate) fn invalidate_provider_process_projection_from_response(
    provider_process_projection: &ProviderProcessProjectionStore,
    result: &Result<LocalDaemonResponse, DaemonError>,
) {
    match result {
        Ok(LocalDaemonResponse::ProviderRun { .. })
        | Ok(LocalDaemonResponse::ProviderRunLaunched { .. })
        | Ok(LocalDaemonResponse::ProviderRunLaunchAccepted { .. }) => {
            // Launch/read services publish the authoritative private run before returning.
            // Public responses cannot restore provider credentials/capabilities.
            provider_process_projection.invalidate();
        }
        _ => {}
    }
}

pub(crate) async fn execute_logout_provider_and_invalidate_catalog_request(
    runtime_state: &KernelRuntimeState,
    provider_catalog_projection: &ProviderCatalogProjectionStore,
    caller_user_id: &str,
    request: LogoutProviderRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let response = execute_provider_logout(runtime_state, caller_user_id, request).await?;
    invalidate_provider_catalog_caches(provider_catalog_projection);
    Ok(response)
}

pub(crate) fn invalidate_provider_catalog_caches(
    provider_catalog_projection: &ProviderCatalogProjectionStore,
) {
    provider_catalog_projection.invalidate();
}

pub(crate) fn ensure_provider_run_visible_to_user(
    provider_run: &crate::provider::RuntimeProviderRun,
    caller_user_id: &str,
) -> Result<(), DaemonError> {
    if provider_run.owned_by(caller_user_id) {
        Ok(())
    } else {
        Err(DaemonError::OwnershipAccessDenied {
            user_id: caller_user_id.to_string(),
            owner_user_id: provider_run.owner_user_id().to_string(),
            resource: format!("provider run `{}`", provider_run.id()),
            operation: "read provider run",
        })
    }
}

#[cfg(test)]
mod mp11_f7_tests {
    use super::*;
    use crate::provider::RuntimeProviderRun;

    #[test]
    fn mp08_mp10_mp11_live_leased_selection_reads_use_worker_projection() {
        let projection = ProviderRunProjectionStore::default();
        let mut run = RuntimeProviderRun::from_control_capability_inference(
            "leased:lease:worker-opencode",
            "session".into(),
            Some("agent".into()),
            "opencode".into(),
        );
        run.set_runtime_mcp_auth_token(Some("mp11-private-sentinel".into()));
        assert!(crate::provider::provider_run_refreshes_selection_on_read(
            &run
        ));
        let starting_run = run.clone();
        for state in [
            crate::provider::ProviderRunState::Running,
            crate::provider::ProviderRunState::Starting,
            crate::provider::ProviderRunState::Parked,
            crate::provider::ProviderRunState::Ended,
        ] {
            match state {
                crate::provider::ProviderRunState::Running => run.mark_running(),
                crate::provider::ProviderRunState::Starting => run = starting_run.clone(),
                crate::provider::ProviderRunState::Parked => run.mark_parked(),
                crate::provider::ProviderRunState::Ended => run.mark_ended(),
            }
            projection.update(run.clone());
            let request = GetProviderRunRequest {
                provider_run_id: run.id().into(),
            };
            let response = projected_provider_run_response(&projection, &request, "local")
                .unwrap()
                .expect("worker-owned selection must come from the worker projection");
            let LocalDaemonResponse::ProviderRun { provider_run } = &response else {
                panic!("unexpected response");
            };
            assert_eq!(provider_run.state(), state);
            assert!(!serde_json::to_string(&response)
                .unwrap()
                .contains("mp11-private-sentinel"));
            assert!(matches!(
                projected_provider_run_response(&projection, &request, "foreign-user"),
                Err(DaemonError::OwnershipAccessDenied { .. })
            ));
            assert_eq!(projection.get(run.id()).unwrap(), run);
        }
        let local_run = RuntimeProviderRun::from_control_capability_inference(
            "local-opencode",
            "session".into(),
            Some("agent".into()),
            "opencode".into(),
        );
        projection.update(local_run.clone());
        assert!(projected_provider_run_response(
            &projection,
            &GetProviderRunRequest {
                provider_run_id: local_run.id().into()
            },
            "local"
        )
        .unwrap()
        .is_none());
    }

    #[test]
    fn mp11_f7_read_projection_keeps_private_authority_and_enforces_owner() {
        let projection = ProviderRunProjectionStore::default();
        let mut run = RuntimeProviderRun::from_control_capability_inference(
            "leased:lease:worker-run",
            "session".into(),
            Some("agent".into()),
            "codex".into(),
        );
        run.set_runtime_mcp_auth_token(Some("mp11-private-sentinel".into()));
        run.mark_ended();
        projection.update(run.clone());
        let request = GetProviderRunRequest {
            provider_run_id: run.id().into(),
        };
        let response = projected_provider_run_response(&projection, &request, "local")
            .unwrap()
            .unwrap();
        let bytes = serde_json::to_string(&response).unwrap();
        assert!(!bytes.contains("mp11-private-sentinel"));
        assert!(!bytes.contains("runtime_mcp_auth_token"));
        assert!(matches!(
            projected_provider_run_response(&projection, &request, "foreign-user"),
            Err(DaemonError::OwnershipAccessDenied { .. })
        ));
        assert_eq!(projection.get(run.id()).unwrap(), run);
    }
}
