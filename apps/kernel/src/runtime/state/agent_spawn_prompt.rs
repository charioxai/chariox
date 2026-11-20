use super::*;
use crate::agent::AgentInstance;
use crate::session::{PromptQueueItem, PromptStatus, PromptSubmissionOutcome};

impl KernelRuntimeState {
    pub(crate) async fn start_spawn_prompt(
        &self,
        agent: &AgentInstance,
        author_name: &str,
        task: String,
    ) -> Result<(), DaemonError> {
        let room = agent.session_id();
        // One agent can have only one first prompt, even when its spawn is retried.
        let prompt_id = format!("spawn-prompt-{}", agent.id());
        if let Some(existing) =
            self.owned
                .durable_state_store
                .agent_turn_task(room, agent.id(), &prompt_id)?
        {
            // The prompt owner's normal replay covers live and queued prompts. The task
            // ledger also survives settlement, when the prompt itself leaves the queue.
            if matches!(
                existing.state,
                crate::durable_state::agent_lifecycle::ExecutionState::Done
                    | crate::durable_state::agent_lifecycle::ExecutionState::Cancelled
            ) {
                return Ok(());
            }
        }
        if self
            .owned
            .operational_history_store
            .load_prompt_settlement_event(room, agent.id(), &prompt_id)?
            .is_some()
        {
            return Ok(());
        }
        let attachment = self.ensure_agent_message_attachment(room, agent)?;
        let prompt = PromptQueueItem::new(
            prompt_id,
            attachment,
            agent.id(),
            task,
            PromptStatus::Queued,
        )
        .with_prompt_author(author_name);
        let mut prepared = crate::app::KernelPreparedPromptSubmission {
            session_id: room.to_string(),
            prompt,
            force_queue: false,
            refresh_projection: true,
        };
        prepared.prepare_task_prompt_identity(|| self.owned.session_store.reserve_prompt_id())?;
        let submission = self
            .submit_prepared_prompt(prepared)
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "agent.spawn",
                message: format!(
                    "agent {} was created, but its first prompt could not be admitted: {error}",
                    agent.agent_ref()
                ),
            })?;
        if let Some(dispatch) = submission.dispatch {
            if let PromptSubmissionOutcome::Started { prompt } = submission.outcome {
                self.start_active_turn_with_trace_id(
                    &dispatch.session_id,
                    &dispatch.agent_id,
                    prompt.id(),
                    &dispatch.provider_run_id,
                    "agent-spawn",
                );
            }
            self.spawn_prompt_dispatch(dispatch, self.provider_runtime_lanes.clone());
        }
        if let Some(dispatch) = submission.remote_dispatch {
            self.spawn_remote_prompt_dispatch(dispatch);
        }
        Ok(())
    }
}
