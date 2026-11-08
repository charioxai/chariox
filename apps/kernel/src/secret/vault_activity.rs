//! Encrypted, value-free Activity shared by owner controls and agent operations.
use super::*;
use crate::local::OwnerVaultActivity;

type Activity = BTreeMap<String, Vec<OwnerVaultActivity>>;
static ACTIVITY_LOCK: Mutex<()> = Mutex::new(());
/// Rows older than this belong to an earlier kernel process.
static PROCESS_STARTED_AT_MS: std::sync::LazyLock<u64> =
    std::sync::LazyLock::new(crate::session::unix_epoch_ms);
const UNFINISHED_USE: &str = "Use failed: the kernel stopped before it finished";

/// A kernel that stops between "Use attempted" and its result row cannot write
/// the result; the attempt then reads as failed closed instead of staying open.
fn close_unfinished_attempts(
    events: Vec<OwnerVaultActivity>,
    started_at_ms: u64,
) -> Vec<OwnerVaultActivity> {
    let mut closed = Vec::with_capacity(events.len());
    let mut open: Option<u64> = None;
    let mut fail_open = |closed: &mut Vec<OwnerVaultActivity>, open: &mut Option<u64>| {
        if let Some(at_ms) = open.take().filter(|at_ms| *at_ms < started_at_ms) {
            closed.push(OwnerVaultActivity {
                at_ms,
                event: UNFINISHED_USE.into(),
            });
        }
    };
    for event in events {
        if event.event.starts_with("Use attempted") {
            fail_open(&mut closed, &mut open);
            open = Some(event.at_ms);
        } else if event.event.starts_with("Used") || event.event.starts_with("Use failed") {
            open = None;
        }
        closed.push(event);
    }
    fail_open(&mut closed, &mut open);
    closed
}

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
        let events = self.read_vault_activity()?.remove(id).unwrap_or_default();
        Ok(close_unfinished_attempts(events, *PROCESS_STARTED_AT_MS))
    }

    pub(crate) fn record_vault_activity(&self, id: &str, event: &str) -> Result<(), DaemonError> {
        let _guard = ACTIVITY_LOCK.lock().map_err(|_| {
            secret_error(
                "vault_activity",
                "Vault Activity is unavailable; restart the kernel".into(),
            )
        })?;
        std::sync::LazyLock::force(&PROCESS_STARTED_AT_MS);
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
