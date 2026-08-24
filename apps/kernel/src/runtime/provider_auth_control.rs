use base64::Engine as _;
use rand::{Rng, distributions::Alphanumeric};

use crate::error::DaemonError;
use crate::local::provider_requests::{
    logout_provider_response, provider_auth_status_response, start_provider_login_response,
};
use crate::local::{
    CancelProviderLoginRequest, GetProviderAuthStatusRequest, GetProviderLoginStatusRequest,
    LocalDaemonRequest, LocalDaemonResponse, LogoutProviderRequest, ProviderLoginProcessState,
    SendProviderLoginInputRequest, StartProviderLoginRequest,
};
use crate::provider::ProviderLoginStart;
use crate::pty::{PtyProcessState, PtySpawnRequest};
use crate::runtime::state::KernelRuntimeState;

const PROVIDER_LOGIN_TIMEOUT_MS: u64 = 10 * 60 * 1_000;

pub(crate) async fn execute_provider_auth_request(
    runtime_state: &KernelRuntimeState,
    owner_user_id: &str,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let owner_user_id = runtime_state.provider_account_authority_owner_user_id(owner_user_id);
    match request {
        LocalDaemonRequest::GetProviderAuthStatus(request) => {
            execute_get_provider_auth_status_request(runtime_state, &owner_user_id, request).await
        }
        LocalDaemonRequest::StartProviderLogin(request) => {
            execute_start_provider_login_request(runtime_state, &owner_user_id, request).await
        }
        LocalDaemonRequest::GetProviderLoginStatus(request) => {
            execute_get_provider_login_status_request(runtime_state, &owner_user_id, request).await
        }
        LocalDaemonRequest::SendProviderLoginInput(request) => {
            execute_send_provider_login_input_request(runtime_state, &owner_user_id, request).await
        }
        LocalDaemonRequest::CancelProviderLogin(request) => {
            execute_cancel_provider_login_request(runtime_state, &owner_user_id, request).await
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "provider auth request",
            message: "unsupported provider auth request".to_string(),
        }),
    }
}

pub(crate) async fn execute_get_provider_auth_status_request(
    runtime_state: &KernelRuntimeState,
    owner_user_id: &str,
    request: GetProviderAuthStatusRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry = runtime_state.provider_account_profile_registry().clone();
    let owner_user_id = owner_user_id.to_string();
    tokio::task::spawn_blocking(move || {
        provider_auth_status_response(&registry, &owner_user_id, request)
    })
    .await
    .map_err(|error| provider_auth_task_error("get provider auth status", error))?
}

