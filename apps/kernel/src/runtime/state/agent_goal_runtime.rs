//! MP-08 / MP-09 / MP-10 / MP-11 A12: wiring to ordinary prompt admission.
use super::*;
use crate::durable_state::agent_lifecycle::{
    self as ledger, AgentTaskExecution, ExecutionState, Operation, Outcome,
};

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct GoalPrompt {
    pub objective: String,
    pub minutes: u32,
}

pub(crate) fn parse_goal_prompt(text: &str) -> Result<Option<GoalPrompt>, DaemonError> {
    let text = text.trim();
    let Some(rest) = text
        .strip_prefix("/goal")
        .filter(|s| s.is_empty() || s.starts_with(char::is_whitespace))
    else {
        return Ok(None);
    };
    let mut rest = rest.trim();
    let mut minutes = 10;
    if let Some(options) = rest.strip_prefix("--interval ") {
        let (n, objective) = options
            .split_once(char::is_whitespace)
            .ok_or_else(|| ledger::error("usage: /goal [--interval 1–60] <objective>"))?;
        minutes = n
            .parse::<u32>()
            .ok()
            .filter(|n| (1..=60).contains(n))
            .ok_or_else(|| ledger::error("goal interval must be 1–60 minutes"))?;
        rest = objective.trim();
    }
    if rest.is_empty() || rest.len() > ledger::goals::RESULT_LIMIT {
        return Err(ledger::error("usage: /goal [--interval 1–60] <objective>"));
    }
    Ok(Some(GoalPrompt {
        objective: rest.into(),
        minutes,
    }))
}

pub(super) fn goal_resume_has_unsettled_turn(
    task: &AgentTaskExecution,
    active: Option<&str>,
) -> bool {
    active.is_some_and(|id| {
        id == task.prompt_id
            || task.pending_prompt_id.as_deref() == Some(id)
            || task
                .goal
                .as_ref()
                .is_some_and(|goal| goal.original_prompt_id == id)
            || ledger::completion::evaluation(task)
                .is_some_and(|check| check.prompt_id == id || check.work_prompt_id == id)
    })
}

/// What the end of a completion check's service bound did to its task.
pub(super) enum CheckExpiry {
    Retried,
    Blocked(Box<AgentTaskExecution>),
    Unchanged,
}

impl KernelRuntimeOwnedState {
    pub(super) fn validate_goal_prompt(
        &self,
        prepared: &crate::app::KernelPreparedPromptSubmission,
    ) -> Result<(), DaemonError> {
        if parse_goal_prompt(prepared.prompt.prompt())?.is_none() {
            return Ok(());
        }
        if !self.config_projection.snapshot().room_agent_tools {
            return Err(ledger::error(
                "hybrid goals require room agent tools; update the kernel configuration",
            ));
        }
        let attachment = self.ensure_attachment_in_session(
            &prepared.session_id,
            prepared.prompt.source_attachment_id(),
        )?;
        let session = self.session_store.get_session(&prepared.session_id)?;
        let agent = self
            .agent_store
            .get_agent(prepared.prompt.target_agent_id())?;
        if attachment.owner_user_id() != session.owner_user_id()
            || !matches!(
                attachment.capability_level(),
                crate::attachment::ClientCapabilityLevel::FullTerminal
                    | crate::attachment::ClientCapabilityLevel::InteractiveStructured
            )
            || agent.session_id() != session.id()
            || agent.is_metaagent()
        {
            return Err(ledger::error(
                "only the room owner can start a goal on an ordinary room agent",
            ));
        }
        if self
            .durable_state_store
            .agent_tasks(Some(session.id()), Some(agent.id()))?
            .iter()
            .any(|t| {
                t.goal.is_some()
                    && t.task_id != prepared.prompt.id()
                    && !matches!(
                        t.state,
                        ledger::ExecutionState::Done | ledger::ExecutionState::Cancelled
                    )
            })
        {
            return Err(ledger::error(
                "this agent already has an unfinished goal; resume, edit or cancel it",
            ));
        }
        Ok(())
    }

