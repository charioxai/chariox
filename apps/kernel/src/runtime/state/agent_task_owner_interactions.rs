//! MP-08 / MP-09 / MP-10 / MP-11 A12-S02: shared owner decision projection.
use super::*;
use crate::durable_state::agent_lifecycle::{AgentTaskExecution, ExecutionState};

fn task_needs_owner_interaction(task: &AgentTaskExecution) -> bool {
    task.state == ExecutionState::Blocked && !task.goal.as_ref().is_some_and(|goal| goal.paused)
}

impl KernelRuntimeState {
    /// A task that left Blocked (for example cancelled with its parent) or
    /// re-blocked at a later revision withdraws its older owner decision.
    pub(super) async fn retract_stale_task_owner_interactions(
        &self,
        room: &str,
        tasks: &[AgentTaskExecution],
    ) {
        let Ok(session) = self.owned.session_store.get_session(room) else {
            return;
        };
        let stale = session
            .active_interactions()
            .iter()
            .map(|i| i.id())
            .filter(|id| {
                let Some(rest) = id.strip_prefix("task-blocked-") else {
                    return false;
                };
                tasks.iter().any(|t| {
                    rest.strip_prefix(t.task_id.as_str())
                        .and_then(|r| r.strip_prefix('-'))
                        .and_then(|r| r.parse::<u64>().ok())
                        .is_some_and(|revision| {
                            !task_needs_owner_interaction(t) || revision != t.blocked_revision
                        })
                })
            })
            .map(str::to_owned)
            .collect::<Vec<_>>();
        for id in stale {
            let _ = self.timeout_runtime_interaction(room, &id).await;
        }
    }
    pub(super) async fn ensure_task_owner_interaction(
        &self,
        task: AgentTaskExecution,
    ) -> Result<(), DaemonError> {
        if self.defer_task_for_human_control(&task).await {
            return Ok(());
        }
        if !task_needs_owner_interaction(&task) {
            self.retract_stale_task_owner_interactions(&task.room_id, std::slice::from_ref(&task))
                .await;
            return Ok(());
        }
        let id = format!("task-blocked-{}-{}", task.task_id, task.blocked_revision);
        let session = self.owned.session_store.get_session(&task.room_id)?;
        let message = crate::terminal::public_task_failure_reason(&task.reason);
        if let Some(existing) = session.active_interactions().iter().find(|i| i.id() == id) {
            if existing.message() != message {
                let mutation = self.owned.begin_managed_activity_mutation();
                let mut sessions = self.owned.session_store.write();
                let mut latest = sessions.get_session(&task.room_id)?;
                if let Some(current) = latest.remove_active_interaction(&id) {
                    latest.add_active_interaction(current.with_message(message));
                    sessions.restore_session(latest);
                    mutation.record();
                }
                drop(sessions);
                self.owned.session_snapshot(&task.room_id)?;
                self.owned
                    .terminal_stream
                    .notify_terminal_projection_change(&task.room_id);
            }
            return Ok(());
        }
        let interaction = crate::session::RuntimeInteraction::for_kernel_operation(
            &id,
            &id,
            "Agent needs your action",
            &message,
            vec![
                crate::session::RuntimeInteractionChoice::new("cancel", "Cancel", "cancel", None),
                crate::session::RuntimeInteractionChoice::new("resume", "Resume", "resume", None),
            ],
        )
        .with_timeout_sec(900);

        let rx = self
            .create_kernel_operation_interaction(
                &task.room_id,
                session.owner_user_id(),
                interaction,
            )
            .await?;
        let state = self.clone();
        tokio::spawn(async move {
            if let Ok(answer) = rx.await {
                if answer.status == "timed_out" {
                    return;
                }
                let resume = answer.choice_id.as_deref() == Some("resume");
                state.resolve_agent_task_owner_action(&task, resume).await;
            }
        });
        Ok(())
    }

    /// A new owner turn replaces a failed restart attempt, not a goal or owned work.
    pub(super) async fn retire_superseded_restart_task(
        &self,
        room: &str,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<bool, DaemonError> {
        if prompt.prompt_origin() != crate::session::PromptOrigin::Chariox
            || prompt.workflow_run_id().is_some()
            || prompt
                .durable_operation_fingerprint()
                .is_some_and(|value| value.starts_with("task:"))
        {
            return Ok(false);
        }
        let session = self.owned.session_store.get_session(room)?;
        let Ok(attachment) = self
            .owned
            .attachment_store
            .get_attachment(prompt.source_attachment_id())
        else {
            return Ok(false);
        };
        if attachment.owner_user_id() != session.owner_user_id()
            || !matches!(
                attachment.capability_level(),
                crate::attachment::ClientCapabilityLevel::FullTerminal
                    | crate::attachment::ClientCapabilityLevel::InteractiveStructured
            )
        {
            return Ok(false);
        }
        let Some(previous) = self
            .owned
            .completed_git_turn_snapshots
            .latest_projection_for_agent(room, prompt.target_agent_id())
        else {
            return Ok(false);
        };
        if previous.outcome.as_ref().is_none_or(|outcome| {
            outcome.kind != crate::provider::TurnOutcomeKind::InterruptedByRestart
        }) || previous.prompt_id == prompt.id()
        {
            return Ok(false);
        }
        let task = self
            .owned
            .durable_state_store
            .agent_tasks(Some(room), Some(prompt.target_agent_id()))?
            .into_iter()
            .find(|task| {
                task.prompt_id == previous.prompt_id
                    && task.owner_user_id == session.owner_user_id()
                    && task.state == ExecutionState::Blocked
                    && task.goal.is_none()
                    && task.obligations.is_empty()
                    && task.wait.is_none()
                    && task.pending_prompt_id.is_none()
            });
        let Some(task) = task else { return Ok(false) };
        let crate::durable_state::agent_lifecycle::Outcome::Task(retired) =
            self.owned.durable_state_store.agent_lifecycle(
                crate::durable_state::agent_lifecycle::Operation::RetireRestartFailure {
                    task: task.task_id,
                    revision: task.blocked_revision,
                    now: crate::session::unix_epoch_ms(),
                },
            )?
        else {
            unreachable!()
        };
        if retired.state == ExecutionState::Blocked {
            return Ok(false);
        }
        self.retract_stale_task_owner_interactions(room, std::slice::from_ref(&retired))
            .await;
        Ok(true)
    }
}
