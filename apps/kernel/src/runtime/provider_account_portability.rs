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
    LocalDaemonResponse, ManagedEnvironmentProviderAccounts,
    PreflightProviderAccountPortabilityRequest,
};
use crate::runtime::command::KernelCommand;

pub(crate) fn execute_preflight(
    config: &DaemonConfig,
    profiles: &ProviderAccountProfileRegistry,
    command: &KernelCommand,
    request: &PreflightProviderAccountPortabilityRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
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
    preflight_selection(config, profiles, owner, &request.provider_accounts)?;
    Ok(LocalDaemonResponse::ProviderAccountPortabilityPreflightPassed {})
}

pub(super) fn preflight_selection(
    config: &DaemonConfig,
    profiles: &ProviderAccountProfileRegistry,
    runtime_owner: &str,
    selection: &ManagedEnvironmentProviderAccounts,
) -> Result<(), DaemonError> {
    let ManagedEnvironmentProviderAccounts::Selected { accounts } = selection else {
        return Ok(());
    };
    let owner = provider_account_authority_owner_user_id(config, runtime_owner);
    for account in accounts {
        profiles
            .export_managed_context_materialization(
                &owner,
                &account.provider,
                &account.account_profile,
            )
            .map_err(|_| {
                error(format!(
                    "selected {} provider account `{}` has no transferable credentials",
                    account.provider, account.account_profile
                ))
            })?;
    }
    Ok(())
}

/// Tells readers which logins a managed machine's context transfer cannot
/// carry, so a client never selects one by default.
pub(crate) fn mark_context_transfer(
    owner: &str,
    profiles: &mut [ProviderAccountProfile],
) -> Result<(), DaemonError> {
    for profile in profiles {
        if crate::provider::provider_account_credential_uses_vault(
            owner,
            &profile.provider,
            &profile.profile_id,
        )? {
            profile.context_transfer = ProviderAccountContextTransfer::VaultCredential;
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