    pub(super) fn admit_goal_prompt(
        &self,
        prepared: &mut crate::app::KernelPreparedPromptSubmission,
    ) -> Result<(), DaemonError> {
        let Some(goal) = parse_goal_prompt(prepared.prompt.prompt())? else {
            return Ok(());
        };
        self.validate_goal_prompt(prepared)?;
        self.durable_state_store
            .agent_lifecycle(Operation::StartGoal {
                task: prepared.prompt.id().into(),
                owner: self
                    .session_store
                    .get_session(&prepared.session_id)?
                    .owner_user_id()
                    .into(),
                // The owner typed the objective; Room protection covers values
                // the product holds, never the owner's own text.
                objective: goal.objective.clone(),
                minutes: goal.minutes,
                now: crate::session::unix_epoch_ms(),
            })?;
        prepared.prompt = prepared.prompt.clone().with_prompt_text(goal.objective);
        Ok(())
    }
}

impl KernelRuntimeState {
    pub(super) async fn stop_goal_turn(
        &self,
        task: &AgentTaskExecution,
    ) -> Result<(), DaemonError> {
        let session = self.owned.session_store.get_session(&task.room_id)?;
        let check = ledger::completion::evaluation(task)
            .filter(|e| e.result.is_none())
            .map(|e| e.prompt_id.as_str());
        if let Some(queue) = session.queued_prompts_for_agent(&task.agent_id) {
            for prompt in queue.iter().filter(|p| {
                p.id() == task.prompt_id
                    || Some(p.id()) == task.pending_prompt_id.as_deref()
                    || Some(p.id()) == check
            }) {
                self.owned.cancel_queued_prompt(
                    &task.room_id,
                    &task.agent_id,
                    prompt.source_attachment_id(),
                    prompt.id(),
                )?;
            }
        }
        let Some(prompt) = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &task.agent_id)
            .filter(|p| p.id() == task.prompt_id || Some(p.id()) == check)
        else {
            return Ok(());
        };
        if let Some(cancellation) = self
            .cancel_remote_agent_prompt_if_remote(
                &task.room_id,
                &task.agent_id,
                prompt.source_attachment_id(),
                None,
                None,
            )
            .await?
        {
            if let Some(dispatch) = cancellation.dispatch {
                self.spawn_prompt_abort(dispatch, self.provider_runtime_lanes.clone());
            }
        } else if let Some(cancellation) = self.owned.cancel_local_prompt_if_matches(
            &task.room_id,
            &task.agent_id,
            prompt.source_attachment_id(),
            Some(prompt.id()),
        )? {
            if let Some(dispatch) = cancellation.dispatch {
                self.spawn_prompt_abort(dispatch, self.provider_runtime_lanes.clone());
            }
        }
        Ok(())
    }

    /// The owner's Vault unlock is all this agent's pending completion checks
    /// wait for: stop their service bound and say so in the task.
    pub(super) fn begin_vault_wait(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Vec<(String, String)> {
        let tasks = self
            .owned
            .durable_state_store
            .agent_tasks(Some(session_id), Some(agent_id))
            .unwrap_or_default();
        let mut waiting = Vec::new();
        for task in tasks {
            let Some(check) = ledger::completion::evaluation(&task).filter(|e| e.result.is_none())
            else {
                continue;
            };
            let prompt = check.prompt_id.clone();
            match self
                .owned
                .durable_state_store
                .agent_lifecycle(Operation::BeginOwnerWait {
                    task: task.task_id.clone(),
                    prompt: prompt.clone(),
                    now: crate::session::unix_epoch_ms(),
                }) {
                Ok(Outcome::Task(_)) => waiting.push((task.task_id, prompt)),
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(%error, "MP-08/MP-10/MP-11: completion check keeps its service bound")
                }
            }
        }
        self.publish_task_change(session_id);
        waiting
    }

    /// An unlock gives the checks back the time they waited; a refusal ends
    /// them Blocked with the reason, like any other definitive outcome.
    pub(super) async fn end_vault_wait(
        &self,
        session_id: &str,
        waiting: Vec<(String, String)>,
        refusal: Option<String>,
    ) {
        for (task, prompt) in waiting {
            let result = self
                .owned
                .durable_state_store
                .agent_lifecycle(Operation::EndOwnerWait {
                    task,
                    prompt,
                    refusal: refusal.clone(),
                    now: crate::session::unix_epoch_ms(),
                });
            match result {
                Ok(Outcome::Task(task)) if task.state == ExecutionState::Blocked => {
                    let _ = self.stop_goal_turn(&task).await;
                    let _ = self.ensure_task_owner_interaction_boxed(task).await;
                }
                Ok(Outcome::Task(task)) => self.arm_goal_check_deadline(&task),
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(%error, "MP-08/MP-10/MP-11: completion check bound resumes at its next sweep")
                }
            }
        }
        self.publish_task_change(session_id);
    }

    // The owner's Resume answer can need the Vault again; the explicit boxed type is where the
    // recursion between that answer and this wait stops needing itself to prove it is Send.
    fn ensure_task_owner_interaction_boxed(
        &self,
        task: AgentTaskExecution,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), DaemonError>> + Send + '_>>
    {
        Box::pin(self.ensure_task_owner_interaction(task))
    }

    fn publish_task_change(&self, session_id: &str) {
        let _ = self.owned.session_snapshot(session_id);
        self.owned
            .terminal_stream
            .notify_terminal_projection_change(session_id);
    }

    pub(super) fn arm_goal_check_deadline(&self, task: &AgentTaskExecution) {
        let Some(check) = ledger::completion::evaluation(task).filter(|e| {
            e.result.is_none() && task.pending_prompt_id.as_deref() == Some(e.prompt_id.as_str())
        }) else {
            return;
        };
        let wait = check
            .deadline_ms
            .saturating_sub(crate::session::unix_epoch_ms());
        let state = self.clone();
        let (task_id, room, agent) = (
            task.task_id.clone(),
            task.room_id.clone(),
            task.agent_id.clone(),
        );
        let prompt = check.prompt_id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(wait)).await;
            // The task may have moved on (answered, retried, resumed) since the timer was armed.
            let Some(task) = state
                .owned
                .durable_state_store
                .agent_tasks(Some(&room), Some(&agent))
                .ok()
                .and_then(|tasks| tasks.into_iter().find(|t| t.task_id == task_id))
            else {
                return;
            };
            let expiry = state
                .expire_goal_check(&task, &prompt, crate::session::unix_epoch_ms())
                .await;
            if let Err(error) = &expiry {
                tracing::warn!(%error, "MP-08/MP-10/MP-11: completion check expiry retained for the next sweep");
            }
            if let Ok(CheckExpiry::Blocked(task)) = expiry {
                let task = *task;
                let _ = state.ensure_task_owner_interaction(task.clone()).await;
                let _ = state.owned.session_snapshot(&task.room_id);
                state
                    .owned
                    .terminal_stream
                    .notify_terminal_projection_change(&task.room_id);
            }
        });
    }

    /// The worker lease the agent's next turn runs under; `None` for a local agent.
    pub(super) fn check_lease(&self, agent_id: &str) -> Option<ledger::goals::CheckLease> {
        let agent = self.owned.agent_store.get_agent(agent_id).ok()?;
        let remote = agent.remote_execution()?;
        Some(ledger::goals::CheckLease {
            worker_kernel_id: remote.worker_kernel_id.clone(),
            worker_machine_id: remote.worker_machine_id.clone(),
            execution_lease_id: remote.execution_lease_id.clone(),
            leased_agent_id: remote.leased_agent_id.clone(),
        })
    }

    /// The restarted worker no longer holds the check's turn, so nothing will ever
    /// settle or acknowledge its cancellation: drop it at home so the retry can run.
    fn abandon_check_turn(&self, task: &AgentTaskExecution) -> Result<(), DaemonError> {
        let Some(check) = ledger::completion::evaluation(task).map(|e| e.prompt_id.as_str()) else {
            return Ok(());
        };
        let session = self.owned.session_store.get_session(&task.room_id)?;
        if let Some(queue) = session.queued_prompts_for_agent(&task.agent_id) {
            for prompt in queue.iter().filter(|p| p.id() == check) {
                self.owned.cancel_queued_prompt(
                    &task.room_id,
                    &task.agent_id,
                    prompt.source_attachment_id(),
                    check,
                )?;
            }
        }
        let Some(active) = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &task.agent_id)
            .filter(|p| p.id() == check)
        else {
            return Ok(());
        };
        self.owned.begin_remote_prompt_cancellation_if_matches(
            &task.room_id,
            &task.agent_id,
            active.source_attachment_id(),
            Some(check),
        )?;
        self.owned
            .finalize_local_prompt_cancellation_with_queued_advance(
                &task.room_id,
                &task.agent_id,
                check,
                None,
            )?;
        Ok(())
    }

    /// The check's service bound ran out. A leased worker that restarted inside
    /// the check is back on a new lease and is asked once more; every other
    /// expiry ends the task Blocked with the reason and Resume.
    pub(super) async fn expire_goal_check(
        &self,
        task: &AgentTaskExecution,
        prompt: &str,
        now: u64,
    ) -> Result<CheckExpiry, DaemonError> {
        let store = &self.owned.durable_state_store;
        if let Some(lease) = self.check_lease(&task.agent_id) {
            if let Outcome::Task(retry) = store.agent_lifecycle(Operation::RetryGoalCheck {
                task: task.task_id.clone(),
                prompt: prompt.into(),
                lease,
                now,
            })? {
                self.abandon_check_turn(task)?;
                let agent = self.owned.agent_store.get_agent(&retry.agent_id)?;
                let attachment = self.ensure_agent_message_attachment(&retry.room_id, &agent)?;
                let context = self
                    .owned
                    .goal_continuation_context(&retry)?
                    .unwrap_or_default();
                Box::pin(self.dispatch_task_continuation(&retry, &attachment, context)).await?;
                return Ok(CheckExpiry::Retried);
            }
        }
        match store.agent_lifecycle(Operation::TimeoutGoal {
            task: task.task_id.clone(),
            prompt: prompt.into(),
            now,
        })? {
            Outcome::Task(blocked) => {
                let _ = self.stop_goal_turn(&blocked).await;
                Ok(CheckExpiry::Blocked(Box::new(blocked)))
            }
            _ => Ok(CheckExpiry::Unchanged),
        }
    }

    /// Resolve terminal shorthand before collecting authority, then fence the
    /// same goal and revision when an owner answers the sudo popup.
    pub(super) fn prepare_goal_control(
        &self,
        request: &crate::local::SubmitPromptRequest,
        owner: &str,
    ) -> Result<Option<crate::local::SubmitPromptRequest>, DaemonError> {
        let Some(rest) = request.prompt.trim().strip_prefix("/goal ") else {
            return Ok(None);
        };
        let mut words = rest.splitn(4, ' ');
        let action = words.next().unwrap_or_default();
        if !matches!(action, "pause" | "resume" | "cancel" | "interval" | "edit") {
            return Ok(None);
        }
        let session = self.owned.session_store.get_session(&request.session_id)?;
        let attachment = self
            .owned
            .ensure_attachment_in_session(session.id(), &request.attachment_id)?;
        if session.owner_user_id() != owner
            || attachment.owner_user_id() != owner
            || !matches!(
                attachment.capability_level(),
                crate::attachment::ClientCapabilityLevel::FullTerminal
                    | crate::attachment::ClientCapabilityLevel::InteractiveStructured
            )
        {
            return Err(ledger::error("only the room owner controls a goal"));
        }
        let rest = rest.strip_prefix(action).unwrap_or_default().trim();
        if rest.starts_with("goal:") {
            return Ok(Some(request.clone()));
        }
        let agent = request
            .target_agent_id
            .as_deref()
            .or(session.focused_agent_id())
            .ok_or_else(|| ledger::error("goal control needs a focused agent"))?;
        let task = self
            .owned
            .durable_state_store
            .agent_tasks(Some(session.id()), Some(agent))?
            .into_iter()
            .find(|task| {
                task.goal.is_some()
                    && !matches!(task.state, ExecutionState::Done | ExecutionState::Cancelled)
            })
            .ok_or_else(|| ledger::error("no active goal on the focused agent"))?;
        let mut request = request.clone();
        request.target_agent_id = Some(agent.into());
        request.prompt = format!(
            "/goal {action} {} {}{}",
            task.goal.as_ref().unwrap().id,
            task.revision,
            if rest.is_empty() {
                String::new()
            } else {
                format!(" {rest}")
            }
        );
        Ok(Some(request))
    }

    pub(super) fn validate_sudo_goal_resume(
        &self,
        request: &crate::local::SubmitPromptRequest,
    ) -> Result<(), DaemonError> {
        let id = request
            .prompt
            .strip_prefix("/goal resume ")
            .and_then(|rest| rest.split_whitespace().next())
            .ok_or_else(|| ledger::error("goal resume needs a goal id"))?;
        let session = self.owned.session_store.get_session(&request.session_id)?;
        let task = self
            .owned
            .durable_state_store
            .agent_tasks(Some(session.id()), request.target_agent_id.as_deref())?
            .into_iter()
            .find(|task| task.goal.as_ref().is_some_and(|goal| goal.id == id))
            .ok_or_else(|| ledger::error("goal unavailable in this room"))?;
        self.ensure_goal_resume_ready(&session, &task)
    }

    fn ensure_goal_resume_ready(
        &self,
        session: &crate::session::RuntimeSession,
        task: &AgentTaskExecution,
    ) -> Result<(), DaemonError> {
        if self.owned.goal_has_native_approval(task) {
            return Err(ledger::error(
                "answer the pending native approval before resuming this goal",
            ));
        }
        if goal_resume_has_unsettled_turn(
            task,
            self.owned
                .prompt_state_owner
                .active_prompt_for_agent(session, &task.agent_id)
                .as_ref()
                .map(|p| p.id()),
        ) {
            return Err(ledger::error(
                "wait for the stopped goal turn to settle before resuming",
            ));
        }
        Ok(())
    }

    /// Owner controls travel as a terminal command on the existing prompt route.
    /// The client includes the displayed task revision, fencing stale views.
    pub(crate) async fn control_goal_prompt(
        &self,
        request: &crate::local::SubmitPromptRequest,
        owner: &str,
    ) -> Result<Option<LocalDaemonResponse>, DaemonError> {
        if let Some(response) = self.adopt_legacy_goal(request, owner)? {
            return Ok(Some(response));
        }
        self.control_goal_prompt_with_sudo(request, owner, None)
            .await
    }

    pub(super) async fn control_goal_prompt_with_sudo(
        &self,
        request: &crate::local::SubmitPromptRequest,
        owner: &str,
        sudo: Option<&crate::local::KernelSudoTurn>,
    ) -> Result<Option<LocalDaemonResponse>, DaemonError> {
        let Some(request) = self.prepare_goal_control(request, owner)? else {
            return Ok(None);
        };
        let Some(rest) = request.prompt.trim().strip_prefix("/goal ") else {
            return Ok(None);
        };
        let mut words = rest.splitn(4, ' ');
        let action = words.next().unwrap_or_default();
        if !matches!(action, "pause" | "resume" | "cancel" | "interval" | "edit") {
            return Ok(None);
        }
        let started = std::time::Instant::now();
        let id = words
            .next()
            .ok_or_else(|| ledger::error("usage: /goal <control> <goal-id> <revision> [value]"))?;
        let revision = words
            .next()
            .and_then(|s| s.parse::<u64>().ok())
            .ok_or_else(|| ledger::error("goal control needs the displayed revision"))?;
        let raw_value = words.next();
        let session = self.owned.session_store.get_session(&request.session_id)?;
        let attachment = self
            .owned
            .ensure_attachment_in_session(session.id(), &request.attachment_id)?;
        if session.owner_user_id() != owner
            || attachment.owner_user_id() != owner
            || !matches!(
                attachment.capability_level(),
                crate::attachment::ClientCapabilityLevel::FullTerminal
                    | crate::attachment::ClientCapabilityLevel::InteractiveStructured
            )
        {
            return Err(ledger::error("only the room owner controls a goal"));
        }
        let value = raw_value.map(str::to_owned);
        let task = self
            .owned
            .durable_state_store
            .agent_tasks(Some(session.id()), request.target_agent_id.as_deref())?
            .into_iter()
            .find(|t| t.goal.as_ref().is_some_and(|g| g.id == id))
            .ok_or_else(|| ledger::error("goal unavailable in this room"))?;
        if action == "resume" {
            self.ensure_goal_resume_ready(&session, &task)?;
        }
        let looked_up = std::time::Instant::now();
        let Outcome::Task(next) =
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::ControlGoal {
                    task: task.task_id.clone(),
                    owner: owner.into(),
                    revision,
                    action: action.into(),
                    value,
                    now: crate::session::unix_epoch_ms(),
                })?
        else {
            unreachable!()
        };
        let committed = std::time::Instant::now();
        self.retract_stale_task_owner_interactions(&next.room_id, std::slice::from_ref(&next))
            .await;
        if action == "resume" {
            let resumed = async {
                if let Some(sudo) = sudo {
                    self.bind_sudo_goal_resume(sudo, &next)?;
                }
                let agent = self.owned.agent_store.get_agent(&next.agent_id)?;
                let attachment = self.ensure_agent_message_attachment(&next.room_id, &agent)?;
                Box::pin(
                    self.dispatch_task_continuation_with_context(
                        &next,
                        &attachment,
                        self.owned
                            .goal_continuation_context(&next)?
                            .unwrap_or_default(),
                        sudo.map(|_| "The owner explicitly resumed this existing goal with /sudo /goal resume and authorized a fresh task-bound sudo window with a passkey. This kernel continuation is the receipt of that owner action; no separate inbox receipt is required. The existing live sudo window applies to this task and its correlated continuations. Use chariox_kernel_request for the remaining kernel operations while the window is live. The continuation itself does not mint authority, and cannot answer approvals. Never ask for or accept a passkey in agent output.".to_owned()),
                    ),
                )
                .await
            }
            .await;
            if let Err(error) = resumed {
                // A failed authorization or dispatch cannot leave resumed work
                // claiming to run without an executor. Keep the original task.
                self.owned.durable_state_store.agent_lifecycle(Operation::Block {
                    task: next.task_id.clone(), prompt: next.prompt_id.clone(),
                    reason: format!("Goal resume failed: {}. Retry /sudo /goal resume when the provider is ready.", crate::secret_redaction::redact_secrets(&error.to_string())),
                })?;
                self.owned.session_snapshot(session.id())?;
                self.owned
                    .terminal_stream
                    .notify_terminal_projection_change(session.id());
                return Err(error);
            }
        } else if action == "cancel" {
            Box::pin(self.cancel_agent_task_resources(&next)).await?;
        } else if action == "pause" || action == "edit" {
            self.stop_goal_turn(&task).await?;
        }
        let effects_completed = std::time::Instant::now();
        let snapshot = self.owned.session_snapshot(session.id())?;
        self.owned
            .terminal_stream
            .notify_terminal_projection_change(session.id());
        let task = snapshot
            .agent_tasks()
            .iter()
            .find(|t| t.task_id == next.task_id)
            .cloned()
            .ok_or_else(|| ledger::error("controlled task projection unavailable"))?;
        crate::runtime::command_latency::log_goal_control_phases(
            &task.task_id,
            action,
            looked_up.duration_since(started),
            committed.duration_since(looked_up),
            effects_completed.duration_since(committed),
            effects_completed.elapsed(),
        );
        Ok(Some(LocalDaemonResponse::AgentTaskUpdated {
            session_id: session.id().to_owned(),
            task: Box::new(task),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a12_goal_interval_defaults_and_bounds() {
        assert_eq!(
            parse_goal_prompt("/goal do work").unwrap().unwrap().minutes,
            10
        );
        for n in [1, 60] {
            assert_eq!(
                parse_goal_prompt(&format!("/goal --interval {n} work"))
                    .unwrap()
                    .unwrap()
                    .minutes,
                n
            );
        }
        for text in [
            "/goal",
            "/goal --interval 0 work",
            "/goal --interval 61 work",
            "/goal --interval NaN work",
        ] {
            assert!(parse_goal_prompt(text).is_err());
        }
        assert!(parse_goal_prompt("/goalpost work").unwrap().is_none());
    }
}
