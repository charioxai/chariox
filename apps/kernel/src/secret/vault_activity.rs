//! Encrypted, value-free Activity shared by owner controls and agent operations.
use super::*;
use crate::local::OwnerVaultActivity;

type Activity = BTreeMap<String, Vec<OwnerVaultActivity>>;
static ACTIVITY_LOCK: Mutex<()> = Mutex::new(());

impl RuntimeSecretService {
    fn read_vault_activity(&self) -> Result<Activity, DaemonError> {
        self.vault_store
            .find_secret(self.vault_service_name()?, VAULT_ACTIVITY_KEY)?
            .map(|text| {
                serde_json::from_str(&text).map_err(|_| {
                    secret_error(
                        "vault_activity",
                        "Vault Activity could not be read; restore its backup before retrying"
                            .into(),
                    )
                })
            })
            .transpose()
            .map(Option::unwrap_or_default)
    }

    pub(crate) fn vault_activity(&self, id: &str) -> Result<Vec<OwnerVaultActivity>, DaemonError> {
        let _guard = ACTIVITY_LOCK.lock().map_err(|_| {
            secret_error(
                "vault_activity",
                "Vault Activity is unavailable; restart the kernel".into(),
            )
        })?;
        Ok(self.read_vault_activity()?.remove(id).unwrap_or_default())
    }

    pub(crate) fn record_vault_activity(&self, id: &str, event: &str) -> Result<(), DaemonError> {
        let _guard = ACTIVITY_LOCK.lock().map_err(|_| {
            secret_error(
                "vault_activity",
                "Vault Activity is unavailable; restart the kernel".into(),
            )
        })?;
        let service = self.vault_service_name()?;
        let previous = self.vault_store.find_secret(service, VAULT_ACTIVITY_KEY)?;
        let mut activity = self.read_vault_activity()?;
        activity
            .entry(id.into())
            .or_default()
            .push(OwnerVaultActivity {
                at_ms: crate::session::unix_epoch_ms(),
                event: event.into(),
            });
        let text = serde_json::to_string(&activity).map_err(|_| {
            secret_error(
                "vault_activity",
                "Could not save Vault Activity; try again".into(),
            )
        })?;
        if let Err(error) = self
            .vault_store
            .set_secret(service, VAULT_ACTIVITY_KEY, &text)
        {
            let rollback = match previous {
                Some(text) => self
                    .vault_store
                    .set_secret(service, VAULT_ACTIVITY_KEY, &text),
                None => self.vault_store.delete_secret(service, VAULT_ACTIVITY_KEY),
            };
            if rollback.is_err() {
                return Err(DaemonError::MachineRequest { code: "vault_owner_recovery_required".into(), message: "Vault Activity could not be restored. Restore the Vault backup before retrying.".into() });
            }
            return Err(error);
        }
        Ok(())
    }
}
