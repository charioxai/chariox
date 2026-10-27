//! A manual stop remains owned even if cancellation wins before initial claim.
//! The existing writer records restart intent; no second journal is introduced.
use super::*;

pub(super) fn persist(
    store: &DurableKernelStateStore,
    owner: &str,
    installation: &str,
    control: &Control,
    budget: AppOperationBudget,
) -> Result<()> {
    if !control.pending_manual_stop() {
        return Ok(());
    }
    cancel_first(store, owner, control, budget.fork(|| false))?;
    store.stop_app_worker_intent(owner, installation, budget.fork(|| false))?;
    store.finish_app_worker_stop(owner, installation, budget)?;
    control.confirm_manual_stop();
    Ok(())
}

/// A precommit first install has a separate durable operation identity. Its
/// recovery must be fenced too; a committed generation instead uses the normal
/// worker stop intent and can never be rolled back by this cancellation.
pub(super) fn cancel_first(
    store: &DurableKernelStateStore,
    owner: &str,
    control: &Control,
    budget: AppOperationBudget,
) -> Result<()> {
    let Some(request_id) = &control.first_request else {
        return Ok(());
    };
    match store.cancel_first_app_install(owner, request_id, budget) {
        Ok(_) => Ok(()),
        Err(InstallOperationError::Conflict)
            if store.first_app_install_status(owner, request_id)?.phase
                == InstallPhase::Committed =>
        {
            Ok(())
        }
        Err(InstallOperationError::CommitUnknown) => {
            let _ = store.fence_writer();
            Err(LifecycleError::CommitUnknown)
        }
        Err(error) => Err(error.into()),
    }
}

impl AppLifecycleService {
    /// Called only by the bounded maintenance owner while holding its App
    /// permit. A transient failure retains the entry and prevents recovery
    /// from treating a stopped installation as an absent owner.
    pub(super) fn persist_pending_manual_stops(&self) {
        self.persist_pending_manual_stops_impl(
            #[cfg(test)]
            None,
        );
    }
    #[cfg(test)]
    pub(super) fn fixture_persist_pending_manual_stops(&self, mut checkpoint: impl FnMut(bool)) {
        self.persist_pending_manual_stops_impl(Some(&mut checkpoint));
    }
    fn persist_pending_manual_stops_impl(
        &self,
        #[cfg(test)] mut checkpoint: Option<&mut dyn FnMut(bool)>,
    ) {
        let pending: Vec<_> = self
            .0
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter(|(_, entry)| entry.control.finished() && entry.control.pending_manual_stop())
            .take(LIVE_LIMIT)
            .map(|(key, entry)| (key.clone(), entry.clone()))
            .collect();
        #[cfg(test)]
        if let Some(checkpoint) = &mut checkpoint {
            checkpoint(false);
        }
        let budget = AppOperationBudget::from_supervisor(|| false);
        for (key, entry) in pending {
            if budget.check().is_err() {
                break;
            }
            let Ok(_operation) = self.0.operation(key.clone()) else {
                continue;
            };
            let current = self
                .0
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&key)
                .is_some_and(|current| {
                    Arc::ptr_eq(current, &entry)
                        && current.control.finished()
                        && current.control.pending_manual_stop()
                });
            if !current {
                continue;
            }
            #[cfg(test)]
            if let Some(checkpoint) = &mut checkpoint {
                checkpoint(true);
            }
            // The same foreground operation guard remains held through both
            // durable writes. A retained old Arc can never stop its replacement.
            let _ = persist(
                &self.0.store,
                &key.0,
                &key.1,
                &entry.control,
                budget.fork(|| false),
            );
        }
    }
}
