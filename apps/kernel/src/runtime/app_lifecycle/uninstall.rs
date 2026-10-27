//! Keep restore and uninstall mutually exclusive through authority/data retirement.
use super::*;
use crate::{local::AppRequestErrorCode as Error, runtime::app_snapshot_restore as snapshots};
impl AppLifecycleService {
    pub(crate) fn begin_uninstall_blocking(
        &self,
        owner: &str,
        installation: &str,
        generation: u64,
        delete_data: bool,
    ) -> snapshots::Result<Operation> {
        if self.0.stopped.load(Ordering::Acquire) {
            return Err(Error::Busy);
        }
        if installation.is_empty()
            || installation.starts_with('.')
            || !matches!(
                std::path::Path::new(installation).components().next(),
                Some(std::path::Component::Normal(_))
            )
            || std::path::Path::new(installation).components().count() != 1
        {
            return Err(Error::InvalidRequest);
        }
        let key = (owner.to_owned(), installation.to_owned());
        let gate = self
            .0
            .operation(key.clone())
            .map_err(super::restore::code)?;
        let current = self
            .0
            .store
            .get_app_installation(owner, installation)
            .map_err(crate::runtime::app_control::registry_error)?;
        if current.generation != generation {
            return Err(Error::Conflict);
        }
        // Retained data must first reach the restore's authoritative decision.
        // Explicit data erasure may abandon a corrupt/unresolvable journal.
        if !delete_data && snapshots::pending(&self.0.store, installation)? {
            return Err(Error::Conflict);
        }
        self.stop_under_gate(owner, installation, key)
            .map_err(super::restore::code)?;
        Ok(gate)
    }
}
