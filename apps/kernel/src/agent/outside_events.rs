//! MP-08 / MP-09 / MP-10 / MP-11: state and membership, never provider text.
use super::*;
use crate::local::{EventRefs, EventSessionState, OutsideEventValue};
impl AgentService {
    pub(crate) fn bind_outside_events(
        &mut self,
        store: crate::durable_state::DurableKernelStateStore,
    ) {
        self.outside_events = Some(store);
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
