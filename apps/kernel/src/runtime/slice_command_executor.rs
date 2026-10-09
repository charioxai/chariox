mod display_endpoint;
mod lifecycle;
#[cfg(test)]
mod local_api_tests;
mod provider_auth;
mod worker_discovery;

use crate::error::DaemonError;
use crate::local::{
    ImportSliceProviderAuthRequest, LocalDaemonRequest, LocalDaemonResponse,
    RemoveSliceProviderAuthRequest, StartSliceProviderLoginRequest,
};
use crate::managed_context::package::ManagedContextDevelopmentSelection;
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
            let mut request = request;
            match inherited_slice_development(&request, managed_kernel_registration) {
                InheritedSliceDevelopment::None => {}
                InheritedSliceDevelopment::Plan(development) => {
                    request.development = Some(development)
                }
                InheritedSliceDevelopment::ImportedProject(project_id) => {
                    let (development, workspace_id, worktree_id) = runtime_state
                        .slice_development_selection_for_project(
                            &project_id,
                            request.workspace_id.as_deref(),
                            request.worktree_id.as_deref(),
                        )?;
                    request.workspace_id = Some(workspace_id);
                    request.worktree_id = Some(worktree_id);
                    request.development = Some(development);
                }
            }
            execute_create_slice_request(runtime_state, request).await
        }
        LocalDaemonRequest::GetSlice(request) => {
            execute_get_slice_request(runtime_state, request).await
        }
        LocalDaemonRequest::StartSlice(request) => {
            let slice = runtime_state.resolve_slice(&request.slice_ref)?;
            let target_git_available = managed_kernel_registration.is_some()
                && crate::managed_context::scm::GitCredentialCommandContext::source_from_process()
                    .is_ok_and(|context| {
                        crate::managed_context::scm::github_credential_is_available(&context)
                    });
            let inherit_git = managed_slice_should_inherit_git_credentials(
                &slice.provider_auth,
                managed_kernel_registration,
                target_git_available,
            );
            execute_start_slice_request(
                runtime_state,
                config_projection,
                relay_state,
                request,
                inherit_git,
            )
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

/// The development a managed kernel's client slice inherits from its context
/// plan. A source-project plan names the source kernel's Workspaces, so the
/// slice instead develops this kernel's imported copy of that Project.
enum InheritedSliceDevelopment {
    None,
    Plan(ManagedContextDevelopmentSelection),
    ImportedProject(String),
}

fn inherited_slice_development(
    request: &crate::local::CreateSliceRequest,
    registration: Option<&crate::managed_bootstrap::ConfirmedManagedKernelRegistration>,
) -> InheritedSliceDevelopment {
    if request.backend != crate::slice::SliceBackendKind::LocalDocker
        || request.development.is_some()
    {
        return InheritedSliceDevelopment::None;
    }
    match registration
        .and_then(|registration| registration.context_plan.as_ref())
        .map(|plan| plan.package_binding().development)
    {
        None => InheritedSliceDevelopment::None,
        Some(ManagedContextDevelopmentSelection::SourceProject { project_id, .. }) => {
            InheritedSliceDevelopment::ImportedProject(project_id)
        }
        Some(development) => InheritedSliceDevelopment::Plan(development),
    }
}

fn managed_slice_should_inherit_git_credentials(
    provider_auth: &[crate::slice_provider_auth::SliceProviderAuthSummary],
    registration: Option<&crate::managed_bootstrap::ConfirmedManagedKernelRegistration>,
    target_git_available: bool,
) -> bool {
    registration.is_some()
        && target_git_available
        && !provider_auth.iter().any(|summary| {
            summary.provider == "github"
                && matches!(
                    summary.state,
                    crate::slice_provider_auth::SliceProviderAuthState::Configured
                        | crate::slice_provider_auth::SliceProviderAuthState::Authenticated
                )
        })
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
        let registry = runtime_state.provider_account_profile_registry();
        let owner = runtime_state.provider_account_authority_owner_user_id(owner_user_id);
        let materialization = registry.export_managed_context_materialization(
            &owner,
            &provider,
            &provider_account.profile_id,
        )?;
        let relay = lifecycle::local_docker_slice_relay(config_projection, &slice).await?;
        let config = relay.worker_discovery_config(config_projection.snapshot());
        let target = worker_discovery::discover_started_slice_worker(&config, &slice).await?;
        let expected_copy = registry.prepare_account_copy(
            &owner,
            &materialization,
            crate::account_profile::ProviderAccountMaterializationTargetKind::Slice,
            &target.machine_id,
            &target.kernel_id,
        )?;
        let response = runtime_state.send_remote_profile_request(
            &config, &target.kernel_id,
            crate::transport::relay_peer::RelayPeerRequest::ImportManagedSliceProviderAccountCopy {
                slice_id: slice.id.clone(), materialization,
            },
        ).await?;
        let crate::transport::relay_peer::RelayPeerResponse::ManagedSliceProviderAccountCopyImported { profile: received } = response else {
            return Err(DaemonError::LocalTransport { operation: "slice.auth.import", message: "receiving kernel did not confirm account import".into() });
        };
        let status = received
            .materializations
            .iter()
            .find(|status| {
                status.copy.as_ref().is_some_and(|copy| {
                    copy.source_account_id == provider_account.profile_id
                        && copy.source_machine_id == config.host_machine_id
                        && copy.source_kernel_id == config.daemon_id
                        && copy.target_kernel_id == target.kernel_id
                        && copy.target_machine_id == target.machine_id
                        && received.provider == provider
                        && copy.target_account_id == received.profile_id
                })
            })
            .cloned()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "slice.auth.import",
                message: "receiving kernel did not confirm copy provenance".into(),
            })?;
        registry.record_confirmed_account_copy(
            &owner,
            &expected_copy,
            crate::account_profile::ProviderAccountMaterializationTargetKind::Slice,
            &target.machine_id,
            &target.kernel_id,
            &received.profile_id,
            status,
        )?;
        let mut provider_auth = slice.provider_auth.clone();
        provider_auth.retain(|summary| {
            !(summary.provider == provider && summary.account_profile == received.profile_id)
        });
        provider_auth.push(crate::slice_provider_auth::SliceProviderAuthSummary {
            provider: provider.clone(),
            account_profile: received.profile_id,
            state: match received.auth_state {
                crate::account_profile::ProviderAccountAuthState::Authenticated => {
                    crate::slice_provider_auth::SliceProviderAuthState::Authenticated
                }
                crate::account_profile::ProviderAccountAuthState::NotConfigured
                | crate::account_profile::ProviderAccountAuthState::Expired => {
                    crate::slice_provider_auth::SliceProviderAuthState::NotConfigured
                }
                _ => crate::slice_provider_auth::SliceProviderAuthState::Unknown,
            },
            auth_type: None,
            account_id: None,
            email: None,
            organization_id: None,
            organization_name: None,
            subscription_type: None,
            source: "managed_account_copy".into(),
        });
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
        let owner = runtime_state.provider_account_authority_owner_user_id(owner_user_id);
        runtime_state
            .provider_account_profile_registry()
            .observe_target_copy(
                &owner,
                &provider,
                &slice.worker_kernel_ref,
                &provider_account.profile_id,
                crate::account_profile::ProviderAccountCopyAuthState::Removed,
            )?;
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
    use crate::managed_bootstrap::{ConfirmedManagedKernelRegistration, ManagedKernelContextPlan};

    fn create_request() -> crate::local::CreateSliceRequest {
        crate::local::CreateSliceRequest {
            source_slice_ref: None,
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

    #[test]
    fn a_source_project_plan_defers_to_the_imported_project() {
        let registration = ConfirmedManagedKernelRegistration {
            context_plan: Some(ManagedKernelContextPlan::source_project_for_tests(
                "context-1",
                "realm-1",
                "source-kernel",
                "thumbprint",
                "project-1",
            )),
            ..empty_registration()
        };
        assert!(matches!(
            inherited_slice_development(&create_request(), Some(&registration)),
            InheritedSliceDevelopment::ImportedProject(project) if project == "project-1"
        ));
    }

    #[test]
    fn client_slice_create_inherits_the_managed_development_plan() {
        assert!(matches!(
            inherited_slice_development(&create_request(), Some(&empty_registration())),
            InheritedSliceDevelopment::Plan(ManagedContextDevelopmentSelection::Empty)
        ));
    }

    #[test]
    fn ordinary_and_explicit_slice_development_are_not_rewritten() {
        assert!(matches!(
            inherited_slice_development(&create_request(), None),
            InheritedSliceDevelopment::None
        ));
        let mut explicit = create_request();
        explicit.development = Some(ManagedContextDevelopmentSelection::Empty);
        assert!(matches!(
            inherited_slice_development(&explicit, Some(&empty_registration())),
            InheritedSliceDevelopment::None
        ));
    }

    #[test]
    fn managed_slice_inherits_target_owned_git_credentials_once() {
        let registration = empty_registration();
        assert!(managed_slice_should_inherit_git_credentials(
            &[],
            Some(&registration),
            true,
        ));

        let configured = vec![auth("github", "github.com")];
        assert!(!managed_slice_should_inherit_git_credentials(
            &configured,
            Some(&registration),
            true,
        ));
        assert!(!managed_slice_should_inherit_git_credentials(
            &[],
            None,
            true,
        ));
        assert!(!managed_slice_should_inherit_git_credentials(
            &[],
            Some(&registration),
            false,
        ));
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
