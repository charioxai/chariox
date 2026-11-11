//! A command that waits for room is told to the room: its followers get the
//! row as an outside event.
use super::process_wait_notices::Notice;
use super::*;
use crate::local::{EventRefs, OutsideEventValue};
use crate::runtime::process_resources::{
    take_wait_events, SessionProcessOwner, SessionProcessState, WaitChange, WaitEvent,
};

impl KernelRuntimeState {
    /// Delivers the waits that began or ended since the last call.
    pub(super) fn deliver_process_waits(&self) {
        self.deliver_process_waits_at(std::time::Instant::now());
    }

    pub(super) fn deliver_process_waits_at(&self, now: std::time::Instant) {
        for event in take_wait_events() {
            let Some((session_id, owner)) = self.process_wait_owner(&event) else {
                continue;
            };
            let (agent, terminal) = match owner {
                SessionProcessOwner::Agent(agent) => (Some(agent), None),
                SessionProcessOwner::Terminal(terminal) => (None, Some(terminal)),
            };
            if let Some(agent) = &agent {
                let ended = self
                    .owned
                    .process_wait_notices
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .record(&session_id, agent, &event, now);
                if let Some(notice) = ended {
                    self.tell_agent_of_wait(&notice);
                }
            }
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
        let due = self
            .owned
            .process_wait_notices
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .due(now);
        for notice in due {
            self.tell_agent_of_wait(&notice);
        }
    }

    /// Puts the notice in the agent's inbox as already delivered: the agent
    /// reads it when it looks, and no turn starts or is steered for it.
    fn tell_agent_of_wait(&self, notice: &Notice) {
        let now_ms = crate::session::unix_epoch_ms();
        let mut event = crate::durable_state::agent_lifecycle::occurrence(
            &notice.session_id,
            &notice.agent_id,
            "process-resources",
            &format!(
                "process-wait-{}-{now_ms}-{}",
                notice.process_id, notice.state
            ),
            "resource_notice",
            serde_json::json!({
                "message": notice.message,
                "state": notice.state,
                "command": notice.command,
                "resource": notice.resource,
                "process_id": notice.process_id,
            }),
        );
        event.state = "accepted".into();
        if let Err(error) = self.owned.durable_state_store.agent_lifecycle(
            crate::durable_state::agent_lifecycle::Operation::Occur(event),
        ) {
            tracing::warn!(%error, "process wait notice was not delivered to the agent inbox");
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
