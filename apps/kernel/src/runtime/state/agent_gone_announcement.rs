//! MP-08 / MP-09 / MP-10 / MP-11: whoever follows an agent hears when it is removed or archived.
use super::*;
use crate::durable_state::agent_lifecycle::Operation;

impl KernelRuntimeOwnedState {
    /// A removed or archived agent never ends another turn. Its source is recorded as lost, so a
    /// follower in the same room or in another room of the owner gets the event a failed turn gives.
    /// A spawn it was never prompted for is released first: nothing was delegated, so that
    /// obligation is cancelled, and only work it had really taken settles as failed.
    pub(super) fn announce_agent_gone(&self, room: &str, agent: &str) {
        if let Err(error) = self
            .durable_state_store
            .agent_lifecycle(Operation::ReleaseSpawn {
                room: room.into(),
                source: agent.into(),
            })
        {
            tracing::warn!(%error, %room, %agent, "MP-08: the spawn of a removed agent was not released");
        }
        let now = crate::session::unix_epoch_ms();
        if let Err(error) = self
            .durable_state_store
            .agent_lifecycle(Operation::SourceOutcome {
                now,
                public_answer: None,
                room: room.into(),
                source: agent.into(),
                occurrence: format!("agent-gone-{agent}-{now}"),
                success: false,
            })
        {
            tracing::warn!(%error, %room, %agent, "MP-08: followers of a removed agent were not told");
        }
    }
}
