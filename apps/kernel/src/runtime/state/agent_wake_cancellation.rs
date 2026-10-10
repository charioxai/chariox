//! MP-09 A03: close wake ownership before removing an agent or Room.
use super::*;
use crate::durable_state::agent_lifecycle::Operation;

impl KernelRuntimeOwnedState {
    /// Best-effort: a wake-ledger failure is reported but never blocks Room
    /// or agent teardown, and watched processes are stopped regardless.
    pub(super) fn cancel_owned_agent_wakes(&self, room: &str, agent: Option<&str>) {
        let store = &self.durable_state_store;
        if let Err(error) = store.agent_lifecycle(Operation::RetireWakes {
            room: room.into(),
            agent: agent.map(str::to_owned),
            now: crate::session::unix_epoch_ms(),
        }) {
            tracing::warn!(%error, "MP-09 A03: wake retirement failed during teardown");
            self.record_notice_for_agent(
                room,
                None,
                agent,
                self.attachment_store.list_session_attachment_ids(room),
                "Wake alert: armed wakes could not be retired from the durable ledger during teardown; watched processes are still stopped",
            );
        }
        match store.agent_wakes(Some(room), agent) {
            Ok(wakes) => {
                for wake in wakes.iter().filter(|w| w.kind == "process") {
                    self.agent_wakes.processes.terminate(&wake.id);
                }
            }
            Err(error) => {
                tracing::warn!(%error, "MP-09 A03: watched processes unavailable during teardown")
            }
        }
    }
}
