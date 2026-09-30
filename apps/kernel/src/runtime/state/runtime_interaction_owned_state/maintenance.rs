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
        self.remove_orphaned_kernel_operation_interactions(shutdown);
    }

    /// A kernel-operation decision or vault prompt persisted in a session
    /// outlives its responder when the kernel stops without its shutdown sweep.
    /// Nothing can answer it after a restart (an answer is "not pending"), yet
    /// every terminal keeps showing it and a decision's subject would block a
    /// retry, so it is dropped from the session. Hidden sessions are scanned
    /// too: decisions can be registered in them.
    fn remove_orphaned_kernel_operation_interactions(&self, shutdown: bool) {
        if !self.orphan_sweep_due(shutdown) {
            return;
        }
        let Ok(_mutation) = self.pending_interactions.mutation.lock() else {
            return;
        };
        let orphans = {
            let pending = self.pending_interactions.write();
            self.session_store
                .list_non_ended_sessions_including_hidden()
                .iter()
                .flat_map(|session| {
                    session
                        .active_interactions()
                        .iter()
                        .filter(|interaction| {
                            waits_in_memory(interaction)
                                && !pending.get(interaction.id()).is_some_and(|value| {
                                    value.session_id == session.id()
                                        && value.belongs_to(&self.session_store)
                                })
                        })
                        .map(|interaction| (session.id().to_owned(), interaction.id().to_owned()))
                        .collect::<Vec<_>>()
                })
                .take(32)
                .collect::<Vec<_>>()
        };
        for (session_id, interaction_id) in orphans {
            let activity_mutation = self.begin_managed_activity_mutation();
            let mut sessions = self.session_store.write();
            let Ok(session) = sessions.get_session(&session_id) else {
                continue;
            };
            let mut session = session.clone();
            if session.remove_active_interaction(&interaction_id).is_none() {
                continue;
            }
            sessions.restore_session(session);
            activity_mutation.record();
            drop(sessions);
            let _ = self.session_snapshot(&session_id);
            self.terminal_stream
                .notify_terminal_projection_change(&session_id);
        }
    }

    /// At most one scan every 30 s per session store; the first is immediate.
    fn orphan_sweep_due(&self, shutdown: bool) -> bool {
        const INTERVAL_MS: u64 = 30_000;
        let now = crate::session::unix_epoch_ms();
        let identity = self.session_store.weak_identity();
        let Ok(mut sweeps) = self.pending_interactions.orphan_sweeps.lock() else {
            return false;
        };
        sweeps.retain(|(store, _)| store.strong_count() > 0);
        match sweeps
            .iter_mut()
            .find(|(store, _)| std::sync::Weak::ptr_eq(store, &identity))
        {
            Some((_, last)) if !shutdown && now.saturating_sub(*last) < INTERVAL_MS => false,
            Some((_, last)) => {
                *last = now;
                true
            }
            None => {
                sweeps.push((identity, now));
                true
            }
        }
    }
}

/// Answered only through a responder held by this kernel process: kernel
/// operation decisions and vault unlock prompts (`vault-unlock-…`).
fn waits_in_memory(interaction: &crate::session::RuntimeInteraction) -> bool {
    interaction.kernel_operation_id().is_some() || interaction.id().starts_with("vault-unlock-")
}
