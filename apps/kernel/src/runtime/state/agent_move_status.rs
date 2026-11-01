//! Home publishes an unsettled Move on the agent: a deferred Move outlives the request that asked
//! for it, and a Move that cannot continue must stay visible so the person can cancel or retry it.
use super::*;
use crate::agent_move::{AgentMoveStage, AgentMoveStatus};

impl KernelRuntimeState {
    pub(super) async fn publish_agent_move_status(
        &self,
        agent_id: &str,
        status: Option<AgentMoveStatus>,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self
            .with_app_side_effect(|app| {
                let mut agent = app.agents().get_agent(agent_id)?;
                if agent.agent_move() == status.as_ref() {
                    return Ok::<_, DaemonError>(agent);
                }
                agent.set_agent_move(status);
                app.agents().restore_agent(agent.clone());
                app.durable_state_store().append_event(
                    "agent.updated",
                    Some(agent.id().into()),
                    serde_json::json!({"agent": agent, "source": "agent_move_status"}),
                )?;
                Ok(agent)
            })
            .await?;
        self.owned.session_snapshot(agent.session_id())?;
        Ok(agent)
    }

    /// A restart ends every wait for a turn, and a Move without a journalled operation cannot continue.
    pub(super) async fn clear_stale_agent_move_statuses(&self) {
        for agent in self.owned.agent_store.list_agents() {
            let stale = match agent.agent_move().map(|status| &status.stage) {
                Some(AgentMoveStage::WaitingForTurn) => true,
                Some(AgentMoveStage::Moving) => self
                    .owned
                    .agent_moves
                    .for_agent(agent.session_id(), agent.id())
                    .is_none_or(|operation| !operation.phase.is_pending()),
                _ => false,
            };
            if stale {
                if let Err(error) = self.publish_agent_move_status(agent.id(), None).await {
                    crate::logging::warn_with_fields(
                        "agent_move.status",
                        "A stale Move status could not be cleared; cancel the Move from the agent menu",
                        serde_json::json!({"agent_id": agent.id(), "error": error.to_string()}),
                    );
                }
            }
        }
    }
}