async fn start_terminal_provider_auth(
    runtime_state: &KernelRuntimeState,
    owner_user_id: &str,
    provider: String,
    account_profile: String,
    operation: crate::runtime::state::ProviderAuthProcessOperation,
) -> Result<LocalDaemonResponse, DaemonError> {
    let provider = crate::provider::canonical_provider_family(&provider)
        .ok_or_else(|| provider_login_error("unsupported provider"))?;
    if operation == crate::runtime::state::ProviderAuthProcessOperation::Login
        && !matches!(provider, "claude" | "opencode")
    {
        return Err(provider_login_error(format!(
            "provider `{}` does not expose a provider-native login command",
            provider
        )));
    }
    if operation == crate::runtime::state::ProviderAuthProcessOperation::Logout
        && provider != "opencode"
    {
        return Err(provider_login_error(format!(
            "provider `{provider}` does not require an interactive logout workflow"
        )));
    }
    let registry = runtime_state.provider_account_profile_registry();
    let profile = registry.get(owner_user_id, provider, &account_profile)?;
    let environment = registry.resolve_environment(owner_user_id, provider, &profile.profile_id)?;
    let (program, args) = match (provider, operation) {
        ("claude", crate::runtime::state::ProviderAuthProcessOperation::Login) => (
            crate::provider::resolve_claude_executable()?,
            vec!["auth".to_string(), "login".to_string()],
        ),
        ("opencode", crate::runtime::state::ProviderAuthProcessOperation::Login) => (
            crate::provider::resolve_opencode_executable()?,
            vec!["auth".to_string(), "login".to_string()],
        ),
        ("opencode", crate::runtime::state::ProviderAuthProcessOperation::Logout) => (
            crate::provider::resolve_opencode_executable()?,
            vec!["auth".to_string(), "logout".to_string()],
        ),
        _ => unreachable!(),
    };
    let login_id = format!(
        "provider-login-{}",
        rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(16)
            .map(char::from)
            .collect::<String>()
            .to_lowercase()
    );
    let now_ms = crate::session::unix_epoch_ms();
    runtime_state.provider_login_process_store().insert(
        crate::runtime::state::ProviderLoginProcessRecord {
            owner_user_id: owner_user_id.to_string(),
            provider: provider.to_string(),
            account_profile: profile.profile_id.clone(),
            login_id: login_id.clone(),
            state: ProviderLoginProcessState::Running,
            backend: crate::runtime::state::ProviderLoginProcessBackend::Terminal,
            operation,
            output: Vec::new(),
            started_at_ms: now_ms,
            updated_at_ms: now_ms,
        },
    )?;
    let env_remove = crate::account_profile::provider_auth_env_vars(provider)
        .iter()
        .map(|name| (*name).to_string())
        .collect::<Vec<_>>();
    let spawn = runtime_state
        .with_app_side_effect(|app| {
            app.pty_mut().spawn(PtySpawnRequest {
                process_key: login_id.clone(),
                provider_run_id: login_id.clone(),
                program: program.to_string_lossy().to_string(),
                args,
                env: environment,
                env_remove,
                working_directory: None,
                cols: 120,
                rows: 40,
            })
        })
        .await;
    if let Err(error) = spawn {
        runtime_state
            .provider_login_process_store()
            .remove(&login_id);
        return Err(error);
    }
    let workflow = ProviderLoginStart {
        provider: provider.to_string(),
        account_profile: profile.profile_id,
        login_kind: if operation == crate::runtime::state::ProviderAuthProcessOperation::Login {
            "terminal".to_string()
        } else {
            "terminal_logout".to_string()
        },
        login_id: Some(login_id),
        auth_url: None,
        verification_url: None,
        user_code: None,
    };
    Ok(
        if operation == crate::runtime::state::ProviderAuthProcessOperation::Login {
            LocalDaemonResponse::ProviderLoginStarted { login: workflow }
        } else {
            LocalDaemonResponse::ProviderLogoutStarted { logout: workflow }
        },
    )
}

