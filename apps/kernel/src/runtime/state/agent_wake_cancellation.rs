//! MP-09 A03: close wake ownership before removing an agent or Room.
use super::wake_driver::Cancel;
use super::*;
use crate::durable_state::agent_lifecycle::{Operation, Outcome};

impl KernelRuntimeOwnedState {
    /// Best-effort: a wake-ledger failure is reported but never blocks Room
    /// or agent teardown.
    pub(super) fn cancel_owned_agent_wakes(&self, room: &str, agent: Option<&str>) {
        let store = &self.durable_state_store;
        match store.agent_lifecycle(Operation::RetireWakes {
            room: room.into(),
            agent: agent.map(str::to_owned),
            now: crate::session::unix_epoch_ms(),
        }) {
            // Timers are settled; a driver-held source is stopped physically
            // and settled once its driver has stopped it.
            Ok(Outcome::Wakes(retired)) => {
                for wake in retired.iter().filter(|w| w.state == "cancelling") {
                    let stopped = self
                        .agent_wakes
                        .drivers
                        .get(&wake.kind)
                        .and_then(|driver| driver.cancel(wake));
                    if matches!(stopped, Ok(Cancel::Done)) {
                        if let Err(error) = store.agent_lifecycle(Operation::CancelWake {
                            id: wake.id.clone(),
                            task: wake.task_id.clone(),
                            prompt: None,
                            pending: false,
                        }) {
                            tracing::warn!(%error, "MP-09 A03: stopped wake not settled during teardown");
                        }
                    } else if let Err(error) = stopped {
                        tracing::warn!(%error, "MP-09 A03: wake source not stopped during teardown");
                    }
                }
            }
            Ok(_) => unreachable!(),
            Err(error) => {
                tracing::warn!(%error, "MP-09 A03: wake retirement failed during teardown");
                self.record_notice_for_agent(
                room,
                None,
                agent,
                self.attachment_store.list_session_attachment_ids(room),
                "Wake alert: armed wakes could not be retired from the durable ledger during teardown",
            );
            }
        }
    }
}
