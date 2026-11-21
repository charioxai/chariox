//! MP-08 / MP-09 / MP-10 / MP-11 A12-S02: shared owner decision projection.
use super::*;
use crate::durable_state::agent_lifecycle::{AgentTaskExecution, ExecutionState};

pub(super) use crate::session::{is_blocked_task_decision, BLOCKED_TASK_DECISION_PREFIX};
use crate::session::{RuntimeInteractionChoice, RuntimeInteractionChoiceStyle};

// Existing question cards called the Dismiss answer "cancel".
pub(super) const DISMISS_BLOCKED_TASK_CARD: &str = "cancel";
const STOP_BLOCKED_TASK: &str = "stop";

fn blocked_card_task(interaction_id: &str) -> Option<(&str, u64)> {
    let (task, revision) = interaction_id
        .strip_prefix(BLOCKED_TASK_DECISION_PREFIX)?
        .rsplit_once('-')?;
    Some((task, revision.parse().ok()?))
}

impl KernelRuntimeOwnedState {
    pub(super) fn dismiss_blocked_task_card(
        &self,
        room: &str,
        interaction_id: &str,
    ) -> Result<(), DaemonError> {
        use crate::durable_state::agent_lifecycle::Operation;
        let (task, blocked_revision) =
            blocked_card_task(interaction_id).ok_or_else(|| DaemonError::LocalTransport {
                operation: "dismiss blocked task card",
                message: "Invalid card ID. Read the Room again.".into(),
            })?;
        self.durable_state_store
            .agent_lifecycle(Operation::DismissTaskCard {
                room: room.into(),
                task: task.into(),
                blocked_revision,
            })?;
        Ok(())
    }

    /// A task-owner card needs a current blocker the owner can act on.
    fn task_needs_owner_interaction(&self, task: &AgentTaskExecution) -> Result<bool, DaemonError> {
        if task.state != ExecutionState::Blocked
            || task.goal.as_ref().is_some_and(|goal| goal.paused)
            // A usage-limit stop has its own stripe and its own resume.
            || self.agent_store.get_agent(&task.agent_id).is_ok_and(|agent| {
                agent.usage_limit_stop().is_some_and(|stop| {
                    stop.prompt_id == task.prompt_id
                        && !matches!(stop.state, crate::agent::UsageLimitStopState::Declined)
                })
            })
            || self.durable_state_store.agent_deletion_recorded(&task.agent_id)?
        {
            return Ok(false);
        }
        // A synthetic delivery task only represents a receipt that still needs
        // reconciliation. A terminal receipt already told whoever was waiting.
        let sequence = task
            .task_id
            .strip_prefix("delivery-")
            .and_then(|id| id.parse().ok());
        if let Some(sequence) = sequence {
            return Ok(self
                .durable_state_store
                .agent_inbox_event(sequence)?
                .is_some_and(|(event, _)| {
                    event.room_id == task.room_id
                        && event.agent_id == task.agent_id
                        && event.state == "blocked"
                }));
        }
        Ok(true)
    }

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
        // Each decision names its task; only those are read, not the Room's finished turns.
        let mut stale = Vec::new();
        for id in session.active_interactions().iter().map(|i| i.id()) {
            if !is_blocked_task_decision(id) {
                continue;
            }
            let current = match blocked_card_task(id) {
                Some((task, revision)) => match self.durable_state_store.agent_task(task)? {
                    Some(task) if task.room_id == room && revision == task.blocked_revision => {
                        self.task_needs_owner_interaction(&task)?
                    }
                    _ => false,
                },
                None => false,
            };
            if !current {
                stale.push(id.to_owned());
            }
        }
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
    /// The person is alerted to a card they hold; a card an agent holds alerts them only when it is
    /// handed up to them.
    fn notify_owner_if_theirs(
        &self,
        task: &AgentTaskExecution,
        id: &str,
    ) -> Result<(), DaemonError> {
        let theirs = self
            .owned
            .pending_interactions
            .write()
            .get(id)
            .is_none_or(|pending| pending.holder == super::DecisionHolder::User);
        if theirs {
            self.owned.emit_task_owner_notification(task, id)?;
        }
        Ok(())
    }