pub(crate) async fn execute_get_provider_login_status_request(
    runtime_state: &KernelRuntimeState,
    owner_user_id: &str,
    request: GetProviderLoginStatusRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let record = runtime_state
        .provider_login_process_store()
        .record_for_owner(owner_user_id, &request.login_id)?;
    if record.state != ProviderLoginProcessState::Running {
        return Ok(LocalDaemonResponse::ProviderLoginStatus {
            login: record.status(),
        });
    }
    let now_ms = crate::session::unix_epoch_ms();
    if now_ms.saturating_sub(record.started_at_ms) >= PROVIDER_LOGIN_TIMEOUT_MS {
        if record.backend == crate::runtime::state::ProviderLoginProcessBackend::CodexAppServer {
            cancel_codex_login(
                runtime_state,
                owner_user_id,
                &record.account_profile,
                &request.login_id,
            )
            .await?;
        } else {
            let _ = runtime_state
                .with_app_side_effect(|app| app.pty_mut().remove_process(&request.login_id))
                .await;
        }
        let login = runtime_state.provider_login_process_store().set_state(
            owner_user_id,
            &request.login_id,
            ProviderLoginProcessState::Failed,
            now_ms,
        )?;
        return Ok(LocalDaemonResponse::ProviderLoginStatus { login });
    }
    if record.backend == crate::runtime::state::ProviderLoginProcessBackend::CodexAppServer {
        let registry = runtime_state.provider_account_profile_registry().clone();
        let owner = owner_user_id.to_string();
        let profile_id = record.account_profile.clone();
        let refresh_result = tokio::task::spawn_blocking(move || {
            let refreshed =
                crate::local::provider_requests::refresh_provider_account_profile_response(
                    &registry,
                    &owner,
                    "codex",
                    &profile_id,
                )?;
            refresh_provider_account_family_after_login(
                &registry,
                &owner,
                "codex",
                &profile_id,
                refreshed,
                |candidate_profile_id| {
                    crate::local::provider_requests::refresh_provider_account_profile_response(
                        &registry,
                        &owner,
                        "codex",
                        candidate_profile_id,
                    )
                },
            )
        })
        .await
        .map_err(|error| provider_auth_task_error("refresh Codex login", error))?;
        let mut status = record.status();
        let authenticated = match refresh_result {
            Ok(profile) => {
                profile.auth_state
                    == crate::account_profile::ProviderAccountAuthState::Authenticated
            }
            Err(error) => {
                status = append_provider_login_refresh_failure(
                    runtime_state,
                    owner_user_id,
                    &request.login_id,
                    crate::runtime::state::ProviderAuthProcessOperation::Login,
                    &error,
                )?;
                let rejected = runtime_state
                    .provider_account_profile_registry()
                    .get(owner_user_id, "codex", &record.account_profile)
                    .is_ok_and(|profile| {
                        profile.auth_state
                            == crate::account_profile::ProviderAccountAuthState::Error
                    });
                if rejected {
                    status = runtime_state.provider_login_process_store().set_state(
                        owner_user_id,
                        &request.login_id,
                        ProviderLoginProcessState::Failed,
                        now_ms,
                    )?;
                }
                false
            }
        };
        let login = if authenticated {
            let login = runtime_state.provider_login_process_store().set_state(
                owner_user_id,
                &request.login_id,
                ProviderLoginProcessState::Succeeded,
                now_ms,
            )?;
            runtime_state
                .with_app_side_effect(|app| app.invalidate_provider_catalog_cache())
                .await;
            login
        } else {
            status
        };
        return Ok(LocalDaemonResponse::ProviderLoginStatus { login });
    }
    let (chunks, process_state) = runtime_state
        .with_app_side_effect(|app| {
            let chunks = app.pty_mut().drain_output(&request.login_id)?;
            let state = app.pty_mut().poll_process_state(&request.login_id)?;
            Ok::<_, DaemonError>((chunks, state))
        })
        .await?;
    let mut status = runtime_state.provider_login_process_store().append_output(
        owner_user_id,
        &request.login_id,
        chunks.into_iter().map(|chunk| chunk.bytes),
        crate::session::unix_epoch_ms(),
    )?;
    if process_state == PtyProcessState::Exited
        && status.state == ProviderLoginProcessState::Running
    {
        let registry = runtime_state.provider_account_profile_registry().clone();
        let owner = owner_user_id.to_string();
        let provider = record.provider.clone();
        let profile = record.account_profile.clone();
        let refresh_result = tokio::task::spawn_blocking(move || {
            let refreshed =
                crate::local::provider_requests::refresh_provider_account_profile_response(
                    &registry, &owner, &provider, &profile,
                )?;
            if record.operation == crate::runtime::state::ProviderAuthProcessOperation::Login
                && refreshed.auth_state
                    == crate::account_profile::ProviderAccountAuthState::Authenticated
            {
                refresh_provider_account_family_after_login(
                    &registry,
                    &owner,
                    &provider,
                    &profile,
                    refreshed,
                    |candidate_profile_id| {
                        crate::local::provider_requests::refresh_provider_account_profile_response(
                            &registry,
                            &owner,
                            &provider,
                            candidate_profile_id,
                        )
                    },
                )
            } else {
                Ok(refreshed)
            }
        })
        .await
        .map_err(|error| provider_auth_task_error("refresh provider login", error))?;
        let (authenticated, refresh_failed) = match refresh_result {
            Ok(profile) => (
                profile.auth_state
                    == crate::account_profile::ProviderAccountAuthState::Authenticated,
                false,
            ),
            Err(error) => {
                append_provider_login_refresh_failure(
                    runtime_state,
                    owner_user_id,
                    &request.login_id,
                    record.operation,
                    &error,
                )?;
                (false, true)
            }
        };
        let succeeded =
            provider_auth_process_succeeded(record.operation, authenticated, refresh_failed);
        if succeeded
            && record.operation == crate::runtime::state::ProviderAuthProcessOperation::Logout
        {
            crate::provider::invalidate_opencode_account_endpoint(
                owner_user_id,
                &record.account_profile,
            );
            let _ = runtime_state
                .provider_account_profile_registry()
                .mark_logged_out(owner_user_id, &record.provider, &record.account_profile);
        }
        status = runtime_state.provider_login_process_store().set_state(
            owner_user_id,
            &request.login_id,
            if succeeded {
                ProviderLoginProcessState::Succeeded
            } else {
                ProviderLoginProcessState::Failed
            },
            crate::session::unix_epoch_ms(),
        )?;
        if succeeded {
            runtime_state
                .with_app_side_effect(|app| app.invalidate_provider_catalog_cache())
                .await;
        }
        let _ = runtime_state
            .with_app_side_effect(|app| app.pty_mut().remove_process(&request.login_id))
            .await;
    }
    Ok(LocalDaemonResponse::ProviderLoginStatus { login: status })
}

