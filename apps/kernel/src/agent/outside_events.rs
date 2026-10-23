//! MP-08 / MP-09 / MP-10 / MP-11: state and membership, never provider text.
use super::*;
use crate::local::{EventRefs, EventRunState, EventSessionState, OutsideEventValue};
impl AgentService {
    pub(crate) fn bind_outside_events(
        &mut self,
        store: crate::durable_state::DurableKernelStateStore,
    ) {
        self.outside_events = Some(store);
    }
    pub(super) fn publish_outside_state(
        &self,
        agent: &AgentInstance,
    ) -> Result<(), crate::error::DaemonError> {
        if let Some(store) = &self.outside_events {
            let state = match agent.state() {
                AgentState::Working => EventRunState::Working,
                AgentState::Error => EventRunState::Error,
                AgentState::Idle | AgentState::Focused => EventRunState::Idle,
            };
            store.outside_publish(
                agent.session_id(),
                Some(agent.id()),
                OutsideEventValue::Agent {
                    run_state: Some(state),
                    task_state: None,
                    reason: None,
                },
                match state {
                    EventRunState::Working => "working",
                    EventRunState::Idle => "idle",
                    EventRunState::Error => "error",
                },
                EventRefs::default(),
            )?;
        }
        Ok(())
    }
    pub(super) fn publish_outside_membership(
        &self,
        agent: &AgentInstance,
        state: EventSessionState,
    ) -> Result<(), crate::error::DaemonError> {
        if let Some(store) = &self.outside_events {
            store.outside_publish(
                agent.session_id(),
                Some(agent.id()),
                OutsideEventValue::Session { state },
                match state {
                    EventSessionState::AgentAdded => "agent added",
                    EventSessionState::AgentRemoved => "agent removed",
                    EventSessionState::Ended => "session ended",
                },
                EventRefs::default(),
            )?;
        }
        Ok(())
    }
}
