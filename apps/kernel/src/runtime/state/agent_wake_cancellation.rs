//! MP-09 A03: close wake ownership before removing an agent or Room.
use super::wake_driver::Cancel;
use super::*;
use crate::durable_state::agent_lifecycle::{AgentWake, Operation, Outcome};

impl KernelRuntimeOwnedState {
    /// Best-effort: a wake-ledger failure is reported but never blocks Room
    /// or agent teardown, and watched processes are stopped regardless.
    pub(super) fn cancel_owned_agent_wakes(&self, room: &str, agent: Option<&str>) {
        let store = &self.durable_state_store;
        match store.agent_lifecycle(Operation::RetireWakes {
            room: room.into(),
            agent: agent.map(str::to_owned),
            now: crate::session::unix_epoch_ms(),
        }) {
            // Timers are settled; a driver-held source is stopped physically
            // and settled once its driver has stopped it. Watched processes
            // settle on their supervised exit below.
            Ok(Outcome::Wakes(retired)) => {
                for wake in retired
                    .iter()
                    .filter(|w| w.state == "cancelling" && w.kind != "process")
                {
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
                    "Wake alert: armed wakes could not be retired from the durable ledger during teardown; watched processes are still stopped",
                );
            }
        }
        // Worker receipt sources share the same guarded supervisor and teardown.
        if let Ok(receipts) = store.leased_process_receipts() {
            for receipt in receipts
                .into_iter()
                .filter(|r| r.backing_room == room && agent.is_none_or(|a| a == r.backing_agent))
            {
                self.agent_wakes.processes.terminate(&receipt.wake.id);
            }
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

impl KernelRuntimeState {
    /// Cancels one wake for `task`; `prompt` names the calling turn, `None`
    /// is the task's own cancellation. A watched process is signalled by its
    /// supervisor and settles on its exit; any other source is stopped by its
    /// driver. Returns whether a process was signalled.
    pub(super) async fn cancel_agent_wake(
        &self,
        wake: &AgentWake,
        task: &str,
        prompt: Option<&str>,
    ) -> Result<bool, DaemonError> {
        if wake.kind != "process" {
            self.cancel_wake_source(wake, task, prompt)?;
            return Ok(false);
        }
        self.owned
            .durable_state_store
            .agent_lifecycle(Operation::CancelWake {
                id: wake.id.clone(),
                task: task.into(),
                prompt: prompt.map(str::to_owned),
                pending: true,
            })?;
        self.cancel_agent_watched_process(&wake.id).await
    }
}
