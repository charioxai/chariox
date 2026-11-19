//! A parent always follows its children: when a child's work ends, the parent
//! gets one urgent inbox event that steers it if it runs and wakes it if it is parked.
use super::*;
use crate::durable_state::agent_lifecycle::{
    self as ledger, AgentTaskExecution, ExecutionState, Operation,
};
use crate::provider::TurnOutcome;

impl KernelRuntimeState {
    /// Records `child_finished` for the parent of the agent whose task just ended and
    /// delivers it. A child with no live parent in its room tells nobody.
    pub(super) async fn announce_child_finished(
        &self,
        task: &AgentTaskExecution,
        failure: Option<&TurnOutcome>,
        public_answer: Option<&serde_json::Value>,
    ) -> Result<(), DaemonError> {
        if let Some(parent) = self
            .record_child_finished(task, failure, public_answer)
            .await?
        {
            Box::pin(self.deliver_agent_inbox(&task.room_id, &parent)).await?;
        }
        Ok(())
    }

    async fn record_child_finished(
        &self,
        task: &AgentTaskExecution,
        failure: Option<&TurnOutcome>,
        public_answer: Option<&serde_json::Value>,
    ) -> Result<Option<String>, DaemonError> {
        // The agent no longer holds decisions: they move up before anyone is told it finished.
        self.move_decisions_up(&task.room_id, &task.agent_id, Some(&task.task_id))
            .await?;
        let child = self.owned.agent_store.get_agent(&task.agent_id)?;
        let Some(parent_id) = child.parent_agent_id() else {
            return Ok(None);
        };
        let parent = match self.owned.agent_store.get_agent(parent_id) {
            Ok(parent) if parent.session_id() == task.room_id && !parent.archived() => parent,
            _ => return Ok(None),
        };
        let deadline_reached = self
            .owned
            .durable_state_store
            .agent_deadline(&task.room_id, &task.agent_id)?
            .is_some_and(|deadline| deadline.state == crate::local::AgentDeadlineState::Reached);
        let outcome = self.owned.finished_task_outcome(task, failure);
        // A blocked `chariox.agents.run` call takes the end in place of the inbox event.
        if self
            .owned
            .agent_run_waiters
            .claim(
                child.id(),
                super::agent_run_waiters::WorkerEnd {
                    task: task.clone(),
                    outcome: outcome.clone(),
                    deadline_reached,
                    public_answer: public_answer.cloned(),
                },
            )
            .await
        {
            return Ok(None);
        }
        let mut event = ledger::occurrence(
            &task.room_id,
            parent.id(),
            child.id(),
            &format!("child-finished-{}", task.task_id),
            "child_finished",
            serde_json::json!({
                "child_agent_id": child.id(),
                "child_alias": child.alias(),
                "child_task_id": task.task_id,
                "success": outcome.kind == crate::provider::TurnOutcomeKind::Completed,
                "outcome": outcome,
                "deadline_reached": deadline_reached,
                "resumes_at_ms": child.usage_limit_stop().and_then(|stop| match stop.state {
                    crate::agent::UsageLimitStopState::Armed { at_ms } => Some(at_ms),
                    _ => None,
                }),
                "public_answer": public_answer,
            }),
        );
        event.urgent = true;
        self.owned
            .durable_state_store
            .agent_lifecycle(Operation::Occur(event))?;
        Ok(Some(parent.id().to_owned()))
    }
}

impl KernelRuntimeState {
    /// A blocking waiter stopped with the old kernel. Reuse the exact answer
    /// already recorded for its task and let the inbox deliver it once.
    pub(super) async fn recover_child_completions(&self) -> Result<(), DaemonError> {
        let store = &self.owned.durable_state_store;
        for end in store.agent_undelivered_delegate_ends()? {
            let Some(task) = store.agent_task(&end.task)? else {
                continue;
            };
            if !self
                .owned
                .agent_store
                .get_agent(&task.agent_id)
                .is_ok_and(|child| child.parent_agent_id() == Some(end.parent.as_str()))
            {
                continue;
            }
            let failure = end
                .answer
                .as_ref()
                .and_then(|answer| answer.get("outcome"))
                .cloned()
                .and_then(|outcome| serde_json::from_value::<TurnOutcome>(outcome).ok());
            self.record_child_finished(&task, failure.as_ref(), end.answer.as_ref())
                .await?;
        }
        Ok(())
    }

    /// A restart ended the child's turn; its parent reads that as an interruption it can retry.
    pub(super) async fn announce_child_interrupted(
        &self,
        room: &str,
        agent: &str,
        prompt: &str,
        reason: &str,
    ) -> Result<(), DaemonError> {
        let Some(task) = self
            .owned
            .durable_state_store
            .agent_tasks(Some(room), Some(agent))?
            .into_iter()
            .rev()
            .find(|task| task.prompt_id == prompt)
        else {
            return Ok(());
        };
        let outcome = TurnOutcome::interrupted_by_restart(reason);
        self.announce_child_finished(&task, Some(&outcome), None)
            .await
    }
}

impl KernelRuntimeOwnedState {
    /// How a finished task's turn ended.
    pub(super) fn finished_task_outcome(
        &self,
        task: &AgentTaskExecution,
        failure: Option<&TurnOutcome>,
    ) -> TurnOutcome {
        if task.state == ExecutionState::Done {
            failure
                .filter(|outcome| outcome.is_error())
                .cloned()
                .unwrap_or_else(TurnOutcome::completed)
        } else {
            // A task that did not finish its work ended cancelled or failed, whatever its turn recorded.
            let mut outcome = failure
                .cloned()
                .or_else(|| self.recorded_turn_outcome(task))
                .filter(|outcome| outcome.kind != crate::provider::TurnOutcomeKind::Completed)
                .unwrap_or_else(TurnOutcome::cancelled);
            // The parent reads why it ended: an outcome with no words of its own says the task's.
            if outcome.reason.is_empty() {
                outcome.reason.clone_from(&task.reason);
            }
            outcome
        }
    }

    /// The outcome the child's last turn recorded in its history, if any.
    fn recorded_turn_outcome(&self, task: &AgentTaskExecution) -> Option<TurnOutcome> {
        self.operational_history_store
            .load_prompt_settlement_event(&task.room_id, &task.agent_id, &task.prompt_id)
            .ok()
            .flatten()
            .and_then(|settlement| {
                settlement
                    .metadata
                    .get(crate::history::TURN_OUTCOME_METADATA_KEY)
                    .and_then(|outcome| serde_json::from_value(outcome.clone()).ok())
            })
    }
}
