//! Completion of the `claude setup-token` provider login: token capture from
//! the rendered screen, verification, Chariox Vault storage, and the vault
//! passphrase exchange held inside the login workflow itself.

use zeroize::Zeroizing;

use crate::error::DaemonError;
use crate::local::{ProviderLoginProcessState, ProviderLoginStatus};
use crate::runtime::state::{
    ClaudeSetupTokenStoreOutcome, ClaudeSetupTokenVaultPrompt, KernelRuntimeState,
    ProviderLoginProcessRecord, SetupTokenScan,
};

pub(crate) const CLAUDE_SETUP_TOKEN_METHOD: &str = "setup_token";

/// Finishes the login once the CLI printed a complete token or exited. Callers
/// hold the login's serialization guard.
pub(super) async fn reconcile(
    runtime_state: &KernelRuntimeState,
    owner_user_id: &str,
    record: &ProviderLoginProcessRecord,
    exited: bool,
    status: ProviderLoginStatus,
) -> Result<ProviderLoginStatus, DaemonError> {
    let store = runtime_state.provider_login_process_store();
    if !exited
        && store.capture_setup_token(owner_user_id, &record.login_id, false)?
            == SetupTokenScan::Pending
    {
        return Ok(status);
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
    if let Err(error) = verify(runtime_state, owner_user_id, record, &token).await {
        return fail(
            runtime_state,
            owner_user_id,
            record,
            &format!("Claude did not accept the setup token; nothing was stored: {error}"),
        );
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

async fn verify(
    runtime_state: &KernelRuntimeState,
    owner_user_id: &str,
    record: &ProviderLoginProcessRecord,
    token: &Zeroizing<String>,
) -> Result<(), DaemonError> {
    let registry = runtime_state.provider_account_profile_registry().clone();
    let owner_user_id = owner_user_id.to_string();
    let account_profile = record.account_profile.clone();
    let token = token.clone();
    tokio::task::spawn_blocking(move || {
        let mut environment =
            registry.resolve_environment(&owner_user_id, "claude", &account_profile)?;
        environment.insert(
            crate::provider::CLAUDE_OAUTH_TOKEN_ENV.to_string(),
            token.to_string(),
        );
        let executable = crate::provider::resolve_claude_executable()?;
        let verified = crate::provider::probe_claude_account_usage(
            &executable,
            &account_profile,
            &environment,
        );
        if let Some(value) = environment.get_mut(crate::provider::CLAUDE_OAUTH_TOKEN_ENV) {
            zeroize::Zeroize::zeroize(value);
        }
        verified.map(|_| ())
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "verify Claude setup token",
        message: error.to_string(),
    })?
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
        .store_claude_setup_token(owner_user_id, &record.account_profile, token, passphrase)
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
            if let Err(error) = runtime_state
                .provider_account_profile_registry()
                .update_observation(
                    owner_user_id,
                    "claude",
                    &record.account_profile,
                    crate::account_profile::ProviderAccountAuthState::Authenticated,
                    None,
                    None,
                    None,
                    None,
                )
            {
                crate::logging::warn_with_fields(
                    "provider.account",
                    "recording a verified Claude setup token failed",
                    serde_json::json!({
                        "account_profile": record.account_profile,
                        "login_id": record.login_id,
                        "error": error.to_string(),
                    }),
                );
            }
            let login = finish(runtime_state, owner_user_id, record, ProviderLoginProcessState::Succeeded)?;
            runtime_state
                .with_app_side_effect(|app| app.invalidate_provider_catalog_cache())
                .await;
            runtime_state.record_waiting_room_change();
            Ok(login)
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
