//! Release queued work through its current local or remote route after Move settles.
use super::*;

impl KernelRuntimeState {
    pub(super) fn spawn_agent_move_queue_promotion(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id
            || self.owned.agent_moves.dispatch_fenced(session_id, agent_id)
        {
            return Ok(());
        }
        if agent.remote_execution().is_none() {
            return self.spawn_idle_local_prompt_queue_promotion(session_id, agent_id);
        }
        let state = self.clone();
        let session_id = session_id.to_string();
        let agent_id = agent_id.to_string();
        tokio::spawn(async move {
            if let Err(error) = state
                .spawn_next_queued_remote_prompt(&session_id, &agent_id)
                .await
            {
                state.owned.record_notice(
                    &session_id,
                    None,
                    state
                        .owned
                        .attachment_store
                        .list_session_attachment_ids(&session_id),
                    format!("Queued prompt could not advance after Move: {error}"),
                );
            }
        });
        Ok(())
    }
}
