//! MP-08 / MP-09 / MP-10 / MP-11: whoever follows an agent hears when it is removed or archived.
use super::*;
use crate::durable_state::agent_lifecycle::Operation;

/// Why an agent runs no more turns.
#[derive(Clone, Copy)]
pub(super) enum AgentGone {
    /// The agent was removed: its followers hear a failed turn.
    Removed,
    /// Its room ended (deleted or archived): its followers hear the loss a follower on another
    /// kernel hears, not a turn that failed.
    RoomEnded,
}

impl AgentGone {
    fn occurrence(self, agent: &str, now: u64) -> String {
        match self {
            Self::Removed => format!("agent-gone-{agent}-{now}"),
            Self::RoomEnded => format!(
                "{}room-ended-{agent}-{now}",
                crate::durable_state::agent_lifecycle::peer_follows::SOURCE_LOST_OCCURRENCE
            ),
        }
    }
}

impl KernelRuntimeOwnedState {
    /// A removed or archived agent never ends another turn. Its source is recorded as lost, so a
    /// follower in the same room or in another room of the owner hears of it.
    /// A spawn it was never prompted for is released first: nothing was delegated, so that
    /// obligation is cancelled, and only work it had really taken settles as failed.
    pub(super) fn announce_agent_gone(&self, room: &str, agent: &str, gone: AgentGone) {
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
                occurrence: gone.occurrence(agent, now),
                success: false,
            })
        {
            tracing::warn!(%error, %room, %agent, "MP-08: followers of a removed agent were not told");
        }
    }
}