fn provider_auth_process_succeeded(
    operation: crate::runtime::state::ProviderAuthProcessOperation,
    authenticated: bool,
    refresh_failed: bool,
) -> bool {
    if operation == crate::runtime::state::ProviderAuthProcessOperation::Logout {
        refresh_failed || !authenticated
    } else {
        !refresh_failed && authenticated
    }
}

fn refresh_provider_account_family_after_login<F>(
    registry: &crate::account_profile::ProviderAccountProfileRegistry,
    owner_user_id: &str,
    provider: &str,
    account_profile: &str,
    initial: crate::account_profile::ProviderAccountProfile,
    mut refresh: F,
) -> Result<crate::account_profile::ProviderAccountProfile, DaemonError>
where
    F: FnMut(&str) -> Result<crate::account_profile::ProviderAccountProfile, DaemonError>,
{
    if initial.auth_state != crate::account_profile::ProviderAccountAuthState::Authenticated {
        return Ok(initial);
    }
    let mut siblings = registry
        .list(owner_user_id, Some(provider))?
        .into_iter()
        .filter(|profile| profile.profile_id != account_profile)
        .collect::<Vec<_>>();
    siblings.sort_by_key(|profile| (!profile.is_default, profile.profile_id.clone()));
    for sibling in siblings {
        if let Err(error) = refresh(&sibling.profile_id) {
            crate::logging::warn_with_fields(
                "provider.account",
                "provider sibling account refresh failed after login",
                serde_json::json!({
                    "provider": provider,
                    "account_profile": sibling.profile_id,
                    "error": error.to_string(),
                }),
            );
        }
    }
    let reconciled = registry.get(owner_user_id, provider, account_profile)?;
    if reconciled.auth_state == crate::account_profile::ProviderAccountAuthState::Error {
        refresh(account_profile)
    } else {
        Ok(reconciled)
    }
}

fn append_provider_login_refresh_failure(
    runtime_state: &KernelRuntimeState,
    owner_user_id: &str,
    login_id: &str,
    operation: crate::runtime::state::ProviderAuthProcessOperation,
    error: &DaemonError,
) -> Result<crate::local::ProviderLoginStatus, DaemonError> {
    runtime_state.provider_login_process_store().append_output(
        owner_user_id,
        login_id,
        std::iter::once(provider_auth_refresh_failure_message(operation, error).into_bytes()),
        crate::session::unix_epoch_ms(),
    )
}

