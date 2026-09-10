mod display_endpoint;
mod lifecycle;
mod provider_auth;
mod worker_discovery;

use crate::error::DaemonError;
use crate::local::{
    ImportSliceProviderAuthRequest, LocalDaemonRequest, LocalDaemonResponse,
    RemoveSliceProviderAuthRequest, StartSliceProviderLoginRequest,
};
use crate::runtime::command::KernelCaller;
use crate::runtime::projection::DaemonConfigProjectionStore;
use crate::runtime::state::KernelRuntimeState;
use crate::transport::relay_client::RelayClientState;
use std::sync::Arc;
use tokio::sync::RwLock;

use display_endpoint::execute_get_slice_display_endpoint_request;
pub(crate) use display_endpoint::register_room_selkies_display_endpoint;
use lifecycle::{
    execute_create_slice_backup_request, execute_create_slice_request,
    execute_delete_slice_request, execute_get_slice_logs_request, execute_get_slice_request,
    execute_get_slice_state_status_request, execute_list_slice_audit_request,
    execute_list_slices_request, execute_reset_slice_state_request,
    execute_restore_slice_backup_request, execute_save_slice_state_request,
    execute_start_slice_request, execute_stop_slice_request,
};
use provider_auth::{
    merge_profile_scoped_provider_auth, normalized_slice_provider, scoped_provider_auth_summaries,
    slice_auth_summary_matches_provider,
};

pub(crate) async fn execute_slice_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    relay_state: Option<Arc<RwLock<RelayClientState>>>,
    caller: &KernelCaller,
    session_id: Option<&str>,
    managed_kernel_registration: Option<
        &crate::managed_bootstrap::ConfirmedManagedKernelRegistration,
    >,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let owner_user_id = caller
        .user_id
        .as_deref()
        .unwrap_or(crate::session::DEFAULT_LOCAL_USER_ID);
    match request {
        LocalDaemonRequest::ListSlices(request) => {
            execute_list_slices_request(runtime_state, request).await
        }
        LocalDaemonRequest::CreateSlice(request) => {
            let request = managed_slice_create_request_for_session(
                runtime_state,
                request,
                managed_kernel_registration,
                session_id,
            )
            .await?;
            execute_create_slice_request(runtime_state, request).await
        }
        LocalDaemonRequest::GetSlice(request) => {
            execute_get_slice_request(runtime_state, request).await
        }
        LocalDaemonRequest::StartSlice(request) => {
            execute_start_slice_request(runtime_state, config_projection, relay_state, request)
                .await
        }
        LocalDaemonRequest::StopSlice(request) => {
            execute_stop_slice_request(runtime_state, config_projection, relay_state, request).await
        }
        LocalDaemonRequest::DeleteSlice(request) => {
            execute_delete_slice_request(runtime_state, config_projection, relay_state, request)
                .await
        }
        LocalDaemonRequest::ImportSliceProviderAuth(request) => {
            execute_import_slice_provider_auth_request(
                runtime_state,
                config_projection,
                owner_user_id,
                request,
            )
            .await
        }
        LocalDaemonRequest::RemoveSliceProviderAuth(request) => {
            execute_remove_slice_provider_auth_request(
                runtime_state,
                config_projection,
                owner_user_id,
                request,
            )
            .await
        }
        LocalDaemonRequest::StartSliceProviderLogin(request) => {
            execute_start_slice_provider_login_request(
                runtime_state,
                config_projection,
                owner_user_id,
                request,
            )
            .await
        }
        LocalDaemonRequest::GetSliceDisplayEndpoint(request) => {
            execute_get_slice_display_endpoint_request(
                runtime_state,
                config_projection,
                relay_state,
                caller,
                request,
            )
            .await
        }
        LocalDaemonRequest::GetSliceLogs(request) => {
            execute_get_slice_logs_request(runtime_state, config_projection, request).await
        }
        LocalDaemonRequest::ListSliceAudit(request) => {
            execute_list_slice_audit_request(runtime_state, request).await
        }
        LocalDaemonRequest::SaveSliceState(request) => {
            execute_save_slice_state_request(runtime_state, config_projection, relay_state, request)
                .await
        }
        LocalDaemonRequest::GetSliceStateStatus(request) => {
            execute_get_slice_state_status_request(runtime_state, request).await
        }
        LocalDaemonRequest::ResetSliceState(request) => {
            execute_reset_slice_state_request(runtime_state, config_projection, request).await
        }
        LocalDaemonRequest::CreateSliceBackup(request) => {
            execute_create_slice_backup_request(runtime_state, config_projection, request).await
        }
        LocalDaemonRequest::RestoreSliceBackup(request) => {
            execute_restore_slice_backup_request(runtime_state, config_projection, request).await
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "slice request",
            message: "unsupported slice request".to_string(),
        }),
    }
}

