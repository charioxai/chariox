//! MP-08 / MP-09 / MP-10 / MP-11 A02: explicit owner disposition and visible failure.
use super::*;
use crate::durable_state::agent_lifecycle::{AgentTaskExecution, Operation, Outcome};

#[derive(Clone, PartialEq, Eq)]
pub(super) enum OwnerTaskAction {
    Cancel,
    Resume,
    StartFreshAgent,
}

impl KernelRuntimeState {
    pub(super) async fn resolve_agent_task_owner_action(
        &self,
        task: &AgentTaskExecution,
        action: OwnerTaskAction,
    ) {
        // The failure is recorded on the task and in the pane; the card the person answered stays theirs.
        let _ = self.try_resolve_agent_task_owner_action(task, action).await;
    }

    /// Applies the owner's disposition and says whether it took: a caller that retries on its own
    /// (a usage-limit resume) needs the failure, not only the notice the pane shows.
    pub(super) async fn try_resolve_agent_task_owner_action(
        &self,
        task: &AgentTaskExecution,
        action: OwnerTaskAction,
    ) -> Result<(), DaemonError> {
        let resume = action == OwnerTaskAction::Resume;
        if action == OwnerTaskAction::StartFreshAgent {
            if let Err(error) = self.start_fresh_agent_for_switch(task).await {
                self.record_agent_task_owner_failure(task, &error);
                return Err(error);
            }
        }
        // Archive keeps the agent out of work. Refusing before the decision is
        // committed leaves the task blocked, and the sweep offers the same
        // decision again once the agent is unarchived.
        if resume
            && self
                .owned
                .agent_store
                .get_agent(&task.agent_id)
                .is_ok_and(|agent| agent.archived())
        {
            let error = crate::durable_state::agent_lifecycle::error(
                "unarchive this agent before resuming its task",
            );
            self.record_agent_task_owner_failure(task, &error);
            return Err(error);
        }
        if resume && task.goal.is_some() {
            if let Ok(session) = self.owned.session_store.get_session(&task.room_id) {
                if self
                    .owned
                    .prompt_state_owner
                    .active_prompt_for_agent(&session, &task.agent_id)
                    .is_some()
                {
                    let error = crate::durable_state::agent_lifecycle::error(
                        "wait for the stopped goal turn to settle before resuming",
                    );
                    self.record_agent_task_owner_failure(task, &error);
                    return Err(error);
                }
            }
        }
        let next = match self
            .owned
            .durable_state_store
            .agent_lifecycle(Operation::OwnerResponse {
                task: task.task_id.clone(),
                revision: task.blocked_revision,
                resume,
                now: crate::session::unix_epoch_ms(),
            }) {
            Ok(Outcome::Task(next)) => next,
            Err(error) => {
                self.record_agent_task_owner_failure(task, &error);
                let _ = self.owned.session_snapshot(&task.room_id);
                return Err(error);
            }
            Ok(_) => unreachable!(),
        };
        let result = async {
            if resume
                && matches!(
                    next.state,
                    crate::durable_state::agent_lifecycle::ExecutionState::Waiting
                        | crate::durable_state::agent_lifecycle::ExecutionState::Done
                )
            {
                Box::pin(self.deliver_agent_inbox(&next.room_id, &next.agent_id)).await
            } else if resume {
                async {
                    let agent = self.owned.agent_store.get_agent(&next.agent_id)?;
                    let attachment = self.ensure_agent_message_attachment(&next.room_id, &agent)?;
                    let mut context = self.owned.task_resume_text(&next, &task.reason)?;
                    if let Some(goal) = self.owned.goal_continuation_context(&next)? {
                        context.push('\n');
                        context.push_str(&goal);
                    }
                    Box::pin(self.dispatch_task_continuation(&next, &attachment, context)).await
                }
                .await
            } else {
                Box::pin(self.cancel_agent_task_resources(&next)).await
            }
        }
        .await;
        let outcome = if let Err(error) = result {
            self.record_agent_task_owner_failure(&next, &error);
            if resume {
                // Preserve the exact current task if it raced a later disposition.
                // A failed continuation has no executor and cannot remain Working.
                let reason = crate::terminal::public_task_failure_reason(&format!(
                    "Owner action failed: {}",
                    crate::secret_redaction::redact_secrets(&error.to_string())
                ));
                let operation = Operation::Block {
                    task: next.task_id.clone(),
                    prompt: next.prompt_id.clone(),
                    reason,
                };
                if let Err(block_error) = self.owned.durable_state_store.agent_lifecycle(operation)
                {
                    tracing::warn!(error=%crate::secret_redaction::redact_secrets(&block_error.to_string()),
                        task_id=%next.task_id, "MP-08/MP-09/MP-10/MP-11 A02: later task state retained after failed owner continuation");
                }
            }
            Err(error)
        } else {
            Ok(())
        };
        let _ = self.owned.session_snapshot(&next.room_id);
        outcome
    }
    async fn start_fresh_agent_for_switch(
        &self,
        task: &AgentTaskExecution,
    ) -> Result<(), DaemonError> {
        use crate::durable_state::agent_lifecycle::ExecutionState;
        let current = self.owned.durable_state_store.agent_task(&task.task_id)?;
        if !current.is_some_and(|current| {
            current.state == ExecutionState::Blocked
                && current.blocked_revision == task.blocked_revision
                && crate::provider::switch_turn::needs_fresh_agent(&current.reason)
        }) {
            return Err(crate::durable_state::agent_lifecycle::error(
                "This task's recovery decision has changed",
            ));
        }
        let source = self.owned.agent_store.get_agent(&task.agent_id)?;
        let brief = self.owned.fresh_agent_switch_brief(task)?;
        let spawn_key = format!("switch-recovery:{}:{}", task.task_id, task.blocked_revision);
        let agent = match self
            .owned
            .agent_store
            .get_session_agents(&task.room_id)
            .into_iter()
            .find(|agent| agent.spawn_key() == Some(spawn_key.as_str()))
        {
            Some(agent) => agent,
            None => {
                let mut request = crate::agent::CreateAgentRequest {
                    model: source.model().map(str::to_string),
                    effort: source.effort().map(str::to_string),
                    account_profile: source.account_profile().map(str::to_string),
                    max_context_tokens: source.max_context_tokens(),
                    execution_mode_override: source.execution_mode_override(),
                    permission_level_override: source.permission_level_override(),
                    worktree_id: source.worktree_id().map(str::to_string),
                    ..crate::agent::CreateAgentRequest::new(&task.room_id, source.provider())
                        .with_owner_user_id(source.owner_user_id())
                        .with_spawn_key(&spawn_key)
                };
                if let Some(parent) = source.parent_agent_id() {
                    request = request.with_parent(&self.owned.agent_store.get_agent(parent)?);
                }
                self.spawn_agent(request).await?
            }
        };
        let attachment = self.ensure_agent_message_attachment(&task.room_id, &agent)?;
        let prompt = crate::session::PromptQueueItem::new(
            self.owned.session_store.reserve_prompt_id(),
            &attachment,
            agent.id(),
            brief,
            crate::session::PromptStatus::Queued,
        )
        .with_durable_operation(&spawn_key, &spawn_key);
        Box::pin(
            self.submit_prepared_prompt(crate::app::KernelPreparedPromptSubmission {
                session_id: task.room_id.clone(),
                prompt,
                force_queue: false,
                refresh_projection: true,
            }),
        )
        .await?;
        Ok(())
    }

    /// The person and the agent read why the answer did not take, and the kernel log keeps it.
    fn record_agent_task_owner_failure(&self, task: &AgentTaskExecution, error: &DaemonError) {
        let error = crate::secret_redaction::redact_secrets(&error.to_string()).into_owned();
        crate::logging::warn_with_fields(
            "runtime.task_owner_action",
            "task owner action failed",
            serde_json::json!({
                "session_id": task.room_id, "agent_id": task.agent_id, "task_id": task.task_id,
                "blocked_revision": task.blocked_revision, "error": error,
            }),
        );
        self.owned.record_notice_for_agent(
            &task.room_id,
            None,
            Some(&task.agent_id),
            self.owned
                .attachment_store
                .list_session_attachment_ids(&task.room_id),
            format!(
                "Owner action failed for task {}: {error}. Check the latest task card and try again.",
                task.task_id
            ),
        );
        self.owned
            .terminal_stream
            .notify_terminal_projection_change(&task.room_id);
    }
}
