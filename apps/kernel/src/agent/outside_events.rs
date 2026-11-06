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
        publish_membership(self.outside_events.as_ref(), agent, state)
    }
}

pub(super) fn publish_membership(
    store: Option<&crate::durable_state::DurableKernelStateStore>,
    agent: &AgentInstance,
    state: EventSessionState,
) -> Result<(), crate::error::DaemonError> {
    if agent.is_sub_agent() {
        // A sub-agent is no member of the room: followers hear that its parent's tree changed.
        let Some(store) = store else {
            return Ok(());
        };
        let root = agent.parent_agent_id().unwrap_or_default();
        store.outside_publish(
            agent.session_id(),
            Some(root),
            OutsideEventValue::AgentTree {
                root: root.to_string(),
                rev: super::tree_rev::bump(),
            },
            "agent tree changed",
            EventRefs::default(),
        )?;
        return Ok(());
    }
    if let Some(store) = store {
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