fn managed_slice_create_request(
    mut request: crate::local::CreateSliceRequest,
    registration: Option<&crate::managed_bootstrap::ConfirmedManagedKernelRegistration>,
) -> crate::local::CreateSliceRequest {
    if request.backend == crate::slice::SliceBackendKind::LocalDocker
        && request.development.is_none()
    {
        request.development = registration
            .and_then(|registration| registration.context_plan.as_ref())
            .map(|plan| plan.package_binding().development);
    }
    request
}

async fn managed_slice_create_request_for_session(
    runtime_state: &KernelRuntimeState,
    request: crate::local::CreateSliceRequest,
    registration: Option<&crate::managed_bootstrap::ConfirmedManagedKernelRegistration>,
    session_id: Option<&str>,
) -> Result<crate::local::CreateSliceRequest, DaemonError> {
    let derives_managed_default = request.backend == crate::slice::SliceBackendKind::LocalDocker
        && request.development.is_none();
    let mut request = managed_slice_create_request(request, registration);
    if !derives_managed_default
        || request.development
            != Some(crate::managed_context::package::ManagedContextDevelopmentSelection::Empty)
    {
        return Ok(request);
    }
    let Some(session_id) = session_id else {
        return Ok(request);
    };
    let session = runtime_state.session_snapshot(session_id).await?;
    let (Some(workspace_id), Some(worktree_id)) = (
        request.workspace_id.as_deref(),
        request.worktree_id.as_deref(),
    ) else {
        return Ok(request);
    };
    if workspace_id != session.workspace_id() {
        return Ok(request);
    }
    request.development =
        Some(runtime_state.slice_development_selection_for_session(&session, worktree_id)?);
    Ok(request)
}