fn provider_auth_refresh_failure_message(
    operation: crate::runtime::state::ProviderAuthProcessOperation,
    error: &DaemonError,
) -> String {
    if operation == crate::runtime::state::ProviderAuthProcessOperation::Logout {
        format!("Provider logout completed; verification is temporarily unavailable: {error}\n")
    } else {
        format!("Provider account validation failed: {error}\n")
    }
}

pub(crate) async fn execute_send_provider_login_input_request(
    runtime_state: &KernelRuntimeState,
    owner_user_id: &str,
    request: SendProviderLoginInputRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let record = runtime_state
        .provider_login_process_store()
        .record_for_owner(owner_user_id, &request.login_id)?;
    if record.state != ProviderLoginProcessState::Running {
        return Err(provider_login_error("provider login is not running"));
    }
    if record.backend != crate::runtime::state::ProviderLoginProcessBackend::Terminal {
        return Err(provider_login_error(
            "Codex device login does not accept terminal input",
        ));
    }
    let data = base64::engine::general_purpose::STANDARD
        .decode(request.data_base64.as_bytes())
        .map_err(|_| provider_login_error("provider login input is not valid base64"))?;
    if data.len() > 8 * 1024 {
        return Err(provider_login_error("provider login input is too large"));
    }
    runtime_state
        .with_app_side_effect(|app| app.pty_mut().write_input(&request.login_id, &data))
        .await?;
    Ok(LocalDaemonResponse::ProviderLoginInputSent {
        login_id: request.login_id,
        byte_count: data.len(),
    })
}

async fn cancel_codex_login(
    runtime_state: &KernelRuntimeState,
    owner_user_id: &str,
    account_profile: &str,
    login_id: &str,
) -> Result<(), DaemonError> {
    let registry = runtime_state.provider_account_profile_registry().clone();
    let owner = owner_user_id.to_string();
    let profile_id = account_profile.to_string();
    let login_id = login_id.to_string();
    tokio::task::spawn_blocking(move || {
        let environment = registry.resolve_environment(&owner, "codex", &profile_id)?;
        let endpoint =
            crate::provider::ensure_codex_account_endpoint(&owner, &profile_id, environment)?;
        crate::provider::CodexClient::new("provider-login-cancel", endpoint)?
            .cancel_login(&login_id)
    })
    .await
    .map_err(|error| provider_auth_task_error("cancel Codex login", error))??;
    Ok(())
}

pub(crate) async fn execute_cancel_provider_login_request(
    runtime_state: &KernelRuntimeState,
    owner_user_id: &str,
    request: CancelProviderLoginRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let record = runtime_state
        .provider_login_process_store()
        .record_for_owner(owner_user_id, &request.login_id)?;
    if record.state != ProviderLoginProcessState::Running {
        if record.state == ProviderLoginProcessState::Cancelled {
            return Ok(LocalDaemonResponse::ProviderLoginCancelled {
                login: record.status(),
            });
        }
        return Err(provider_login_error(
            "provider authentication workflow is not running",
        ));
    }
    if record.backend == crate::runtime::state::ProviderLoginProcessBackend::CodexAppServer {
        cancel_codex_login(
            runtime_state,
            owner_user_id,
            &record.account_profile,
            &request.login_id,
        )
        .await?;
    } else {
        let _ = runtime_state
            .with_app_side_effect(|app| app.pty_mut().remove_process(&request.login_id))
            .await?;
    }
    let login = runtime_state.provider_login_process_store().set_state(
        owner_user_id,
        &request.login_id,
        ProviderLoginProcessState::Cancelled,
        crate::session::unix_epoch_ms(),
    )?;
    Ok(LocalDaemonResponse::ProviderLoginCancelled { login })
}

