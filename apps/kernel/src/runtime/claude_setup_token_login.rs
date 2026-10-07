//! Completion of the `claude setup-token` provider login: token capture from
//! the rendered screen, verification, Chariox Vault storage, and the vault
//! passphrase exchange held inside the login workflow itself.

use zeroize::Zeroizing;

use super::command::{command_caller_user_id, KernelCommand};
use crate::error::DaemonError;
use crate::local::{
    LocalDaemonResponse, ProviderLoginProcessState, ProviderLoginStatus,
    SetProviderAccountCredentialRequest, StartProviderLoginRequest,
};
use crate::provider::ClaudeCredentialCheckError;
use crate::runtime::state::{
    ClaudeSetupTokenStoreOutcome, ClaudeSetupTokenVaultPrompt, KernelRuntimeState,
    ProviderLoginProcessRecord, SetupTokenScan,
};

pub(crate) const CLAUDE_SETUP_TOKEN_METHOD: &str = "setup_token";

/// MP-08/MP-11: credential creation is an adapter for StartProviderLogin,
/// never a second PTY capture or Vault authority path.
pub(super) async fn start(
    runtime_state: &KernelRuntimeState,
    command: &KernelCommand,
    request: SetProviderAccountCredentialRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    if request.provider != "claude" || !request.value.is_empty() {
        return Err(login_error(
            "setup-token --run requires Claude and no credential value",
        ));
    }
    let owner =
        runtime_state.provider_account_authority_owner_user_id(&command_caller_user_id(command));
    let profile = runtime_state.provider_account_profile_registry().get(
        &owner,
        "claude",
        &request.account_profile,
    )?;
    if !request.overwrite {
        let credential_id =
            crate::provider::provider_account_credential_id(&owner, "claude", &profile.profile_id);
        let exists = tokio::task::spawn_blocking(move || {
            crate::credential::CharioxCredentialRegistry::user()?
                .get(&credential_id)
                .map(|value| value.is_some())
        })
        .await
        .map_err(|error| login_error(&format!("check setup token replacement: {error}")))??;
        if exists {
            return Err(login_error("A setup token already exists for this profile; use --replace to authorize replacement"));
        }
    }
    super::provider_auth_control::execute_start_provider_login_with_overwrite(
        runtime_state,
        &owner,
        StartProviderLoginRequest {
            provider: "claude".into(),
            account_profile: profile.profile_id,
            method: Some(CLAUDE_SETUP_TOKEN_METHOD.into()),
        },
        request.overwrite,
    )
    .await
}

/// Finishes the login after a successful CLI exit and the reader's final
/// rendered screen. Callers hold the login's serialization guard.
pub(super) async fn reconcile(
    runtime_state: &KernelRuntimeState,
    owner_user_id: &str,
    record: &ProviderLoginProcessRecord,
    process_state: &crate::pty::PtyProcessState,
    status: ProviderLoginStatus,
) -> Result<ProviderLoginStatus, DaemonError> {
    let store = runtime_state.provider_login_process_store();
    // Wait for the reader's final screen and a successful CLI exit. Capturing
    // an earlier row could miss a second token or a later provider failure.
    if !process_state.is_exited()
        || !record
            .setup_token
            .as_ref()
            .is_some_and(|login| login.reader_finished())
    {
        return Ok(status);
    }
    if !matches!(
        process_state,
        crate::pty::PtyProcessState::Exited {
            exit_code: Some(0),
            signal: None
        }
    ) {
        let _ = runtime_state
            .with_app_side_effect(|app| app.pty_mut().remove_process(&record.login_id))
            .await;
        return fail(
            runtime_state,
            owner_user_id,
            record,
            "Claude setup-token did not exit successfully; nothing was stored.",
        );
    }
    let remaining = runtime_state
        .with_app_side_effect(|app| app.pty_mut().drain_output(&record.login_id))
        .await
        .unwrap_or_default();
    store.append_output(
        owner_user_id,
        &record.login_id,
        remaining.into_iter().map(|chunk| chunk.bytes),
        crate::session::unix_epoch_ms(),
    )?;
    let _ = runtime_state
        .with_app_side_effect(|app| app.pty_mut().remove_process(&record.login_id))
        .await;
    let token = match store.capture_setup_token(owner_user_id, &record.login_id, true)? {
        SetupTokenScan::Found(token) => token,
        SetupTokenScan::Pending => {
            return fail(
                runtime_state,
                owner_user_id,
                record,
                "Claude setup-token finished without printing a Claude OAuth token; nothing was stored.",
            );
        }
        SetupTokenScan::Ambiguous => {
            return fail(
                runtime_state,
                owner_user_id,
                record,
                "Claude setup-token printed more than one token; nothing was stored.",
            );
        }
    };
    store.append_setup_token_note(
        owner_user_id,
        &record.login_id,
        "Verifying the setup token with Claude…",
        crate::session::unix_epoch_ms(),
    )?;
    if let Err(message) = verify(
        runtime_state,
        owner_user_id,
        &record.account_profile,
        &token,
    )
    .await
    {
        return fail(runtime_state, owner_user_id, record, &message);
    }
    store_token(runtime_state, owner_user_id, record, token, None).await
}

