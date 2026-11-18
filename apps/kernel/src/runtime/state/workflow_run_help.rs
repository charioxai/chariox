//! A workflow run never raises a card. When a step's agent cannot go on without someone (its
//! task blocked, or it asked for a permission, an answer or the browser) the step ends: the run
//! fails with `StepNeedsHelp` and the agent's own reason, and the run's responsible reads it.
use tokio::sync::oneshot;

use super::*;
use crate::durable_state::agent_lifecycle::{
    AgentTaskExecution, ExecutionState, Operation, Outcome,
};

/// Longest reason a step carries; the same bound a run outcome's detail has.
const REASON_CHARS: usize = crate::session::OUTCOME_DETAIL_CHARS;

pub(super) fn is_run_step(prompt: &crate::session::PromptQueueItem) -> bool {
    prompt.workflow_run_id().is_some() && prompt.workflow_node_run_id().is_some()
}

/// The first line of an agent's own words, redacted and bounded.
fn short_reason(text: &str) -> String {
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    crate::secret_redaction::redact_secrets(line.trim())
        .chars()
        .take(REASON_CHARS)
        .collect()
}

/// What the interaction asked for, as a short typed reason.
fn interaction_reason(interaction: &crate::session::RuntimeInteraction) -> String {
    use crate::session::RuntimeInteractionKind;
    let subject = interaction
        .title()
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| interaction.message());
    match interaction.kind() {
        RuntimeInteractionKind::Permission => {
            format!("needs permission: {}", short_reason(subject))
        }
        RuntimeInteractionKind::Choice => {
            format!("needs an answer: {}", short_reason(interaction.message()))
        }
    }
}

impl KernelRuntimeOwnedState {
    /// The prompt of `agent_id`'s running turn when that turn is a workflow step.
    pub(super) fn run_prompt_of_agent_turn(
        &self,
        room_id: &str,
        agent_id: &str,
    ) -> Option<crate::session::PromptQueueItem> {
        let session = self.session_store.get_session(room_id).ok()?;
        self.prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .filter(is_run_step)
    }

    /// A step whose provider failed blocked its task; the failed run already says why, so the
    /// task ends as the agent's own instead of waiting for a decision nobody will be asked.
    pub(super) fn end_blocked_task_of_failed_step(
        &self,
        room_id: &str,
        agent_id: &str,
        prompt_id: &str,
    ) -> Result<(), DaemonError> {
        let owner = self
            .session_store
            .get_session(room_id)?
            .owner_user_id()
            .to_owned();
        for task in self
            .durable_state_store
            .agent_tasks(Some(room_id), Some(agent_id))?
            .into_iter()
            .filter(|task| task.prompt_id == prompt_id && task.state == ExecutionState::Blocked)
        {
            self.durable_state_store
                .agent_lifecycle(Operation::CancelTaskByAgent {
                    task: task.task_id,
                    owner: owner.clone(),
                    revision: task.revision,
                })?;
        }
        Ok(())
    }

    /// The prompt of `task` when it is a workflow step.
    pub(super) fn run_prompt_of_task(
        &self,
        task: &AgentTaskExecution,
    ) -> Option<crate::session::PromptQueueItem> {
        let session = self.session_store.get_session(&task.room_id).ok()?;
        let of_task = |prompt: &crate::session::PromptQueueItem| {
            prompt.id() == task.prompt_id || task.pending_prompt_id.as_deref() == Some(prompt.id())
        };
        self.prompt_state_owner
            .active_prompt_for_agent(&session, &task.agent_id)
            .into_iter()
            .chain(
                session
                    .queued_prompts_for_agent(&task.agent_id)
                    .into_iter()
                    .flatten()
                    .cloned(),
            )
            .find(|prompt| of_task(prompt) && is_run_step(prompt))
    }
}

impl KernelRuntimeState {
    /// A blocked task of a workflow step ends the step instead of raising a card or a Blocked
    /// row: the run fails with the task's reason and the task is cancelled as the agent's own.
    /// Answers whether the task belongs to a run.
    pub(super) async fn end_run_step_of_blocked_task(
        &self,
        task: &AgentTaskExecution,
    ) -> Result<bool, DaemonError> {
        if task.state != ExecutionState::Blocked {
            return Ok(false);
        }
        let Some(prompt) = self.owned.run_prompt_of_task(task) else {
            return Ok(false);
        };
        let dispatches = self.owned.workflow_fail_prompt_needing_help(
            &task.room_id,
            &prompt,
            &short_reason(&task.reason),
        )?;
        self.spawn_workflow_prompt_dispatches(dispatches);
        // The task as it stands now: a step whose task already moved on has nothing to cancel.
        let current = self
            .owned
            .durable_state_store
            .agent_task(&task.task_id)?
            .filter(|current| current.state == ExecutionState::Blocked);
        if let Some(current) = current {
            let owner = self
                .owned
                .session_store
                .get_session(&current.room_id)?
                .owner_user_id()
                .to_owned();
            if let Outcome::Task(cancelled) =
                self.owned
                    .durable_state_store
                    .agent_lifecycle(Operation::CancelTaskByAgent {
                        task: current.task_id.clone(),
                        owner,
                        revision: current.revision,
                    })?
            {
                // The run has ended; stopping what the task still holds is best effort.
                if let Err(error) = self.cancel_agent_task_resources(&cancelled).await {
                    tracing::warn!(%error, "the resources of a step that needs help were not all stopped");
                }
            }
        }
        self.retract_stale_task_owner_interactions(&task.room_id)
            .await;
        Ok(true)
    }

    /// A step's agent asked for a permission, an answer or the browser: the provider is refused
    /// at once, the step ends and its turn stops. Answers `None` for an agent outside a run.
    pub(super) async fn refuse_run_step_interaction(
        &self,
        session_id: &str,
        agent_id: &str,
        interaction: &crate::session::RuntimeInteraction,
    ) -> Result<Option<oneshot::Receiver<PendingInteractionResolution>>, DaemonError> {
        let Some(prompt) = self.owned.run_prompt_of_agent_turn(session_id, agent_id) else {
            return Ok(None);
        };
        let dispatches = self.owned.workflow_fail_prompt_needing_help(
            session_id,
            &prompt,
            &interaction_reason(interaction),
        )?;
        self.spawn_workflow_prompt_dispatches(dispatches);
        // The run has ended and the provider is refused; stopping its turn is best effort.
        match self.owned.cancel_local_prompt_if_matches(
            session_id,
            agent_id,
            prompt.source_attachment_id(),
            Some(prompt.id()),
        ) {
            Ok(Some(cancellation)) => {
                if let Some(dispatch) = cancellation.dispatch {
                    self.spawn_prompt_abort(dispatch, self.provider_runtime_lanes.clone());
                }
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(%error, "the turn of a step that needs help was not stopped");
            }
        }
        let (answer, receiver) = oneshot::channel();
        let _ = answer.send(PendingInteractionResolution {
            status: "timed_out",
            choice_id: None,
            reply: None,
        });
        Ok(Some(receiver))
    }
}
