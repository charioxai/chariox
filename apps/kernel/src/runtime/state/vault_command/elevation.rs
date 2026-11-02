//! The one gate for Vault capture and use. Today a live sudo window elevates;
//! the project mandate (VAUDIT S1) joins it here as mandate-or-sudo.
use super::*;

#[derive(Clone, Copy)]
pub(super) enum VaultElevationKind {
    Capture,
    Use,
}

pub(super) struct VaultElevation {
    pub(super) turn: KernelSudoTurn,
    pub(super) run: crate::provider::RuntimeProviderRun,
}

impl KernelRuntimeState {
    pub(super) fn vault_elevation_allows(
        &self,
        token: &str,
        kind: VaultElevationKind,
    ) -> Result<VaultElevation, DaemonError> {
        let turn = self.sudo_for_auth_token(token).map_err(|_| {
            error(match kind {
                VaultElevationKind::Capture => "Vault capture requires a live sudo window",
                VaultElevationKind::Use => "Vault use requires a live sudo window",
            })
        })?;
        // An enrolled kernel's owner acts as its Cloud user; map it as MD-5 does.
        if self.provider_account_authority_owner_user_id(&turn.owner_user_id)
            != crate::session::DEFAULT_LOCAL_USER_ID
            || self
                .owned
                .config_projection
                .snapshot()
                .user_config
                .credential_vault
                .agent_management
                == crate::config::CredentialVaultAgentManagementPolicy::Deny
        {
            return Err(error("the owner's Vault is not available to this agent"));
        }
        let run = turn
            .provider_run_id
            .as_deref()
            .ok_or_else(|| error("the sudo window has no running provider turn"))
            .and_then(|run| self.watched_process_run(run))?;
        Ok(VaultElevation { turn, run })
    }

    /// The Vault unlock wait can outlast, revoke or replace the window.
    pub(super) fn recheck_vault_elevation(
        &self,
        token: &str,
        elevation: &VaultElevation,
    ) -> Result<(), DaemonError> {
        if self
            .sudo_for_auth_token(token)
            .ok()
            .is_none_or(|current| current.entry_id != elevation.turn.entry_id)
        {
            return Err(error("sudo window ended during the Vault wait"));
        }
        Ok(())
    }
}
