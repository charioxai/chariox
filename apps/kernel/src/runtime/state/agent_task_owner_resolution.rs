//! MP-08 / MP-09 / MP-10 / MP-11 A02: explicit owner disposition and visible failure.
use super::*;
use crate::durable_state::agent_lifecycle::{AgentTaskExecution, Operation, Outcome};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum OwnerTaskAction {
    Cancel,
    Resume,
    AllowBrowser,
}

impl KernelRuntimeState {
    pub(super) async fn block_for_browser_access(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        agent: &crate::agent::AgentInstance,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let refused = || {
            super::capability_grant_runtime::refused(
                crate::error::UserDomainRefusalReason::NotRequested,
            )
        };
        let session = self.owned.session_store.get_session(agent.session_id())?;
        let prompt = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent_snapshot(&session, agent.id())
            .ok_or_else(refused)?;
        if agent.remote_execution().is_some()
            || self
                .app_prompt_delivery(agent)
                .is_none_or(|delivery| !delivery.belongs_to(run.id()))
        {
            return Err(refused());
        }
        let task = self
            .owned
            .durable_state_store
            .agent_tasks(Some(agent.session_id()), Some(agent.id()))?
            .into_iter()
            .find(|task| {
                task.prompt_id == prompt.id()
                    && task.provider_run_id.as_deref() == Some(run.id())
                    && matches!(
                        task.state,
                        crate::durable_state::agent_lifecycle::ExecutionState::Working
                            | crate::durable_state::agent_lifecycle::ExecutionState::Blocked
                    )
            })
            .ok_or_else(refused)?;
        let Outcome::Task(blocked) =
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::BlockForBrowserAccess {
                    task: task.task_id,
                    prompt: prompt.id().into(),
                    reason: "Browser access ended. Please allow browser access.".into(),
                })?
        else {
            unreachable!()
        };
        self.ensure_task_owner_interaction(blocked).await?;
        Ok(crate::transport::runtime_tools::RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "status":"approval_required", "message":"Browser access needs your approval. Choose Allow browser to continue.",
                "next_action":"End this turn and wait for the owner's choice. Chariox continues the same task after approval."
            }),
        })
    }

    /// The "allow browser access" card asks for consent. A browser that failed to start is not a
    /// consent question: nothing the owner could allow brings it up, so that never becomes a card.
    /// A grant that merely lacks the resource the task needs still can: approval widens it.
    pub(super) fn refuse_browser_access_card_after_start_failure(
        &self,
        room: &str,
        actor: &str,
    ) -> Result<(), DaemonError> {
        let Some(agent) = self
            .session_agents(room)
            .into_iter()
            .find(|agent| agent.id() == actor)
        else {
            return Ok(());
        };
        let user = self.provider_account_authority_owner_user_id(agent.owner_user_id());
        if self.owned.kernel_browser_host.start_failed(&user) {
            return Err(crate::durable_state::agent_lifecycle::error(
                "The browser did not start, and that is not something the owner can allow. \
                 Call start once more; if it still fails, tell the owner in plain words that the \
                 browser did not start (include the reason class from the error) and do not use blocked.",
            ));
        }
        Ok(())
    }

    fn browser_resume_is_current(&self, next: &AgentTaskExecution) -> Result<bool, DaemonError> {
        Ok(self
            .owned
            .durable_state_store
            .agent_tasks(Some(&next.room_id), Some(&next.agent_id))?
            .iter()
            .any(|task| {
                task.task_id == next.task_id
                    && task.prompt_id == next.prompt_id
                    && task.blocked_revision == next.blocked_revision
                    && matches!(
                        task.state,
                        crate::durable_state::agent_lifecycle::ExecutionState::Working
                            | crate::durable_state::agent_lifecycle::ExecutionState::Waiting
                    )
            }))
    }

    pub(super) async fn resolve_agent_task_owner_action(
        &self,
        task: &AgentTaskExecution,
        action: OwnerTaskAction,
    ) {
        let resume = action != OwnerTaskAction::Cancel;
        let browser_grant = if action == OwnerTaskAction::AllowBrowser {
            match self.owned.agent_store.get_agent(&task.agent_id) {
                Ok(agent) if agent.session_id() == task.room_id && agent.owner_user_id() == task.owner_user_id
                    && agent.remote_execution().is_none() && !agent.archived()
                    && task.owner_action == Some(crate::durable_state::agent_lifecycle::TaskOwnerAction::BrowserAccess) => {
                    let user = self.provider_account_authority_owner_user_id(agent.owner_user_id());
                    let fence = self.owned.kernel_browser_host.acquisition_fence(&user, agent.id());
                    Some((user, fence))
                }
                _ => return,
            }
        } else {
            None
        };
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
            self.record_agent_task_owner_failure(
                task,
                &crate::durable_state::agent_lifecycle::error(
                    "unarchive this agent before resuming its task",
                ),
            );
            return;
        }
        if resume && task.goal.is_some() {
            if let Ok(session) = self.owned.session_store.get_session(&task.room_id) {
                if self
                    .owned
                    .prompt_state_owner
                    .active_prompt_for_agent(&session, &task.agent_id)
                    .is_some()
                {
                    self.record_agent_task_owner_failure(
                        task,
                        &crate::durable_state::agent_lifecycle::error(
                            "wait for the stopped goal turn to settle before resuming",
                        ),
                    );
                    return;
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
                return;
            }
            Ok(_) => unreachable!(),
        };
        let result = async {
            if let Some((user, fence)) = &browser_grant {
                if !self.browser_resume_is_current(&next)? {
                    return Err(crate::durable_state::agent_lifecycle::error(
                        "This task changed. Open its latest decision.",
                    ));
                }
                self.owned.kernel_browser_host.request_task_grant(user, &task.agent_id, &task.task_id,
                    task.blocked_revision, *fence, &task.room_id).map_err(super::kernel_browser_runtime::host_error)?;
                if !self.browser_resume_is_current(&next)? {
                    self.owned.kernel_browser_host.revoke_task_grant(user, &task.agent_id, &task.task_id, task.blocked_revision).map_err(super::kernel_browser_runtime::host_error)?;
                    return Err(crate::durable_state::agent_lifecycle::error("This task changed. Open its latest decision."));
                }
                let agent = self.owned.agent_store.get_agent(&task.agent_id)?;
                if crate::provider::provider_runtime_catalog_requires_reload(agent.provider()) {
                    self.remember_pending_provider_catalog_reload(&task.room_id, &task.agent_id);
                }
            }
            if resume
                && next.state == crate::durable_state::agent_lifecycle::ExecutionState::Waiting
            {
                Box::pin(self.deliver_agent_inbox(&next.room_id, &next.agent_id)).await
            } else if resume {
                async {
                    let agent = self.owned.agent_store.get_agent(&next.agent_id)?;
                    let attachment = self.ensure_agent_message_attachment(&next.room_id, &agent)?;
                    let mut context = if task.owner_action.is_some() {
                        "The owner has allowed this task to continue. Browser access, if requested, has been restored. Check the current page and continue the original task. If a previous verification card ended, take a fresh snapshot and raise a new card. Write card instructions for the person, and keep tool names and internal task bookkeeping out of your visible replies.".into()
                    } else {
                        self.owned.task_resume_text(&next, &task.reason)?
                    };
                    if let Some(goal) = self.owned.goal_continuation_context(&next)? {
                        context.push('\n');
                        context.push_str(&goal);
                    }
                    if browser_grant.is_some() { context.push_str(" Browser access has been approved. Take a fresh snapshot of the original tab and replace any retired verification card. Write the card's steps directly to the person. Do not narrate snapshots, browser grants, tool loading, scheduling instructions or task bookkeeping; tell the person only what they need to do or what result changed."); }
                    Box::pin(self.dispatch_task_continuation(&next, &attachment, context)).await
                }.await
            } else {
                Box::pin(self.cancel_agent_task_resources(&next)).await
            }
        }.await;
        if let Err(error) = result {
            self.record_agent_task_owner_failure(&next, &error);
            if resume {
                // Preserve the exact current task if it raced a later disposition.
                // A failed continuation has no executor and cannot remain Working.
                let reason = crate::terminal::public_task_failure_reason(&format!(
                    "Owner action failed: {}",
                    crate::secret_redaction::redact_secrets(&error.to_string())
                ));
                let operation = if browser_grant.is_some() {
                    Operation::BlockForBrowserAccess {
                        task: next.task_id.clone(),
                        prompt: next.prompt_id.clone(),
                        reason,
                    }
                } else {
                    Operation::Block {
                        task: next.task_id.clone(),
                        prompt: next.prompt_id.clone(),
                        reason,
                    }
                };
                if let Err(block_error) = self.owned.durable_state_store.agent_lifecycle(operation)
                {
                    tracing::warn!(error=%crate::secret_redaction::redact_secrets(&block_error.to_string()),
                        task_id=%next.task_id, "MP-08/MP-09/MP-10/MP-11 A02: later task state retained after failed owner continuation");
                }
            }
        }
        let _ = self.owned.session_snapshot(&next.room_id);
    }
    fn record_agent_task_owner_failure(&self, task: &AgentTaskExecution, error: &DaemonError) {
        tracing::warn!(task_id=%task.task_id, error=%crate::secret_redaction::redact_secrets(&error.to_string()), "task owner action failed");
        self.owned.record_notice_for_agent(
            &task.room_id,
            None,
            Some(&task.agent_id),
            self.owned
                .attachment_store
                .list_session_attachment_ids(&task.room_id),
            "Owner action failed. Check the latest task card and try again.",
        );
        self.owned
            .terminal_stream
            .notify_terminal_projection_change(&task.room_id);
    }
}
