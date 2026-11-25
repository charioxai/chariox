//! A blocked task is status, never a card. The agent's row shows it blocked with its reason, what
//! unblocks it and the command; the parent agent is told with one `child_blocked` event that
//! carries the same words. `agent resume <agent>` resumes the task and `agent resume cancel
//! <agent>` cancels it, for the person (CLI) and for the agents above (agent command) alike.
//! A card is raised only for what no command can give: the agent's own question, a secret, a
//! login, a payment or a consent each have their own kind of card.
use super::agent_task_owner_resolution::OwnerTaskAction;
use super::*;
use crate::agent::authority_limit::ancestors;
use crate::durable_state::agent_lifecycle::{
    self as ledger, AgentTaskExecution, ExecutionState, Operation, TaskOwnerAction, TaskUnblock,
};
use crate::local::UsageLimitStopAction;

pub(crate) const CHILD_BLOCKED_EVENT: &str = "child_blocked";

fn refused(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "blocked task",
        message: message.into(),
    }
}

impl KernelRuntimeOwnedState {
    /// What unblocks `task`, and the commands that settle it.
    pub(super) fn task_unblock(
        &self,
        agent: &crate::agent::AgentInstance,
        task: &AgentTaskExecution,
    ) -> TaskUnblock {
        let reference = agent.alias().unwrap_or(agent.agent_ref());
        let steps = if crate::provider::switch_turn::needs_fresh_agent(&task.reason) {
            "Start a fresh agent to continue with the brief.".to_owned()
        } else {
            match self.task_settle_steps(task) {
                Ok(steps) if !steps.is_empty() => format!("Open: {}.", steps.join("; ")),
                Ok(_) => "Resume it to continue, or cancel it.".to_owned(),
                Err(error) => {
                    let reason =
                        crate::secret_redaction::redact_secrets(&error.to_string()).into_owned();
                    format!("Recovery instructions could not be read: {reason}. Cancel this task or restore its authoritative receipts before resuming.")
                }
            }
        };
        TaskUnblock {
            steps,
            resume_command: format!("agent resume {reference}"),
            cancel_command: format!("agent resume cancel {reference}"),
        }
    }

    /// The blocked task of `agent_id` that only a command settles: not the agent's own question
    /// (the person answers that), not a usage stop (it resumes at its reset).
    pub(super) fn blocked_task_for_command(
        &self,
        room: &str,
        agent_id: &str,
    ) -> Result<Option<AgentTaskExecution>, DaemonError> {
        Ok(self
            .durable_state_store
            .agent_unfinished_tasks(room, Some(agent_id))?
            .into_iter()
            .rev()
            .find(|task| {
                task.state == ExecutionState::Blocked
                    && task.owner_action != Some(TaskOwnerAction::Question)
                    && self.blocked_task_is_open(task).unwrap_or(false)
            }))
    }
}

impl KernelRuntimeState {
    /// Tells the agent that started the blocked task's agent. A root agent has no parent: its row is
    /// the only place, and the person acts from there.
    pub(super) async fn announce_blocked_task(
        &self,
        task: &AgentTaskExecution,
    ) -> Result<(), DaemonError> {
        let owned = &self.owned;
        let Ok(child) = owned.agent_store.get_agent(&task.agent_id) else {
            return Ok(());
        };
        let Some(parent) = child
            .parent_agent_id()
            .and_then(|id| owned.agent_store.get_agent(id).ok())
            .filter(|parent| !parent.archived())
        else {
            return Ok(());
        };
        let unblock = owned.task_unblock(&child, task);
        let reason = crate::terminal::public_task_failure_reason(&task.reason);
        let mut event = ledger::occurrence(
            parent.session_id(),
            parent.id(),
            child.id(),
            &format!("child-blocked-{}-{}", task.task_id, task.blocked_revision),
            CHILD_BLOCKED_EVENT,
            serde_json::json!({
                "child_agent_id": child.id(),
                "child_alias": child.alias(),
                "task_id": task.task_id,
                "reason": reason,
                "unblock": unblock.steps,
                "resume_command": unblock.resume_command,
                "cancel_command": unblock.cancel_command,
                "needs_answer": false,
            }),
        );
        event.urgent = true;
        owned
            .durable_state_store
            .agent_lifecycle(Operation::Occur(event))?;
        Box::pin(self.deliver_agent_inbox(parent.session_id(), parent.id())).await
    }

    /// `agent resume` / `agent resume cancel` on an agent that has no usage stop: settles its
    /// blocked task. The person, and any agent above the target, may; the authority is the one that
    /// answered the card before.
    pub(super) async fn resolve_blocked_task_command(
        &self,
        agent: &crate::agent::AgentInstance,
        caller_agent_id: Option<&str>,
        action: UsageLimitStopAction,
    ) -> Result<(), DaemonError> {
        let owned = &self.owned;
        if let Some(caller) = caller_agent_id {
            let agents = owned.agent_store.list_agents();
            if !ancestors(agent, &agents)
                .iter()
                .any(|ancestor| ancestor.id() == caller)
            {
                return Err(refused("only an agent above this one may settle its task"));
            }
        }
        let resume = match action {
            UsageLimitStopAction::ResumeWhenBack => true,
            UsageLimitStopAction::Cancel | UsageLimitStopAction::NotNow => false,
            UsageLimitStopAction::ContinueOn { .. } => {
                return Err(refused("this agent is not stopped by a usage limit"))
            }
        };
        let Some(task) = owned.blocked_task_for_command(agent.session_id(), agent.id())? else {
            return Err(refused(
                "this agent has no blocked task and is not stopped by a usage limit",
            ));
        };
        let action = match (
            resume,
            crate::provider::switch_turn::needs_fresh_agent(&task.reason),
        ) {
            (true, true) => OwnerTaskAction::StartFreshAgent,
            (true, false) => OwnerTaskAction::Resume,
            (false, _) => OwnerTaskAction::Cancel,
        };
        self.try_resolve_agent_task_owner_action(&task, action)
            .await?;
        self.retract_stale_task_owner_interactions(agent.session_id())
            .await;
        Ok(())
    }
}
