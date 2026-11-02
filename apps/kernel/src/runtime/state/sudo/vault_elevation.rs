//! MP-08 / MP-09 / MP-10 / MP-11: shared elevation for agent Vault operations.
use super::{error, KernelRuntimeState, KernelSudoTurn};
use crate::config::{UserCredentialConfig, UserCredentialInjectionConfig};
use crate::error::DaemonError;

#[derive(Clone, Copy)]
pub(crate) enum VaultCaller<'a> {
    AuthToken(&'a str),
    ProviderRun(&'a str),
}

impl KernelRuntimeState {
    /// Shared admission for agent Vault operations; rechecked after every wait.
    pub(crate) fn vault_elevation(
        &self,
        caller: VaultCaller<'_>,
        operation: &str,
    ) -> Result<KernelSudoTurn, DaemonError> {
        let turn = match caller {
            VaultCaller::AuthToken(token) => self.sudo_for_auth_token(token),
            VaultCaller::ProviderRun(run) => self.sudo_for_provider_run(run),
        }
        .map_err(|_| error(format!("{operation} requires a live sudo window")))?;
        // Token-based creation/fill keeps the owner's Vault-management policy.
        // Run-based register metadata and existing factor use retain their sudo
        // admission even when the owner has disabled agent Vault management.
        if matches!(caller, VaultCaller::AuthToken(_))
            && (self.provider_account_authority_owner_user_id(&turn.owner_user_id)
                != crate::session::DEFAULT_LOCAL_USER_ID
                || self
                    .owned
                    .config_projection
                    .snapshot()
                    .user_config
                    .credential_vault
                    .agent_management
                    == crate::config::CredentialVaultAgentManagementPolicy::Deny)
        {
            return Err(error("the owner's Vault is not available to this agent"));
        }
        Ok(turn)
    }

    /// The Vault unlock wait can outlast, revoke or replace the window.
    pub(crate) fn recheck_vault_elevation(
        &self,
        token: &str,
        turn: &KernelSudoTurn,
    ) -> Result<(), DaemonError> {
        if self
            .vault_elevation(VaultCaller::AuthToken(token), "Vault operation")?
            .entry_id
            != turn.entry_id
        {
            return Err(error("sudo window ended during the Vault wait"));
        }
        Ok(())
    }

    /// Keep the exact current task check: an open window for another agent or
    /// an earlier task is not permission to capture or use this factor.
    pub(crate) fn require_vault_factor_elevation(
        &self,
        session_id: &str,
        agent_id: &str,
        credential: &UserCredentialConfig,
    ) -> Result<(), DaemonError> {
        if !matches!(
            credential.injection,
            UserCredentialInjectionConfig::Totp(_) | UserCredentialInjectionConfig::BackupCodes
        ) {
            return Ok(());
        }
        let run = self
            .owned
            .agent_store
            .get_agent(agent_id)
            .ok()
            .and_then(|agent| {
                if agent.session_id() != session_id {
                    return None;
                }
                if let Some(remote) = agent.remote_execution() {
                    remote.active_worker_provider_run_id.as_deref().map(|run| {
                        crate::provider::projected_leased_provider_run_id(
                            &remote.leased_agent_id,
                            run,
                        )
                    })
                } else {
                    self.owned
                        .provider_store
                        .get_run_for_agent(session_id, agent_id)
                        .map(|run| run.id().to_string())
                }
            });
        if run.is_some_and(|run| {
            self.vault_elevation(
                VaultCaller::ProviderRun(&run),
                "TOTP and backup-code capture/use",
            )
            .is_ok()
        }) {
            return Ok(());
        }
        Err(DaemonError::LocalTransport {
            operation: "vault_elevation",
            message: "TOTP and backup-code capture/use requires a live sudo window for this task"
                .into(),
        })
    }
}
