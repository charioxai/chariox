//! A command that waits for room is told to the room: its followers get the
//! row as an outside event.
use super::process_wait_notices::Notice;
use super::*;
use crate::durable_state::agent_lifecycle::{Operation, Outcome};
use crate::local::{EventRefs, OutsideEventValue};
use crate::runtime::process_resources::{
    take_scope_kills, take_wait_events, ExhaustionStep, ScopeKill, SessionProcessOwner,
    SessionProcessState, WaitChange,
};

impl KernelRuntimeState {
    /// Delivers the waits that began or ended since the last call.
    pub(super) fn deliver_process_waits(&self) {
        self.deliver_process_waits_at(std::time::Instant::now());
    }

    pub(super) fn deliver_process_waits_at(&self, now: std::time::Instant) {
        for event in take_wait_events() {
            let Some((session_id, owner)) = self
                .process_owner(&event.keys)
                .or_else(|| event.owner.clone())
            else {
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
                (WaitChange::Exhaustion { step, message }, _) => (
                    message
                        .trim_end()
                        .trim_start_matches("chariox: ")
                        .to_string(),
                    None,
                    matches!(step, ExhaustionStep::Stopped).then(|| message.trim_end().to_string()),
                ),
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
        for kill in take_scope_kills() {
            self.tell_owner_of_scope_kill(&kill);
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

    /// The memory controller killed a process of the agent's unit. The agent
    /// owns it, so every process it waits on hears one occurrence now: a
    /// home-made wait on a file that process would have written ends here
    /// instead of at its deadline. Nothing is refused or stopped.
    fn tell_owner_of_scope_kill(&self, kill: &ScopeKill) {
        let Some((session_id, SessionProcessOwner::Agent(agent))) = self.process_owner(&kill.keys)
        else {
            return;
        };
        let store = &self.owned.durable_state_store;
        let waiting = match store.agent_wakes(Some(&session_id), Some(&agent)) {
            Ok(wakes) => wakes,
            Err(error) => {
                tracing::warn!(%error, "scope kill could not read the agent's waits");
                return;
            }
        };
        for wake in waiting
            .into_iter()
            .filter(|wake| wake.kind == "process" && wake.state == "running")
        {
            let occurred = store.agent_lifecycle(Operation::WakeOccurred {
                occurrence: format!("{}-oom-{}", wake.id, kill.total),
                id: wake.id,
                kind: "scope_process_killed".into(),
                payload: serde_json::json!({"reason": kill.reason}),
                summary: Some(kill.reason.clone()),
                end: None,
                now: crate::session::unix_epoch_ms(),
            });
            match occurred {
                Ok(Outcome::Wakes(wakes)) => {
                    for wake in wakes {
                        self.wake_notice(
                            &wake,
                            format!("Wake alert: a process of yours was killed: {}", kill.reason),
                        );
                        self.schedule_wake_delivery(&wake);
                    }
                }
                Ok(_) => {}
                Err(error) => tracing::warn!(%error, "scope kill was not delivered to the wait"),
            }
        }
    }

    fn process_owner(&self, keys: &[String]) -> Option<(String, SessionProcessOwner)> {
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
                .find(|(_, key)| keys.contains(key))
                .map(|(owner, _)| (session.id().to_string(), owner))
            })
    }
}