/// Consumes one secret workflow input as the Chariox Vault passphrase. A new
/// vault's passphrase must be entered twice before the vault is created.
pub(super) async fn submit_vault_passphrase(
    runtime_state: &KernelRuntimeState,
    owner_user_id: &str,
    record: &ProviderLoginProcessRecord,
    input: &[u8],
) -> Result<ProviderLoginStatus, DaemonError> {
    let store = runtime_state.provider_login_process_store();
    let passphrase = Zeroizing::new(
        std::str::from_utf8(input)
            .map_err(|_| login_error("Chariox Vault passphrase must be UTF-8"))?
            .trim_end_matches(['\r', '\n'])
            .to_string(),
    );
    let now_ms = crate::session::unix_epoch_ms();
    match record
        .setup_token
        .as_ref()
        .and_then(|login| login.vault_prompt)
    {
        Some(ClaudeSetupTokenVaultPrompt::Create) => {
            store.exchange_new_vault_passphrase(
                owner_user_id,
                &record.login_id,
                Some(passphrase),
            )?;
            return store.await_vault_passphrase(
                owner_user_id,
                &record.login_id,
                ClaudeSetupTokenVaultPrompt::ConfirmCreate,
                "Enter the new Chariox Vault passphrase again to confirm it.",
                now_ms,
            );
        }
        Some(ClaudeSetupTokenVaultPrompt::ConfirmCreate) => {
            let first =
                store.exchange_new_vault_passphrase(owner_user_id, &record.login_id, None)?;
            if first.as_deref().map(String::as_str) != Some(passphrase.as_str()) {
                return store.await_vault_passphrase(
                    owner_user_id,
                    &record.login_id,
                    ClaudeSetupTokenVaultPrompt::Create,
                    "The passphrases did not match. Choose a Chariox Vault passphrase again.",
                    now_ms,
                );
            }
        }
        Some(ClaudeSetupTokenVaultPrompt::Unlock) => {}
        None => {
            return Err(login_error(
                "the login is not waiting for a vault passphrase",
            ))
        }
    }
    let token = store
        .setup_token_secret(owner_user_id, &record.login_id)?
        .ok_or_else(|| login_error("the Claude setup token is no longer available"))?;
    store_token(
        runtime_state,
        owner_user_id,
        record,
        token,
        Some(passphrase),
    )
    .await
}

/// Checks a setup token with Claude before it is stored. The error is the
/// user-facing reason; nothing is stored when it fails.
pub(super) async fn verify(
    runtime_state: &KernelRuntimeState,
    owner_user_id: &str,
    account_profile: &str,
    token: &Zeroizing<String>,
) -> Result<(), String> {
    let registry = runtime_state.provider_account_profile_registry().clone();
    let owner_user_id = owner_user_id.to_string();
    let account_profile = account_profile.to_string();
    let token = token.clone();
    let checked = tokio::task::spawn_blocking(move || {
        let mut environment = registry
            .resolve_environment(&owner_user_id, "claude", &account_profile)
            .map_err(|error| ClaudeCredentialCheckError::Inconclusive(error.to_string()))?;
        let executable = crate::provider::resolve_claude_executable()
            .map_err(|error| ClaudeCredentialCheckError::Inconclusive(error.to_string()))?;
        environment.insert(
            crate::provider::CLAUDE_OAUTH_TOKEN_ENV.to_string(),
            token.to_string(),
        );
        let checked = crate::provider::verify_claude_account_credential(&executable, &environment);
        if let Some(value) = environment.get_mut(crate::provider::CLAUDE_OAUTH_TOKEN_ENV) {
            zeroize::Zeroize::zeroize(value);
        }
        checked
    })
    .await
    .unwrap_or_else(|error| Err(ClaudeCredentialCheckError::Inconclusive(error.to_string())));
    match checked {
        Ok(()) => Ok(()),
        Err(ClaudeCredentialCheckError::Rejected) => Err(
            "Claude rejected the setup token: it is invalid, expired or revoked. Nothing was stored. Create a new token with `claude setup-token` and store it again.".to_string(),
        ),
        Err(ClaudeCredentialCheckError::Inconclusive(reason)) => Err(format!(
            "Claude could not verify the setup token ({reason}). Nothing was stored. Check the network and the Claude CLI, then try again."
        )),
    }
}

