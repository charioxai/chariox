//! Reuse the kernel pump to expire decisions and remove abandoned responders.
//! No timer task or caller lifetime can retain an approval slot indefinitely.
use super::*;

impl KernelRuntimeOwnedState {
    pub(in crate::runtime::state) fn sweep_kernel_operation_interactions(&self, shutdown: bool) {
        {
            let Ok(_mutation) = self.pending_interactions.mutation.lock() else {
                return;
            };
            self.pending_interactions.prune_abandoned_kernel_owners();
        }
        let candidates = self
            .pending_interactions
            .write()
            .iter()
            .filter(|(_, pending)| pending.belongs_to(&self.session_store))
            .filter(|(_, pending)| pending.kernel_operation_owner.is_some())
            .filter(|(_, pending)| {
                shutdown
                    || pending
                        .kernel_operation_deadline
                        .is_some_and(|deadline| std::time::Instant::now() >= deadline)
                    || pending.responder.lock().map_or(true, |sender| {
                        sender.as_ref().is_none_or(|sender| sender.is_closed())
                    })
            })
            .take(32)
            .map(|(id, pending)| (id.clone(), pending.clone()))
            .collect::<Vec<_>>();
        for (id, pending) in candidates {
            let _ = self.timeout_runtime_interaction_if_current(
                &pending.session_id,
                &id,
                Some(&pending),
            );
        }
    }
}
