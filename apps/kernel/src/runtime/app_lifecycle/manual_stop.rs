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
    store.stop_app_worker_intent(owner, installation, budget.fork(|| false))?;
    store.finish_app_worker_stop(owner, installation, budget)?;
    control.confirm_manual_stop();
    Ok(())
}
impl AppLifecycleService {
    /// Called only by the bounded maintenance owner while holding its App
    /// permit. A transient failure retains the entry and prevents recovery
    /// from treating a stopped installation as an absent owner.
    pub(super) fn persist_pending_manual_stops(&self) {
        let pending: Vec<_> = self
            .0
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter(|(_, entry)| entry.control.finished() && entry.control.pending_manual_stop())
            .take(LIVE_LIMIT)
            .map(|((owner, id), entry)| (owner.clone(), id.clone(), entry.control.clone()))
            .collect();
        let budget = AppOperationBudget::from_supervisor(|| false);
        for (owner, id, control) in pending {
            if budget.check().is_err() {
                break;
            }
            let _ = persist(&self.0.store, &owner, &id, &control, budget.fork(|| false));
        }
    }
}
