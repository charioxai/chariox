//! MP-08 / MP-11: kernel-owned provider credential portability, separate from
//! human Cloud mutations. Preflight reads only existing provider-native exports
//! and drops their bytes; callers receive an empty acknowledgement.
use crate::account_profile::{
    provider_account_authority_owner_user_id, ProviderAccountContextTransfer,
    ProviderAccountProfile, ProviderAccountProfileRegistry,
};
use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::local::{
    LocalDaemonResponse, PreflightProviderAccountPortabilityRequest,
    ProviderAccountTransferSelection,
};
use crate::runtime::command::KernelCommand;

pub(crate) async fn execute_preflight(
    runtime_state: &crate::runtime::state::KernelRuntimeState,
    config: &DaemonConfig,
    profiles: &ProviderAccountProfileRegistry,
    command: &KernelCommand,
    request: &PreflightProviderAccountPortabilityRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    if check_preflight(config, profiles, command, request)? {
        // A locked Vault shows its unlock card in the launching session and the launch goes on
        // when it opens. A caller without a session gets the typed `vault_locked` refusal.
        // The unlock stays open for the export that follows the machine's creation.
        let unlocked = runtime_state
            .ensure_vault_unlocked_for_command_context(
                command,
                None,
                None,
                "provider_account_portability",
            )
            .await?;
        runtime_state.hold_vault_for_login_export(unlocked, command, request.launch_id.as_deref());
    }
    Ok(LocalDaemonResponse::ProviderAccountPortabilityPreflightPassed {})
}

/// The owner's check of the selection; `true` when a Vault login is selected, which must be
/// found unlocked before the launch goes on.
fn check_preflight(
    config: &DaemonConfig,
    profiles: &ProviderAccountProfileRegistry,
    command: &KernelCommand,
    request: &PreflightProviderAccountPortabilityRequest,
) -> Result<bool, DaemonError> {
    if !crate::runtime::cloud_relay_authorization::kernel_cloud_owner(config, command) {
        return Err(error(
            "only this kernel's owner can preflight provider accounts",
        ));
    }
    let owner = config
        .cloud_relay
        .as_ref()
        .map(|cloud| cloud.user_id.as_str())
        .unwrap_or(crate::session::DEFAULT_LOCAL_USER_ID);
    preflight_selection(config, profiles, owner, &request.provider_accounts)
}

/// Whether every selected login can move. Answers `true` when one of them is a Vault credential,
/// which the caller must find unlocked before the launch goes on.
pub(super) fn preflight_selection(
    config: &DaemonConfig,
    profiles: &ProviderAccountProfileRegistry,
    runtime_owner: &str,
    selection: &ProviderAccountTransferSelection,
) -> Result<bool, DaemonError> {
    let ProviderAccountTransferSelection::Selected { accounts } = selection else {
        return Ok(false);
    };
    let owner = provider_account_authority_owner_user_id(config, runtime_owner);
    let mut carries_vault_login = false;
    for account in accounts {
        let unavailable = || {
            error(format!(
                "selected {} provider account `{}` has no transferable credentials",
                account.provider, account.account_profile
            ))
        };
        if profiles
            .login_is_vault_credential(&owner, &account.provider, &account.account_profile)
            .map_err(|_| unavailable())?
        {
            if !vault_can_be_sealed(&config.user_config.credential_vault) {
                return Err(unavailable());
            }
            carries_vault_login = true;
            continue;
        }
        profiles
            .export_managed_context_materialization(
                &owner,
                &account.provider,
                &account.account_profile,
            )
            .map_err(|_| unavailable())?;
    }
    Ok(carries_vault_login)
}

/// Only a Chariox Vault can be sealed for a new machine's own Vault (local 973).
fn vault_can_be_sealed(vault: &crate::config::UserCredentialVaultConfig) -> bool {
    vault.backend == crate::config::CredentialVaultBackend::CharioxEncrypted
}

/// Tells readers which logins a managed machine's context transfer cannot
/// carry, so a client never selects one by default.
pub(crate) fn mark_context_transfer(
    owner: &str,
    vault: &crate::config::UserCredentialVaultConfig,
    profiles: &mut [ProviderAccountProfile],
) -> Result<(), DaemonError> {
    for profile in profiles {
        if !vault_can_be_sealed(vault)
            && crate::provider::provider_account_credential_uses_vault(
                owner,
                &profile.provider,
                &profile.profile_id,
            )?
        {
            profile.context_transfer = ProviderAccountContextTransfer::Unavailable;
        }
    }
    Ok(())
}

fn error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "provider account portability preflight",
        message: message.into(),
    }
}

#[cfg(test)]
mod tests;