pub(crate) async fn execute_import_slice_provider_auth_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    owner_user_id: &str,
    request: ImportSliceProviderAuthRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let _operation =
        runtime_state.begin_slice_operation(&request.slice_ref, "slice.auth.import")?;
    let slice = runtime_state.resolve_slice(&request.slice_ref)?;
    let provider = normalized_slice_provider(&request.provider)?;
    let provider_account = resolve_local_docker_provider_account(
        runtime_state,
        owner_user_id,
        &provider,
        &request.account_profile,
    )?;
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
        let provider_account_for_action = provider_account.clone();
        let import_result = tokio::task::spawn_blocking(move || {
            crate::slice::run_local_docker_slice_action(
                &resolved_slice,
                crate::slice::LocalDockerSliceAction::ImportProviderAuth,
                None,
                Some(&provider_for_action),
                Some(&provider_account_for_action),
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
        let verified_provider_auth = crate::slice::inspect_local_docker_slice_provider_auth(
            &slice,
            &provider,
            Some(&provider_account),
        )?;
        if provider != "all" && verified_provider_auth.is_empty() {
            let message = format!(
                "{provider} credentials were not found in slice `{}` after import",
                slice.name
            );
            runtime_state.record_slice_audit_event(
                &slice,
                "auth.import",
                "failed",
                Some(&provider),
                Some(&message),
            )?;
            return Err(DaemonError::LocalTransport {
                operation: "slice.auth.import",
                message,
            });
        }
        let imported_provider_auth = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .map(|home| crate::slice_provider_auth::inspect_home_provider_auth(&home))
            .map(|summaries| {
                scoped_provider_auth_summaries(&provider, summaries)
                    .into_iter()
                    .map(|mut summary| {
                        summary.account_profile = provider_account.profile_id.clone();
                        summary
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let imported_provider_auth = crate::slice_provider_auth::merge_provider_auth_summaries(
            imported_provider_auth
                .into_iter()
                .chain(verified_provider_auth)
                .collect(),
        );
        let provider_auth = merge_profile_scoped_provider_auth(
            slice.provider_auth,
            &provider,
            &provider_account.profile_id,
            imported_provider_auth,
        );
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
    let message = format!(
        "slice auth import is only implemented for local Docker slices, got `{:?}`",
        slice.backend
    );
    runtime_state.record_slice_audit_event(
        &slice,
        "auth.import",
        "failed",
        Some(&provider),
        Some(&message),
    )?;
    Err(DaemonError::LocalTransport {
        operation: "slice.auth.import",
        message,
    })
}

pub(crate) async fn execute_remove_slice_provider_auth_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    owner_user_id: &str,
    request: RemoveSliceProviderAuthRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let _operation =
        runtime_state.begin_slice_operation(&request.slice_ref, "slice.auth.remove")?;
    let slice = runtime_state.resolve_slice(&request.slice_ref)?;
    let provider = normalized_slice_provider(&request.provider)?;
    let provider_account = resolve_local_docker_provider_account(
        runtime_state,
        owner_user_id,
        &provider,
        &request.account_profile,
    )?;
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
        let provider_account_for_action = provider_account.clone();
        let remove_result = tokio::task::spawn_blocking(move || {
            crate::slice::run_local_docker_slice_action(
                &resolved_slice,
                crate::slice::LocalDockerSliceAction::RemoveProviderAuth,
                None,
                Some(&provider_for_action),
                Some(&provider_account_for_action),
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
            .filter(|summary| {
                !slice_auth_summary_matches_provider(&summary.provider, &provider)
                    || summary.account_profile != provider_account.profile_id
            })
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
    let message = format!(
        "slice auth removal is only implemented for local Docker slices, got `{:?}`",
        slice.backend
    );
    runtime_state.record_slice_audit_event(
        &slice,
        "auth.remove",
        "failed",
        Some(&provider),
        Some(&message),
    )?;
    Err(DaemonError::LocalTransport {
        operation: "slice.auth.remove",
        message,
    })
}

pub(crate) async fn execute_start_slice_provider_login_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    owner_user_id: &str,
    request: StartSliceProviderLoginRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let _operation = runtime_state.begin_slice_operation(&request.slice_ref, "slice.auth.login")?;
    let slice = runtime_state.resolve_slice(&request.slice_ref)?;
    let provider_account = resolve_local_docker_provider_account(
        runtime_state,
        owner_user_id,
        &request.provider,
        &request.account_profile,
    )?;
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
    let provider_account_for_login = provider_account.clone();
    let login_result = tokio::task::spawn_blocking(move || {
        crate::slice::start_local_docker_slice_provider_login(
            &resolved_slice,
            &provider,
            &provider_account_for_login,
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

fn resolve_local_docker_provider_account(
    runtime_state: &KernelRuntimeState,
    owner_user_id: &str,
    provider: &str,
    account_profile: &str,
) -> Result<crate::slice::LocalDockerProviderAccount, DaemonError> {
    let owner_user_id = runtime_state.provider_account_authority_owner_user_id(owner_user_id);
    let registry = runtime_state.provider_account_profile_registry();
    let profile = registry.get(&owner_user_id, provider, account_profile)?;
    Ok(crate::slice::LocalDockerProviderAccount {
        owner_path_component: crate::account_profile::account_owner_path_component(&owner_user_id),
        profile_id: profile.profile_id.clone(),
        environment: registry.resolve_environment(&owner_user_id, provider, &profile.profile_id)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{DaemonApp, KernelSessionService};
    use crate::managed_bootstrap::{ConfirmedManagedKernelRegistration, ManagedKernelContextPlan};
    use crate::managed_context::package::ManagedContextDevelopmentSelection;
    use crate::runtime::router::CommandRouter;
    use crate::session::CreateSessionRequest;
    use tokio::sync::Mutex;

    fn create_request() -> crate::local::CreateSliceRequest {
        crate::local::CreateSliceRequest {
            name: "browser-work".to_string(),
            backend: crate::slice::SliceBackendKind::LocalDocker,
            os: "linux".to_string(),
            display_mode: crate::slice::SliceDisplayMode::Headed,
            display_backend: Default::default(),
            workspace_id: Some("/home/chariox".to_string()),
            worktree_id: Some("/home/chariox".to_string()),
            workspace_mount: Some("/home/chariox".to_string()),
            development: None,
            worker_kernel_ref: None,
            display_url: None,
            provider_auth: Vec::new(),
            from_saved_state: None,
            base: Some(crate::local::SliceCreateBase::Clean),
        }
    }

    fn empty_registration() -> ConfirmedManagedKernelRegistration {
        ConfirmedManagedKernelRegistration {
            environment_id: "environment-1".to_string(),
            machine_id: "machine-1".to_string(),
            kernel_id: "kernel-1".to_string(),
            context_plan: Some(ManagedKernelContextPlan::empty_for_tests("context-1")),
        }
    }

    #[tokio::test]
    async fn managed_empty_slice_uses_the_selected_session_directory_project() {
        let root = std::env::temp_dir().join(format!(
            "chariox-managed-slice-directory-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).expect("create directory Workspace");
        std::fs::write(workspace.join("notes.txt"), "selected directory contents\n")
            .expect("write directory Workspace file");

        let workspace_id = workspace.to_string_lossy().into_owned();
        let mut config = crate::config::DaemonConfig::for_tests();
        config.user_config.slices.root = Some(root.join("slices").to_string_lossy().into_owned());
        let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
        let (session, _) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(&workspace_id, &workspace_id))
            .expect("create current session");
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1)
            .with_managed_kernel_registration(empty_registration());
        let mut request = create_request();
        request.workspace_id = Some(workspace_id.clone());
        request.worktree_id = Some(workspace_id.clone());
        request.workspace_mount = Some(workspace_id.clone());
        let request = LocalDaemonRequest::CreateSlice(request);
        let mut command = crate::runtime::command::KernelCommand::from_local_request(
            "create-managed-directory-slice",
            None,
            None,
            &request,
        );
        command.session_id = Some(session.id().to_string());

        let LocalDaemonResponse::SliceCreated { slice } = router
            .dispatch(command, request)
            .await
            .expect("create managed slice")
        else {
            panic!("expected created slice");
        };
        let Some(ManagedContextDevelopmentSelection::SourceProject {
            project_id,
            repositories,
        }) = slice.development
        else {
            panic!("managed slice should select the current session Project");
        };
        assert_eq!(project_id, session.project_id());
        assert_eq!(repositories.len(), 1);
        let resolved = crate::managed_context::outbound_service::resolve_repository_selection(
            &repositories[0],
        )
        .expect("resolve kernel-authorized directory selection");
        assert_eq!(
            std::fs::read_to_string(resolved.worktree_path.join("notes.txt"))
                .expect("read selected directory file"),
            "selected directory contents\n"
        );
        assert!(!resolved.worktree_path.join(".git").exists());

        std::fs::remove_dir_all(root).expect("clean test root");
    }

    #[test]
    fn client_slice_create_inherits_the_managed_development_plan() {
        let request = managed_slice_create_request(create_request(), Some(&empty_registration()));

        assert_eq!(
            request.development,
            Some(ManagedContextDevelopmentSelection::Empty)
        );
    }

    #[test]
    fn ordinary_and_explicit_slice_development_are_not_rewritten() {
        let ordinary = managed_slice_create_request(create_request(), None);
        assert_eq!(ordinary.development, None);

        let mut explicit = create_request();
        explicit.development = Some(ManagedContextDevelopmentSelection::Empty);
        let explicit = managed_slice_create_request(explicit, Some(&empty_registration()));
        assert_eq!(
            explicit.development,
            Some(ManagedContextDevelopmentSelection::Empty)
        );
    }

    fn auth(provider: &str, account: &str) -> crate::slice_provider_auth::SliceProviderAuthSummary {
        crate::slice_provider_auth::SliceProviderAuthSummary {
            provider: provider.to_string(),
            account_profile: "default".to_string(),
            state: crate::slice_provider_auth::SliceProviderAuthState::Configured,
            auth_type: Some("test".to_string()),
            account_id: Some(account.to_string()),
            email: None,
            organization_id: None,
            organization_name: None,
            subscription_type: None,
            source: "test".to_string(),
        }
    }

    #[test]
    fn scoped_provider_auth_import_filters_requested_provider() {
        let summaries = vec![
            auth("codex", "codex-1"),
            auth("opencode:openai", "openai-1"),
            auth("opencode:opencode", "opencode-1"),
            auth("claude", "claude-1"),
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
                auth("codex", "codex-1"),
                auth("opencode:openai", "openai-1"),
                auth("claude", "claude-1"),
            ],
        );
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn scoped_provider_auth_remove_matches_provider_families() {
        let existing = vec![
            auth("codex", "codex-1"),
            auth("opencode:openai", "openai-1"),
            auth("opencode:opencode", "opencode-1"),
            auth("claude", "claude-1"),
        ];

        let remaining = existing
            .into_iter()
            .filter(|summary| !slice_auth_summary_matches_provider(&summary.provider, "opencode"))
            .map(|summary| summary.provider)
            .collect::<Vec<_>>();

        assert_eq!(remaining, vec!["codex", "claude"]);
    }
}
