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
        // The agent no longer holds decisions: they move up before anyone is told it finished.
        self.move_decisions_up(&task.room_id, &task.agent_id)
            .await?;
        let child = self.owned.agent_store.get_agent(&task.agent_id)?;
        let Some(parent_id) = child.parent_agent_id() else {
            return Ok(());
        };
        let parent = match self.owned.agent_store.get_agent(parent_id) {
            Ok(parent) if parent.session_id() == task.room_id && !parent.archived() => parent,
            _ => return Ok(()),
        };
        let deadline_reached = self
            .owned
            .durable_state_store
            .agent_deadline(&task.room_id, &task.agent_id)?
            .is_some_and(|deadline| deadline.state == crate::local::AgentDeadlineState::Reached);
        let outcome = self.owned.finished_task_outcome(task, failure);
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
                "success": task.state == ExecutionState::Done,
                "outcome": outcome,
                "deadline_reached": deadline_reached,
                "public_answer": public_answer,
            }),
        );
        event.urgent = true;
        self.owned
            .durable_state_store
            .agent_lifecycle(Operation::Occur(event))?;
        Box::pin(self.deliver_agent_inbox(&task.room_id, parent.id())).await
    }
}

impl KernelRuntimeState {
    /// Ends the sub-agents whose `child_finished` receipts the parent just handled.
    /// Only a child the parent spawned (it carries the spawn key) and whose latest
    /// task is the one reported goes: an agent a person moved under the parent, or
    /// one given new work since, stays.
    pub(super) async fn end_handled_children(
        &self,
        room: &str,
        parent: &str,
        receipts: &[serde_json::Value],
    ) -> Result<(), DaemonError> {
        for receipt in receipts {
            let (Some(child_id), Some(task_id)) = (
                receipt["child_agent_id"].as_str(),
                receipt["child_task_id"].as_str(),
            ) else {
                continue;
            };
            let Ok(child) = self.owned.agent_store.get_agent(child_id) else {
                continue;
            };
            if child.session_id() != room
                || child.parent_agent_id() != Some(parent)
                || child.spawn_key().is_none()
            {
                continue;
            }
            let reported_is_latest = self
                .owned
                .durable_state_store
                .agent_tasks(Some(room), Some(child_id))?
                .last()
                .is_some_and(|latest| latest.task_id == task_id);
            if reported_is_latest {
                self.destroy_one_agent(child_id, child.owner_user_id(), true)
                    .await?;
                self.destroy_sub_agents_of(child_id).await;
            }
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
            TurnOutcome::completed()
        } else {
            // A task that did not finish its work ended cancelled or failed, whatever its turn recorded.
            failure
                .cloned()
                .or_else(|| self.recorded_turn_outcome(task))
                .filter(|outcome| outcome.kind != crate::provider::TurnOutcomeKind::Completed)
                .unwrap_or_else(TurnOutcome::cancelled)
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
