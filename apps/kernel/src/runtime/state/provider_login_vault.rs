//! The Vault value a provider's own API-key login takes: read in the kernel,
//! typed into the login by the kernel, never handed to a model.
use super::*;
use crate::config::VaultUseTarget;
use crate::runtime::command::KernelCommand;

const OPERATION: &str = "provider_login_vault_entry";

impl KernelRuntimeState {
    /// The value of an owner Vault entry bound to exactly `origin`. An entry
    /// for any other use is refused before the Vault is asked to unlock; a
    /// locked Vault waits for the owner when an agent of a session asks and is
    /// refused for a client that has to unlock it first.
    pub(crate) async fn read_provider_login_vault_value(
        &self,
        command: &KernelCommand,
        credential_id: &str,
        origin: &'static str,
    ) -> Result<zeroize::Zeroizing<String>, DaemonError> {
        let actor = command.agent_id.as_deref().unwrap_or("the owner's client");
        let service = self.home_runtime_secret_service()?;
        if let Err(refused) = service.validate_provider_login_secret(credential_id, origin) {
            if service.credential(credential_id).is_ok() {
                self.record_vault_refusal(
                    credential_id,
                    &VaultUseTarget::Site {
                        origin: origin.to_string(),
                    },
                    actor,
                )?;
            }
            return Err(refused);
        }
        let _unlocked = match command.session_id.as_deref() {
            Some(session_id) => {
                let agent_id = self
                    .vault_prompt_agent(session_id, command.agent_id.as_deref(), OPERATION)
                    .await?;
                Some(
                    self.ensure_vault_unlocked_for_agent(session_id, &agent_id, OPERATION)
                        .await?,
                )
            }
            None => {
                self.require_unlocked_vault()?;
                None
            }
        };
        // The unlock can wait while the owner changes this entry.
        let service = self.home_runtime_secret_service()?;
        service.validate_provider_login_secret(credential_id, origin)?;
        let attempt = service.begin_vault_use(
            credential_id,
            &format!("Use attempted at {origin} by {actor}"),
        )?;
        let value = service.provider_login_secret_input(credential_id, origin)?;
        attempt.finish(&format!("Used at {origin} by {actor}"))?;
        Ok(value)
    }
}
