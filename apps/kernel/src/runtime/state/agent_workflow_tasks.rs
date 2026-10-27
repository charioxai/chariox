//! MP-08 / MP-09 / MP-10 / MP-11: workflow supervision follows durable tasks.
use super::*;
use crate::durable_state::agent_lifecycle::{
    self as ledger, AgentTaskExecution, ExecutionState, Operation, Outcome,
};

impl KernelRuntimeOwnedState {
    // MP-08 / MP-10 / MP-11 A12-S06: a terminal source is archived, not lost.
    pub(super) fn agent_workflow_source(
        &self,
        session: &crate::session::RuntimeSession,
        run: &str,
    ) -> Result<Option<crate::session::WorkflowRun>, DaemonError> {
        let active = self
            .session_store
            .read()
            .resolve_workflow_run_ref(session.id(), run)
            .ok();
        match active {
            Some(run) => Ok(Some(run)),
            None => self.durable_state_store.resolve_workflow_run(
                session.host_daemon_id(),
                session.id(),
                run,
            ),
        }
    }

    /// A01-S01: a run an agent starts while its workflow is busy is admitted
    /// as a queued prompt and its obligation names that queue item. It stays
    /// open while queued, then follows the run promoted from it.
    pub(super) fn agent_workflow_obligation_outcome(
        &self,
        session: &crate::session::RuntimeSession,
        source: &str,
    ) -> Result<Option<bool>, DaemonError> {
        let queued = session
            .workflow_queued_prompts()
            .iter()
            .find(|queued| queued.id() == source);
        let run = match queued {
            Some(queued) => match queued.workflow_run_id() {
                Some(run) => self.agent_workflow_source(session, run)?,
                None if queued.status()
                    == crate::session::WorkflowQueuedPromptStatus::Cancelled =>
                {
                    return Ok(Some(false))
                }
                None => return Ok(None),
            },
            None => self.agent_workflow_source(session, source)?.or_else(|| {
                session
                    .workflow_runs()
                    .iter()
                    .find(|run| run.queue_item_id() == Some(source))
                    .cloned()
            }),
        };
        Ok(run.map_or(Some(false), |run| match run.status() {
            crate::session::WorkflowRunStatus::Completed => Some(true),
            crate::session::WorkflowRunStatus::Failed
            | crate::session::WorkflowRunStatus::Stopped => Some(false),
            _ => None,
        }))
    }

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
                    .any(ledger::AgentObligation::requires_resolution)
                || self.agent_task_resources_unsettled(&task)?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

impl KernelRuntimeState {
    // MP-08 / MP-10 / MP-11: terminal runs are archived out of the hot cache.
    // Reconcile durable failures even after the parent handled its run receipt.
    pub(super) async fn reconcile_failed_workflow_tasks(
        &self,
        task: &AgentTaskExecution,
    ) -> Result<(), DaemonError> {
        if !task
            .obligations
            .iter()
            .any(|o| o.kind == "delegate" && o.status == "open")
        {
            return Ok(());
        }
        let session = self.owned.session_store.get_session(&task.room_id)?;
        for receipt in task
            .obligations
            .iter()
            .filter(|o| o.tracks_workflow_run() && o.dispatch_state == "accepted")
        {
            let Some(id) = receipt.resource_id.as_deref() else {
                continue;
            };
            let Some(run) = self.owned.durable_state_store.resolve_workflow_run(
                session.host_daemon_id(),
                &task.room_id,
                id,
            )?
            else {
                continue;
            };
            if !matches!(
                run.status(),
                crate::session::WorkflowRunStatus::Failed
                    | crate::session::WorkflowRunStatus::Stopped
            ) {
                continue;
            }
            self.cancel_workflow_agent_tasks(&task.room_id, id).await?;
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::ReconcileFailedWorkflow {
                    owner_id: session.host_daemon_id().into(),
                    room: task.room_id.clone(),
                    run: id.into(),
                })?;
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::SourceOutcome {
                    room: task.room_id.clone(),
                    source: id.into(),
                    occurrence: format!("workflow-failure-{id}"),
                    success: false,
                    now: crate::session::unix_epoch_ms(),
                    public_answer: None,
                })?;
        }
        Ok(())
    }
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
            // MP-08 / MP-10 / MP-11: successful settlement has already
            // published its terminal receipt. Cancelling it would replay that
            // same occurrence as failure and abort the lifecycle sweep.
            if task.state != ExecutionState::Done
                && self
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
