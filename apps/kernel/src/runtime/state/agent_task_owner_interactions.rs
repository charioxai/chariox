//! MP-08 / MP-09 / MP-10 / MP-11 A12-S02: shared owner decision projection.
use super::*;
use crate::durable_state::agent_lifecycle::{AgentTaskExecution, ExecutionState};

fn task_needs_owner_interaction(task: &AgentTaskExecution) -> bool {
    task.state == ExecutionState::Blocked && !task.goal.as_ref().is_some_and(|goal| goal.paused)
}

/// The decision a blocked task raises: Resume or Cancel. It belongs to the owner but not to their terminal,
/// so a holder or an agent driving the room may answer it like any routine question.
pub(super) const BLOCKED_TASK_DECISION_PREFIX: &str = "task-blocked-";

pub(super) fn is_blocked_task_decision(interaction_id: &str) -> bool {
    interaction_id.starts_with(BLOCKED_TASK_DECISION_PREFIX)
}

impl KernelRuntimeOwnedState {
    /// Admission cannot wait for the asynchronous task pump to release ended decisions.
    pub(super) fn sweep_stale_task_owner_interactions(&self) -> Result<(), DaemonError> {
        let rooms = self
            .pending_interactions
            .write()
            .iter()
            .filter(|(id, pending)| {
                is_blocked_task_decision(id) && pending.belongs_to(&self.session_store)
            })
            .map(|(_, pending)| pending.session_id.clone())
            .collect::<BTreeSet<_>>();
        if rooms.is_empty() {
            return Ok(());
        }
        for room in rooms {
            self.retract_stale_task_owner_interactions(&room)?;
        }
        Ok(())
    }

    /// A task that left Blocked (for example cancelled with its parent) or
    /// re-blocked at a later revision withdraws its older owner decision.
    pub(super) fn retract_stale_task_owner_interactions(
        &self,
        room: &str,
    ) -> Result<(), DaemonError> {
        let Ok(session) = self.session_store.get_session(room) else {
            return Ok(());
        };
        let tasks = self.durable_state_store.agent_tasks(Some(room), None)?;
        let stale = session
            .active_interactions()
            .iter()
            .map(|i| i.id())
            .filter(|id| {
                let Some(rest) = id.strip_prefix(BLOCKED_TASK_DECISION_PREFIX) else {
                    return false;
                };
                let Some((task_id, revision)) = rest.rsplit_once('-') else {
                    return true;
                };
                let Ok(revision) = revision.parse::<u64>() else {
                    return true;
                };
                tasks
                    .iter()
                    .find(|task| task.task_id == task_id)
                    .is_none_or(|task| {
                        !task_needs_owner_interaction(task) || revision != task.blocked_revision
                    })
            })
            .map(str::to_owned)
            .collect::<Vec<_>>();
        for id in stale {
            self.timeout_runtime_interaction(room, &id)?;
            tracing::debug!(interaction_id=%id, room_id=room, "resolved task owner decision withdrawn");
        }
        Ok(())
    }
}

