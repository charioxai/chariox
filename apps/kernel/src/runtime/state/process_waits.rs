//! A command that waits for room is told to the room: its followers get the
//! row as an outside event.
use super::*;
use crate::local::{EventRefs, OutsideEventValue};
use crate::runtime::process_resources::{
    take_wait_events, SessionProcessOwner, SessionProcessState, WaitChange, WaitEvent,
};

impl KernelRuntimeState {
    /// Delivers the waits that began or ended since the last call.
    pub(super) fn deliver_process_waits(&self) {
        for event in take_wait_events() {
            let Some((session_id, owner)) = self.process_wait_owner(&event) else {
                continue;
            };
            let (agent, terminal) = match owner {
                SessionProcessOwner::Agent(agent) => (Some(agent), None),
                SessionProcessOwner::Terminal(terminal) => (None, Some(terminal)),
            };
            let (change, state) = (&event.change, &event.state);
            let (summary, waited_ms, ended) = match (change, state) {
                (WaitChange::Stopped { reason }, _) => {
                    ("stopped".to_string(), None, Some(reason.clone()))
                }
                (WaitChange::Resumed { waited_ms }, _) => {
                    ("running again".to_string(), Some(*waited_ms), None)
                }
                (_, SessionProcessState::Held(row) | SessionProcessState::Paused(row)) => (
                    format!("waiting for room: {}", row.command_line),
                    None,
                    None,
                ),
                _ => continue,
            };
            if let Err(error) = self.owned.durable_state_store.outside_publish(
                &session_id,
                agent.as_deref(),
                OutsideEventValue::Process {
                    process_id: event.process_id.clone(),
                    state: event.state.clone(),
                    waited_ms,
                    ended,
                },
                &summary,
                EventRefs {
                    terminal_id: terminal,
                    ..Default::default()
                },
            ) {
                tracing::warn!(%error, "process wait event was not published");
            }
        }
    }

    fn process_wait_owner(&self, event: &WaitEvent) -> Option<(String, SessionProcessOwner)> {
        self.owned
            .session_store
            .list_non_ended_sessions_including_hidden()
            .into_iter()
            .find_map(|session| {
                crate::runtime::process_resources::keys_for_session(
                    session.id(),
                    &self.owned.provider_store,
                    &self.owned.room_terminals,
                )
                .into_iter()
                .find(|(_, key)| event.keys.contains(key))
                .map(|(owner, _)| (session.id().to_string(), owner))
            })
    }
}