    pub(super) async fn ensure_task_owner_interaction(
        &self,
        task: AgentTaskExecution,
    ) -> Result<(), DaemonError> {
        // The sweep may have read its candidate before a reply or cancellation committed.
        let Some(task) = self
            .owned
            .durable_state_store
            .agent_task(&task.task_id)?
            .filter(|current| {
                current.state == ExecutionState::Blocked
                    && current.blocked_revision == task.blocked_revision
            })
        else {
            self.retract_stale_task_owner_interactions(&task.room_id)
                .await;
            return Ok(());
        };
        if self.owned.goal_has_native_approval(&task) {
            return Ok(());
        }
        // A workflow step never raises a card: its run ends and says why.
        if self.end_run_step_of_blocked_task(&task).await? {
            return Ok(());
        }
        if self.defer_task_for_human_control(&task).await {
            return Ok(());
        }
        if !self.owned.task_needs_owner_interaction(&task)? {
            self.retract_stale_task_owner_interactions(&task.room_id)
                .await;
            return Ok(());
        }
        let id = format!(
            "{BLOCKED_TASK_DECISION_PREFIX}{}-{}",
            task.task_id, task.blocked_revision
        );
        let session = self.owned.session_store.get_session(&task.room_id)?;
        let question = task.owner_action
            == Some(crate::durable_state::agent_lifecycle::TaskOwnerAction::Question);
        let agent = self.owned.agent_store.get_agent(&task.agent_id).ok();
        let name = agent
            .as_ref()
            .map(|agent| agent.alias().unwrap_or(agent.agent_ref()));
        let message = if question {
            let reason = crate::terminal::public_task_failure_reason(&task.reason);
            match name {
                Some(name) => format!("{name}: {reason}"),
                None => reason,
            }
        } else {
            self.owned.blocked_card_message(name, &task)
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
                self.notify_owner_if_theirs(&task, &id)?;
                return Ok(());
            }
            // Hot reload retains the task's decision, but an in-memory reply channel must be reattached.
            self.owned.timeout_runtime_interaction(&task.room_id, &id)?;
        }
        let mut choices = vec![RuntimeInteractionChoice::new(
            DISMISS_BLOCKED_TASK_CARD,
            "Dismiss",
            DISMISS_BLOCKED_TASK_CARD,
            None,
        )
        .with_description(
            "Hide this card from the Room. The task keeps waiting and you can answer in the Inbox.",
        )];
        if !question {
            let (id, label, description) =
                if crate::provider::switch_turn::needs_fresh_agent(&task.reason) {
                    (
                        "start_fresh_agent",
                        "Start a fresh agent",
                        "Start a new agent for this work.",
                    )
                } else {
                    ("resume", "Resume", "Continue the waiting task.")
                };
            choices.push(
                RuntimeInteractionChoice::new(
                    id,
                    label,
                    id,
                    Some(RuntimeInteractionChoiceStyle::Primary),
                )
                .with_description(description),
            );
        }
        choices.push(
            RuntimeInteractionChoice::new(
                STOP_BLOCKED_TASK,
                name.map_or_else(
                    || "Stop task".to_owned(),
                    |name| format!("Stop {name}'s task"),
                ),
                STOP_BLOCKED_TASK,
                Some(RuntimeInteractionChoiceStyle::Danger),
            )
            .with_description("End the waiting task."),
        );
        let dismissed = self
            .owned
            .durable_state_store
            .task_card_is_dismissed(&task.task_id, task.blocked_revision)?;
        let mut interaction = crate::session::RuntimeInteraction::for_kernel_operation(
            &id,
            &id,
            if question {
                "Question"
            } else {
                "Agent needs your action"
            },
            &message,
            choices,
        )
        .without_timeout()
        .with_related_agent_id(task.agent_id.clone())
        .with_room_dismissed(dismissed);
        if question {
            interaction = interaction.with_question_reply(
                crate::session::RuntimeInteractionCustomChoice::new(
                    "reply",
                    "Reply",
                    None,
                    Some(1),
                    None,
                )
                .with_description("Send your reply and continue the waiting task."),
            );
        }

        let rx = self
            .create_kernel_operation_interaction(
                &task.room_id,
                session.owner_user_id(),
                interaction,
            )
            .await?;
        // The agent that holds the card is told; a card nobody above can hold is the person's.
        self.announce_child_decision(&task.room_id, &id).await?;
        let notification = self.notify_owner_if_theirs(&task, &id);
        let state = self.as_kernel();
        tokio::spawn(async move {
            if let Ok(answer) = rx.await {
                if answer.status == "timed_out" {
                    return;
                }
                use super::agent_task_owner_resolution::OwnerTaskAction;
                let action = match answer.choice_id.as_deref() {
                    Some("resume") => OwnerTaskAction::Resume,
                    Some("start_fresh_agent") => OwnerTaskAction::StartFreshAgent,
                    Some("reply") => OwnerTaskAction::Reply(answer.reply.unwrap_or_default()),
                    Some(STOP_BLOCKED_TASK) => OwnerTaskAction::Cancel,
                    _ => return,
                };
                state.resolve_agent_task_owner_action(&task, action).await;
            }
        });
        notification?;
        Ok(())
    }

    /// A later turn of an agent replaces its interrupted attempts (a restart, a refused or
    /// unacknowledged dispatch, a wake that never resumed them), not a goal or open work. Every such
    /// attempt still waiting on the owner is cancelled with that reason, however many turns ago it
    /// was interrupted, so no card stays open for work the agent has moved past. A task with a
    /// live obligation or registration keeps its card and its real reason.
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
            .agent_unfinished_tasks(room, Some(prompt.target_agent_id()))?
            .into_iter()
            .filter(|task| {
                task.prompt_id != prompt.id()
                    && task.state == ExecutionState::Blocked
                    && !task.owns_unfinished_work()
            })
        {
            let crate::durable_state::agent_lifecycle::Outcome::Task(retired) =
                self.owned.durable_state_store.agent_lifecycle(
                    crate::durable_state::agent_lifecycle::Operation::RetireRestartFailure {
                        task: task.task_id,
                        revision: task.blocked_revision,
                        later_prompt: prompt.id().to_owned(),
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