impl KernelRuntimeState {
    pub(super) async fn retract_stale_task_owner_interactions(&self, room: &str) {
        if let Err(error) = self.owned.retract_stale_task_owner_interactions(room) {
            tracing::warn!(%error, room_id=room, "task owner decision withdrawal failed; interaction admission will retry");
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
            self.retract_stale_task_owner_interactions(&task.room_id)
                .await;
            return Ok(());
        }
        let id = format!(
            "{BLOCKED_TASK_DECISION_PREFIX}{}-{}",
            task.task_id, task.blocked_revision
        );
        let session = self.owned.session_store.get_session(&task.room_id)?;
        let browser_access = task.owner_action
            == Some(crate::durable_state::agent_lifecycle::TaskOwnerAction::BrowserAccess)
            && self
                .owned
                .agent_store
                .get_agent(&task.agent_id)
                .is_ok_and(|agent| agent.remote_execution().is_none());
        let message = if browser_access {
            "Browser access ended. Allow this agent to use your browser tabs and continue?"
                .to_owned()
        } else {
            let reason = crate::terminal::public_task_failure_reason(&task.reason);
            // The card shows in the room agent's pane, so it says which agent needs the person.
            match self.owned.agent_store.get_agent(&task.agent_id) {
                Ok(agent) => format!("{}: {reason}", agent.alias().unwrap_or(agent.agent_ref())),
                Err(_) => reason,
            }
        };
        if let Some(existing) = session.active_interactions().iter().find(|i| i.id() == id) {
            let live = self
                .owned
                .pending_interactions
                .write()
                .get(&id)
                .is_some_and(|pending| {
                    pending.belongs_to(&self.owned.session_store)
                        && pending
                            .responder
                            .lock()
                            .expect("interaction responder")
                            .as_ref()
                            .is_some_and(|sender| !sender.is_closed())
                });
            if live {
                if existing.message() != message
                    || existing.related_agent_id() != Some(task.agent_id.as_str())
                {
                    let mutation = self.owned.begin_managed_activity_mutation();
                    let mut sessions = self.owned.session_store.write();
                    let mut latest = sessions.get_session(&task.room_id)?;
                    if let Some(current) = latest.remove_active_interaction(&id) {
                        latest.add_active_interaction(
                            current
                                .with_message(message)
                                .with_related_agent_id(task.agent_id.clone()),
                        );
                        sessions.restore_session(latest);
                    }
                    drop(sessions);
                    mutation.record();
                    self.owned.session_snapshot(&task.room_id)?;
                    self.owned
                        .terminal_stream
                        .notify_terminal_projection_change(&task.room_id);
                }
                self.owned.emit_task_owner_notification(&task, &id)?;
                return Ok(());
            }
            // Hot reload retains the task's decision, but an in-memory reply channel must be reattached.
            self.owned.timeout_runtime_interaction(&task.room_id, &id)?;
        }
        let interaction = crate::session::RuntimeInteraction::for_kernel_operation(
            &id,
            &id,
            if browser_access {
                "Allow browser access"
            } else {
                "Agent needs your action"
            },
            &message,
            vec![
                crate::session::RuntimeInteractionChoice::new("cancel", "Cancel", "cancel", None),
                crate::session::RuntimeInteractionChoice::new(
                    if browser_access {
                        "allow_browser"
                    } else {
                        "resume"
                    },
                    if browser_access {
                        "Allow browser"
                    } else {
                        "Resume"
                    },
                    "resume",
                    None,
                ),
            ],
        )
        .without_timeout()
        .with_related_agent_id(task.agent_id.clone());

        let rx = self
            .create_kernel_operation_interaction(
                &task.room_id,
                session.owner_user_id(),
                interaction,
            )
            .await?;
        let notification = self.owned.emit_task_owner_notification(&task, &id);
        let state = self.as_kernel();
        tokio::spawn(async move {
            if let Ok(answer) = rx.await {
                if answer.status == "timed_out" {
                    return;
                }
                use super::agent_task_owner_resolution::OwnerTaskAction;
                let action = match answer.choice_id.as_deref() {
                    Some("allow_browser") => OwnerTaskAction::AllowBrowser,
                    Some("resume") => OwnerTaskAction::Resume,
                    _ => OwnerTaskAction::Cancel,
                };
                state.resolve_agent_task_owner_action(&task, action).await;
            }
        });
        notification?;
        Ok(())
    }

    /// A later turn of an agent replaces its interrupted attempts (a restart, a refused or
    /// unacknowledged dispatch), not a goal or owned work. Every such attempt still waiting on the
    /// owner is cancelled, however many turns ago it was interrupted, so no card stays open for
    /// work the agent has moved past.
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
        let mut retired_any = false;
        for task in self
            .owned
            .durable_state_store
            .agent_tasks(Some(room), Some(prompt.target_agent_id()))?
            .into_iter()
            .filter(|task| {
                task.prompt_id != prompt.id()
                    && task.state == ExecutionState::Blocked
                    && task.goal.is_none()
                    && task.obligations.is_empty()
                    && task.wait.is_none()
                    && task.pending_prompt_id.is_none()
            })
        {
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
            retired_any |= retired.state != ExecutionState::Blocked;
        }
        if retired_any {
            self.retract_stale_task_owner_interactions(room).await;
        }
        Ok(retired_any)
    }
}
