//! A holder that follows a target after a decision opened still hears of it.
use super::*;
use crate::durable_state::agent_lifecycle::outside::OpenDecision;
use std::collections::BTreeSet;

impl KernelRuntimeOwnedState {
    /// The decisions open now in the sessions of `targets` on this kernel, as
    /// the `pending` events a holder subscribing in time would have received.
    /// Called inside the subscribe mutation guard, which registrations also
    /// hold, so a decision is either listed here or published live.
    pub(in crate::runtime::state) fn outside_open_decisions(
        &self,
        targets: &[EventTarget],
    ) -> Vec<OpenDecision> {
        let kernel = self.config_projection.snapshot().daemon_id;
        let sessions: BTreeSet<_> = targets
            .iter()
            .filter(|target| target.kernel == kernel)
            .map(|target| target.session_id.as_str())
            .collect();
        let mut open = Vec::new();
        for id in sessions {
            let Some(Ok(session)) = self.session_store.try_get_session_for_projection(id) else {
                continue;
            };
            for interaction in session.active_interactions() {
                // Only decisions the kernel still waits on can be answered.
                let Some(pending) = self
                    .pending_interactions
                    .write()
                    .get(interaction.id())
                    .cloned()
                else {
                    continue;
                };
                let (value, summary) = self.outside_interaction_value(
                    interaction,
                    EventInteractionState::Pending,
                    pending.passkey_prompt.as_deref(),
                );
                open.push(OpenDecision {
                    session_id: id.into(),
                    agent_id: interaction.agent_id().map(str::to_string),
                    interaction_id: interaction.id().into(),
                    summary,
                    value,
                });
            }
        }
        open
    }
}