/// Records a verified, stored setup token: the account can run agents.
pub(super) async fn record_verified_token(
    runtime_state: &KernelRuntimeState,
    owner_user_id: &str,
    account_profile: &str,
) -> Result<(), DaemonError> {
    runtime_state
        .provider_account_profile_registry()
        .update_observation(
            owner_user_id,
            "claude",
            account_profile,
            crate::account_profile::ProviderAccountAuthState::Authenticated,
            None,
            None,
            None,
            None,
        )?;
    runtime_state
        .with_app_side_effect(|app| app.invalidate_provider_catalog_cache())
        .await;
    runtime_state.record_waiting_room_change();
    Ok(())
}

async fn store_token(
    runtime_state: &KernelRuntimeState,
    owner_user_id: &str,
    record: &ProviderLoginProcessRecord,
    token: Zeroizing<String>,
    passphrase: Option<Zeroizing<String>>,
) -> Result<ProviderLoginStatus, DaemonError> {
    let store = runtime_state.provider_login_process_store();
    let outcome = runtime_state
        .store_claude_setup_token(
            owner_user_id,
            &record.account_profile,
            token,
            record
                .setup_token
                .as_ref()
                .is_some_and(|login| login.overwrite),
            passphrase,
        )
        .await;
    let now_ms = crate::session::unix_epoch_ms();
    match outcome {
        Ok(ClaudeSetupTokenStoreOutcome::VaultPassphraseRequired(prompt)) => {
            store.await_vault_passphrase(
                owner_user_id,
                &record.login_id,
                prompt,
                match prompt {
                    ClaudeSetupTokenVaultPrompt::Unlock => {
                        "Enter your Chariox Vault passphrase to store the token."
                    }
                    _ => "Choose a Chariox Vault passphrase. The vault is created with it and stores the token.",
                },
                now_ms,
            )
        }
        Ok(ClaudeSetupTokenStoreOutcome::VaultUnlockFailed(error)) => {
            store.append_setup_token_note(
                owner_user_id,
                &record.login_id,
                &format!("Chariox Vault unlock failed: {error}. Enter the passphrase again."),
                now_ms,
            )
        }
        Err(error) => fail(
            runtime_state,
            owner_user_id,
            record,
            &format!("Storing the Claude setup token failed: {error}"),
        ),
        Ok(ClaudeSetupTokenStoreOutcome::Stored) => {
            store.append_setup_token_note(
                owner_user_id,
                &record.login_id,
                "Claude setup token verified and stored in the Chariox Vault. Unattended agents can now use this account.",
                now_ms,
            )?;
            record_verified_token(runtime_state, owner_user_id, &record.account_profile).await?;
            finish(runtime_state, owner_user_id, record, ProviderLoginProcessState::Succeeded)
        }
    }
}

fn fail(
    runtime_state: &KernelRuntimeState,
    owner_user_id: &str,
    record: &ProviderLoginProcessRecord,
    message: &str,
) -> Result<ProviderLoginStatus, DaemonError> {
    runtime_state
        .provider_login_process_store()
        .append_setup_token_note(
            owner_user_id,
            &record.login_id,
            message,
            crate::session::unix_epoch_ms(),
        )?;
    finish(
        runtime_state,
        owner_user_id,
        record,
        ProviderLoginProcessState::Failed,
    )
}

fn finish(
    runtime_state: &KernelRuntimeState,
    owner_user_id: &str,
    record: &ProviderLoginProcessRecord,
    state: ProviderLoginProcessState,
) -> Result<ProviderLoginStatus, DaemonError> {
    let store = runtime_state.provider_login_process_store();
    match store.finish_if_running(
        owner_user_id,
        &record.login_id,
        state,
        crate::session::unix_epoch_ms(),
    )? {
        Some(status) => Ok(status),
        None => Ok(store
            .record_for_owner(owner_user_id, &record.login_id)?
            .status()),
    }
}

fn login_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "provider login",
        message: message.to_string(),
    }
}