pub(crate) async fn execute_start_provider_login_request(
    runtime_state: &KernelRuntimeState,
    owner_user_id: &str,
    request: StartProviderLoginRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    crate::account_profile::validate_provider_enrollment_method(
        &request.provider,
        request.method.as_deref(),
    )?;
    if crate::provider::canonical_provider_family(&request.provider) != Some("codex") {
        return start_terminal_provider_auth(
            runtime_state,
            owner_user_id,
            request.provider,
            request.account_profile,
            crate::runtime::state::ProviderAuthProcessOperation::Login,
        )
        .await;
    }
    let registry = runtime_state.provider_account_profile_registry().clone();
    let owner = owner_user_id.to_string();
    let response = tokio::task::spawn_blocking(move || {
        start_provider_login_response(&registry, &owner, request)
    })
    .await
    .map_err(|error| provider_auth_task_error("start provider login", error))??;
    if let LocalDaemonResponse::ProviderLoginStarted { login } = &response {
        if let Some(login_id) = login.login_id.as_deref() {
            let now_ms = crate::session::unix_epoch_ms();
            runtime_state.provider_login_process_store().insert(
                crate::runtime::state::ProviderLoginProcessRecord {
                    owner_user_id: owner_user_id.to_string(),
                    provider: "codex".to_string(),
                    account_profile: login.account_profile.clone(),
                    login_id: login_id.to_string(),
                    state: ProviderLoginProcessState::Running,
                    backend: crate::runtime::state::ProviderLoginProcessBackend::CodexAppServer,
                    operation: crate::runtime::state::ProviderAuthProcessOperation::Login,
                    output: Vec::new(),
                    started_at_ms: now_ms,
                    updated_at_ms: now_ms,
                },
            )?;
        }
    }
    Ok(response)
}

pub(crate) async fn execute_logout_provider_request(
    runtime_state: &KernelRuntimeState,
    owner_user_id: &str,
    request: LogoutProviderRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let owner_user_id = runtime_state.provider_account_authority_owner_user_id(owner_user_id);
    let registry = runtime_state.provider_account_profile_registry().clone();
    crate::runtime::provider_account_control::ensure_profile_idle(
        runtime_state,
        &registry,
        &owner_user_id,
        &request.provider,
        &request.account_profile,
    )?;
    if crate::provider::canonical_provider_family(&request.provider) == Some("opencode") {
        return start_terminal_provider_auth(
            runtime_state,
            &owner_user_id,
            request.provider,
            request.account_profile,
            crate::runtime::state::ProviderAuthProcessOperation::Logout,
        )
        .await;
    }
    let response = tokio::task::spawn_blocking(move || {
        let provider = request.provider.clone();
        let account_profile = request.account_profile.clone();
        let response = logout_provider_response(&registry, &owner_user_id, request)?;
        registry.mark_logged_out(&owner_user_id, &provider, &account_profile)?;
        Ok(response)
    })
    .await
    .map_err(|error| provider_auth_task_error("logout provider", error))??;
    runtime_state
        .with_app_side_effect(|app| app.invalidate_provider_catalog_cache())
        .await;
    Ok(response)
}

fn provider_auth_task_error(operation: &'static str, error: tokio::task::JoinError) -> DaemonError {
    DaemonError::LocalTransport {
        operation,
        message: error.to_string(),
    }
}

