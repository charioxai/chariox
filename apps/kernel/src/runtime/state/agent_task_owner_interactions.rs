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
        if self.owned.goal_has_native_approval(&task) {
            return Ok(());
        }
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
        let state = self.as_kernel();
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
}
