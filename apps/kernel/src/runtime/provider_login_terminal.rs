//! Official terminal authentication process launch and rejected-code retry.
use super::provider_auth_control::provider_login_error;
use crate::error::DaemonError;
use crate::local::{ProviderLoginProcessState, ProviderLoginStatus};
use crate::pty::PtySpawnRequest;
use crate::runtime::state::{
    KernelRuntimeState, ProviderAuthProcessOperation, ProviderLoginProcessRecord,
};
use base64::Engine as _;

pub(super) fn launch(
    provider: &str,
    operation: crate::runtime::state::ProviderAuthProcessOperation,
    method: Option<&str>,
    environment: std::collections::BTreeMap<String, String>,
) -> Result<crate::provider::ProviderLaunchResult, DaemonError> {
    let (program, args) = match (provider, operation) {
        ("claude", crate::runtime::state::ProviderAuthProcessOperation::Login) => (
            crate::provider::resolve_claude_executable()?,
            super::provider_auth_control::terminal_provider_auth_args(provider, operation, method),
        ),
        ("opencode", crate::runtime::state::ProviderAuthProcessOperation::Login) => (
            crate::provider::resolve_opencode_executable()?,
            super::provider_auth_control::terminal_provider_auth_args(provider, operation, method),
        ),
        ("opencode", crate::runtime::state::ProviderAuthProcessOperation::Logout) => (
            crate::provider::resolve_opencode_executable()?,
            super::provider_auth_control::terminal_provider_auth_args(provider, operation, None),
        ),
        _ => unreachable!(),
    };
    let auth_label = match operation {
        crate::runtime::state::ProviderAuthProcessOperation::Login => {
            format!("{provider}:auth-login")
        }
        crate::runtime::state::ProviderAuthProcessOperation::Logout => {
            format!("{provider}:auth-logout")
        }
    };
    let launch = crate::provider::managed_isolated_utility_launch(
        program.to_string_lossy().to_string(),
        args,
        environment,
        None,
        &auth_label,
    )?;
    Ok(launch)
}

pub(super) async fn spawn(
    runtime_state: &KernelRuntimeState,
    record: &crate::runtime::state::ProviderLoginProcessRecord,
    launch: crate::provider::ProviderLaunchResult,
) -> Result<(), DaemonError> {
    let mut env_remove = launch.pty_env_remove;
    env_remove.extend(
        crate::account_profile::provider_auth_env_vars(&record.provider)
            .iter()
            .map(|name| (*name).to_string()),
    );
    env_remove.sort();
    env_remove.dedup();
    let program = launch
        .pty_program
        .ok_or_else(|| provider_login_error("provider login launch has no executable"))?;
    let setup_login = record.setup_token.clone();
    runtime_state
        .with_app_side_effect(|app| {
            app.pty_mut()
                .spawn_with_output_filter(
                    PtySpawnRequest {
                        agent: None,
                        process_key: record.login_id.clone(),
                        provider_run_id: record.login_id.clone(),
                        program,
                        args: launch.pty_args,
                        env: launch.pty_env,
                        env_remove,
                        working_directory: launch.working_directory,
                        cols: if setup_login.is_some() {
                            crate::runtime::state::CLAUDE_SETUP_TOKEN_COLUMNS
                        } else {
                            120
                        },
                        rows: if setup_login.is_some() {
                            crate::runtime::state::CLAUDE_SETUP_TOKEN_ROWS
                        } else {
                            40
                        },
                    },
                    &crate::provider::ProviderCredentialEnvironment::default(),
                    true,
                    setup_login.map(|login| {
                        Box::new(move |bytes: &[u8]| login.read_private_output(bytes))
                            as Box<dyn FnMut(&[u8]) -> Vec<u8> + Send>
                    }),
                )
                .map(|_| ())
        })
        .await
}

pub(super) async fn reconcile_failed_claude_login(
    runtime: &KernelRuntimeState,
    owner: &str,
    record: &ProviderLoginProcessRecord,
    process: &crate::pty::PtyProcessState,
    status: &ProviderLoginStatus,
) -> Result<Option<ProviderLoginStatus>, DaemonError> {
    if record.provider != "claude"
        || record.operation != ProviderAuthProcessOperation::Login
        || record.setup_token.is_some()
        || !matches!(process, crate::pty::PtyProcessState::Exited { exit_code, .. } if *exit_code != Some(0))
    {
        return Ok(None);
    }
    let output = base64::engine::general_purpose::STANDARD
        .decode(&status.terminal_output_base64)
        .expect("login status encodes valid base64");
    let rejected_code = record.code_submitted
        && String::from_utf8_lossy(&output)
            .contains("Login failed: Request failed with status code 400");
    let store = runtime.provider_login_process_store();
    runtime
        .with_app_side_effect(|app| app.pty_mut().remove_process(&record.login_id))
        .await?;
    if rejected_code {
        let retry =
            store.prepare_code_retry(owner, &record.login_id, crate::session::unix_epoch_ms())?;
        let restarted: Result<(), DaemonError> = async {
            let registry = runtime.provider_account_profile_registry();
            let environment =
                registry.resolve_environment(owner, "claude", &record.account_profile)?;
            let scope = super::provider_auth_control::provider_auth_credential_scope(
                "claude",
                &record.account_profile,
                environment.get("CLAUDE_CONFIG_DIR").map(String::as_str),
            );
            if scope != record.credential_scope {
                return Err(provider_login_error(
                    "The account location changed during sign-in. Cancel and start again.",
                ));
            }
            let command = launch("claude", record.operation, Some("terminal"), environment)?;
            spawn(runtime, &retry, command).await
        }
        .await;
        if let Err(error) = restarted {
            crate::logging::warn_with_fields(
                "provider.account",
                "Claude authorization retry could not start",
                serde_json::json!({"login_id": record.login_id, "error": error.to_string()}),
            );
            store.set_message(
                owner,
                &record.login_id,
                "Could not restart sign-in. Cancel this attempt and try again.",
                crate::session::unix_epoch_ms(),
            )?;
            store.set_state(
                owner,
                &record.login_id,
                ProviderLoginProcessState::Failed,
                crate::session::unix_epoch_ms(),
            )?;
        }
    } else {
        store.set_message(
            owner,
            &record.login_id,
            "Sign-in could not finish. Please try again.",
            crate::session::unix_epoch_ms(),
        )?;
        store.set_state(
            owner,
            &record.login_id,
            ProviderLoginProcessState::Failed,
            crate::session::unix_epoch_ms(),
        )?;
    }
    Ok(Some(
        store.record_for_owner(owner, &record.login_id)?.status(),
    ))
}
