//! MP-08 / MP-10 / MP-11: an agent's latest turn, read the way `GetSessionHistoryOutline` reads it.
use super::CommandRouter;
use crate::error::DaemonError;
use crate::local::{
    GetSessionHistoryOutlineRequest, LocalDaemonRequest, LocalDaemonResponse,
    SessionHistoryOutlineTurn,
};
use crate::runtime::history_executor::execute_history_request;

/// The agent's latest turn and the history revision it is at least as new as.
pub(crate) struct AgentTurnRead {
    pub(crate) history_revision: u64,
    pub(crate) turn: SessionHistoryOutlineTurn,
}

impl CommandRouter {
    /// `None` when the agent has no turn yet. The revision is read before the turn, so the turn is
    /// never older than its revision.
    pub(crate) async fn read_agent_latest_turn(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Option<AgentTurnRead>, DaemonError> {
        let history = self.operational_history_store.clone();
        let revision_session_id = session_id.to_string();
        let revision_agent_id = agent_id.to_string();
        // A revision query can wait on SQLite too; it must yield just like the outline read.
        let history_revision = tokio::task::spawn_blocking(move || {
            history.latest_session_agent_sequence(&revision_session_id, &revision_agent_id)
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "read agent history revision",
            message: format!("history revision worker failed: {error}"),
        })??;
        let response = execute_history_request(
            self.history_store.clone(),
            self.operational_history_store.clone(),
            &self.runtime_state,
            &self.config_projection,
            LocalDaemonRequest::GetSessionHistoryOutline(GetSessionHistoryOutlineRequest {
                session_id: session_id.to_string(),
                agent_ids: Some(vec![agent_id.to_string()]),
                latest_prompt_count: Some(1),
                cursor: None,
                include_prefetched_content: false,
            }),
        )
        .await?;
        let LocalDaemonResponse::SessionHistoryOutline { agents } = response else {
            return Err(DaemonError::LocalTransport {
                operation: "read agent latest turn",
                message: "history outline answered with another response".into(),
            });
        };
        Ok(agents
            .into_iter()
            .find(|agent| agent.agent_id == agent_id)
            .and_then(|agent| agent.turns.into_iter().next_back())
            .map(|turn| AgentTurnRead {
                history_revision,
                turn,
            }))
    }
}
