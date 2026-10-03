use std::path::Path;

use crate::error::DaemonError;

use super::policy::prune_expired;
use super::{ManagedContextTransferPhase, ManagedContextTransferStore};

impl ManagedContextTransferStore {
    // A terminal transfer is never resumed. Cleanup is best effort, but its
    // record must survive when deletion cannot prove ownership of its bytes.
    pub(super) fn cleanup_failed_transfers(&self) {
        let failed = {
            let state = self.lock_state();
            state
                .entries
                .iter()
                .filter(|(_, entry)| entry.phase == ManagedContextTransferPhase::Failed)
                .map(|(id, entry)| (id.clone(), entry.destination_root.clone()))
                .collect::<Vec<_>>()
        };
        for (id, destination) in failed {
            if let Err(error) = self.cleanup_failed_transfer(&id, &destination) {
                report_retained_transfer(&id, &error);
            }
        }
    }

    pub(super) fn prune_expired_transfers(&self, now_ms: u64) -> Result<(), DaemonError> {
        let mut state = self.lock_state();
        let before = state.clone();
        let expired = prune_expired(&mut state, now_ms);
        if expired.is_empty() {
            return Ok(());
        }
        let mut removed_any = false;
        for id in expired {
            let entry = before.entries.get(&id).expect("pruned transfer existed");
            let cleanup = if entry.phase == ManagedContextTransferPhase::Failed {
                self.cleanup_failed_transfer(&id, &entry.destination_root)
            } else {
                self.cleanup_transfer_artifacts(&id)
            };
            match cleanup {
                Ok(()) => removed_any = true,
                Err(error) if entry.phase == ManagedContextTransferPhase::Failed => {
                    state.entries.insert(id.clone(), entry.clone());
                    report_retained_transfer(&id, &error);
                }
                Err(error) => {
                    *state = before;
                    return Err(error);
                }
            }
        }
        if !removed_any {
            return Ok(());
        }
        if let Err(error) = self.persist_locked(&state) {
            *state = before;
            return Err(error);
        }
        Ok(())
    }

    fn cleanup_failed_transfer(&self, id: &str, destination: &Path) -> Result<(), DaemonError> {
        // Validate the publication before touching staging or archives: a
        // missing/mismatched ownership proof must preserve all ambiguous data.
        crate::managed_context::development::cleanup_development_context_publication(
            destination,
            id,
        )?;
        crate::managed_context::development::cleanup_development_context_publication_staging(
            destination,
            id,
        )?;
        self.cleanup_transfer_artifacts(id)
    }
}

fn report_retained_transfer(id: &str, error: &DaemonError) {
    crate::logging::warn_with_fields(
        "managed_context.transfer",
        "managed context transfer cleanup failed; Failed record and remaining artifacts retained",
        serde_json::json!({ "transfer_id": id, "error": error.to_string() }),
    );
}
