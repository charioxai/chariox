//! A command that waits for room is told to the room: its followers get the
//! row as an outside event, and the person is told once a wait grows long.
use super::*;
use crate::local::{EventRefs, OutsideEventValue};
use crate::runtime::process_resources::{
    long_wait_notice, take_wait_events, SessionProcessOwner, SessionProcessState, WaitChange,
    WaitEvent,
};

impl KernelRuntimeState {
    /// Delivers the waits that began, ended or grew long since the last call.
    pub(super) fn deliver_process_waits(&self) {
        for event in take_wait_events() {
            let Some((session_id, owner)) = self.process_wait_owner(&event) else {
                continue;
            };
            let (agent, terminal) = match owner {
                SessionProcessOwner::Agent(agent) => (Some(agent), None),
                SessionProcessOwner::Terminal(terminal) => (None, Some(terminal)),
            };
            let now = crate::session::unix_epoch_ms();
            match (&event.change, &event.state) {
                (
                    WaitChange::Long,
                    SessionProcessState::Held(row) | SessionProcessState::Paused(row),
                ) => {
                    self.owned.record_notice_for_agent(
                        &session_id,
                        None,
                        agent.as_deref(),
                        self.owned
                            .attachment_store
                            .list_session_attachment_ids(&session_id),
                        long_wait_notice(row, now),
                    );
                }
                (change, state) => {
                    let (summary, waited_ms) = match (change, state) {
                        (WaitChange::Resumed { waited_ms }, _) => {
                            ("running again".to_string(), Some(*waited_ms))
                        }
                        (_, SessionProcessState::Held(row) | SessionProcessState::Paused(row)) => {
                            (format!("waiting for room: {}", row.command_line), None)
                        }
                        _ => continue,
                    };
                    if let Err(error) = self.owned.durable_state_store.outside_publish(
                        &session_id,
                        agent.as_deref(),
                        OutsideEventValue::Process {
                            process_id: event.process_id.clone(),
                            state: event.state.clone(),
                            waited_ms,
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
