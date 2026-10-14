//! MP-08 / MP-09 / MP-10 / MP-11: workflow supervision follows durable tasks.
use super::*;
use crate::durable_state::agent_lifecycle::{
    self as ledger, AgentTaskExecution, ExecutionState, Operation, Outcome,
};

impl KernelRuntimeOwnedState {
    pub(super) fn bind_agent_workflow_task(
        &self,
        task: &AgentTaskExecution,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<(), DaemonError> {
        if let (Some(run), Some(node)) = (prompt.workflow_run_id(), prompt.workflow_node_run_id()) {
            self.validate_agent_workflow_task(task, run, node)?;
            self.durable_state_store
                .agent_lifecycle(Operation::BindWorkflow {
                    task: task.task_id.clone(),
                    prompt: prompt.id().into(),
                    run: run.into(),
                    node: node.into(),
                })?;
        }
        Ok(())
    }

    fn validate_agent_workflow_task(
        &self,
        task: &AgentTaskExecution,
        run: &str,
        node: &str,
    ) -> Result<(), DaemonError> {
        let session = self.session_store.get_session(&task.room_id)?;
        if session.workflow_runs().iter().any(|r| {
            r.id() == run
                && r.node_runs()
                    .iter()
                    .any(|n| n.id() == node && n.agent_id() == task.agent_id)
        }) {
            Ok(())
        } else {
            Err(ledger::error("workflow task is outside its agent and room"))
        }
    }

    pub(super) fn agent_workflow_task_context(
        &self,
        task: &AgentTaskExecution,
    ) -> Result<Option<(String, String)>, DaemonError> {
        let binding = self
            .durable_state_store
            .agent_task_workflow(&task.task_id)?;
        if let Some((run, node)) = &binding {
            self.validate_agent_workflow_task(task, run, node)?;
        }
        Ok(binding)
    }

    pub(super) fn supervised_workflow_targets(
        &self,
        room: &str,
    ) -> Result<BTreeSet<(String, String)>, DaemonError> {
        let mut targets = BTreeSet::new();
        for task in self.durable_state_store.agent_tasks(Some(room), None)? {
            if !matches!(task.state, ExecutionState::Done | ExecutionState::Cancelled) {
                if let Some(binding) = self.agent_workflow_task_context(&task)? {
                    targets.insert(binding);
                }
            }
        }
        Ok(targets)
    }

    pub(super) fn workflow_agent_tasks_unsettled(
        &self,
        room: &str,
        run: &str,
    ) -> Result<bool, DaemonError> {
        for task in self.durable_state_store.agent_tasks(Some(room), None)? {
            if self
                .durable_state_store
                .agent_task_workflow(&task.task_id)?
                .is_none_or(|(id, _)| id != run)
            {
                continue;
            }
            if !matches!(task.state, ExecutionState::Done | ExecutionState::Cancelled)
                || task
                    .obligations
                    .iter()
                    .any(|o| matches!(o.status.as_str(), "open" | "failed" | "settling"))
                || self.agent_task_resources_unsettled(&task)?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

impl KernelRuntimeState {
    pub(super) async fn cancel_workflow_agent_tasks(
        &self,
        room: &str,
        run: &str,
    ) -> Result<(), DaemonError> {
        let owner = self
            .owned
            .session_store
            .get_session(room)?
            .owner_user_id()
            .to_string();
        for task in self
            .owned
            .durable_state_store
            .agent_tasks(Some(room), None)?
        {
            if self
                .owned
                .durable_state_store
                .agent_task_workflow(&task.task_id)?
                .is_some_and(|(id, _)| id == run)
            {
                let Outcome::Task(cancelled) =
                    self.owned
                        .durable_state_store
                        .agent_lifecycle(Operation::CancelTask {
                            task: task.task_id,
                            owner: owner.clone(),
                            revision: task.revision,
                        })?
                else {
                    unreachable!()
                };
                Box::pin(self.cancel_agent_task_resources(&cancelled)).await?;
            }
        }
        Ok(())
    }
}