fn provider_login_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "provider login",
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        provider_auth_process_succeeded, provider_auth_refresh_failure_message,
        refresh_provider_account_family_after_login,
    };
    use crate::account_profile::{ProviderAccountAuthState, ProviderAccountProfileRegistry};
    use rand::Rng as _;

    fn fixture() -> (std::path::PathBuf, ProviderAccountProfileRegistry) {
        let root = std::env::temp_dir().join(format!(
            "chariox-provider-login-refresh-test-{}-{}",
            std::process::id(),
            rand::thread_rng().gen::<u64>(),
        ));
        std::fs::create_dir_all(&root).expect("fixture root should create");
        let registry = ProviderAccountProfileRegistry::open(root.join("accounts.json"))
            .expect("registry should open");
        registry
            .migrate_effective_defaults("owner-a", &root.join("home"))
            .expect("defaults should migrate");
        (root, registry)
    }

    #[test]
    fn login_completion_refreshes_siblings_and_rejects_a_duplicate_identity() {
        let (root, registry) = fixture();
        let secondary = registry
            .create_managed("owner-a", "claude", "Claude secondary")
            .expect("secondary profile should create");
        let initial = registry
            .update_observation(
                "owner-a",
                "claude",
                &secondary.profile_id,
                ProviderAccountAuthState::Authenticated,
                Some("same@example.test".to_string()),
                Some("pro".to_string()),
                None,
                None,
            )
            .expect("secondary login should initially authenticate");

        let mut refreshed_profiles = Vec::new();
        let result = refresh_provider_account_family_after_login(
            &registry,
            "owner-a",
            "claude",
            &secondary.profile_id,
            initial,
            |profile_id| {
                refreshed_profiles.push(profile_id.to_string());
                registry.update_observation(
                    "owner-a",
                    "claude",
                    profile_id,
                    ProviderAccountAuthState::Authenticated,
                    Some("same@example.test".to_string()),
                    Some("pro".to_string()),
                    None,
                    None,
                )
            },
        );

        assert!(
            result
                .expect_err("duplicate login should be rejected")
                .to_string()
                .contains("already authenticated as `Default`")
        );
        assert_eq!(
            registry
                .get("owner-a", "claude", "default")
                .expect("default profile should remain")
                .auth_state,
            ProviderAccountAuthState::Authenticated,
        );
        assert_eq!(
            registry
                .get("owner-a", "claude", &secondary.profile_id)
                .expect("secondary profile should remain")
                .auth_state,
            ProviderAccountAuthState::Error,
        );
        assert_eq!(
            refreshed_profiles,
            vec!["default".to_string(), secondary.profile_id.clone()],
            "the completed profile should only refresh again to surface its collision",
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn login_completion_keeps_distinct_sibling_identities_authenticated() {
        let (root, registry) = fixture();
        let secondary = registry
            .create_managed("owner-a", "codex", "Codex secondary")
            .expect("secondary profile should create");
        let initial = registry
            .update_observation(
                "owner-a",
                "codex",
                &secondary.profile_id,
                ProviderAccountAuthState::Authenticated,
                Some(format!("{}@example.test", secondary.profile_id)),
                None,
                None,
                None,
            )
            .expect("secondary login should initially authenticate");

        let mut refreshed_profiles = Vec::new();
        let result = refresh_provider_account_family_after_login(
            &registry,
            "owner-a",
            "codex",
            &secondary.profile_id,
            initial,
            |profile_id| {
                refreshed_profiles.push(profile_id.to_string());
                registry.update_observation(
                    "owner-a",
                    "codex",
                    profile_id,
                    ProviderAccountAuthState::Authenticated,
                    Some(format!("{profile_id}@example.test")),
                    None,
                    None,
                    None,
                )
            },
        )
        .expect("distinct account should authenticate");

        assert_eq!(result.profile_id, secondary.profile_id);
        assert_eq!(result.auth_state, ProviderAccountAuthState::Authenticated);
        assert_eq!(
            registry
                .get("owner-a", "codex", "default")
                .expect("default profile should remain")
                .auth_state,
            ProviderAccountAuthState::Authenticated,
        );
        assert_eq!(
            refreshed_profiles,
            vec!["default".to_string()],
            "a distinct completed profile must not be refreshed a second time",
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn completed_logout_survives_a_transient_post_logout_refresh_failure() {
        use crate::runtime::state::ProviderAuthProcessOperation;

        assert!(provider_auth_process_succeeded(
            ProviderAuthProcessOperation::Logout,
            false,
            true,
        ));
        assert!(!provider_auth_process_succeeded(
            ProviderAuthProcessOperation::Login,
            false,
            true,
        ));
        let message = provider_auth_refresh_failure_message(
            ProviderAuthProcessOperation::Logout,
            &super::provider_login_error("status probe unavailable"),
        );
        assert!(
            message
                .starts_with("Provider logout completed; verification is temporarily unavailable:")
        );
        assert!(!message.contains("validation failed"));
    }
}
