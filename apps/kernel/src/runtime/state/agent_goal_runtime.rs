//! MP-08 / MP-09 / MP-10 / MP-11 A12: wiring to ordinary prompt admission.
use super::*;
use crate::durable_state::agent_lifecycle::{
    self as ledger, AgentTaskExecution, Operation, Outcome,
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
        let task_id = task.task_id.clone();
        let prompt = check.prompt_id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(wait)).await;
            let result = state
                .owned
                .durable_state_store
                .agent_lifecycle(Operation::TimeoutGoal {
                    task: task_id,
                    prompt,
                    now: crate::session::unix_epoch_ms(),
                });
            if let Ok(Outcome::Task(task)) = result {
                let _ = state.stop_goal_turn(&task).await;
                let _ = state.ensure_task_owner_interaction(task.clone()).await;
                let _ = state.owned.session_snapshot(&task.room_id);
                state
                    .owned
                    .terminal_stream
                    .notify_terminal_projection_change(&task.room_id);
            }
        });
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
        if action == "resume"
            && goal_resume_has_unsettled_turn(
                &task,
                self.owned
                    .prompt_state_owner
                    .active_prompt_for_agent(&session, &task.agent_id)
                    .as_ref()
                    .map(|p| p.id()),
            )
        {
            return Err(ledger::error(
                "wait for the stopped goal turn to settle before resuming",
            ));
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
        if action == "resume" {
            let agent = self.owned.agent_store.get_agent(&next.agent_id)?;
            let attachment = self.ensure_agent_message_attachment(&next.room_id, &agent)?;
            Box::pin(
                self.dispatch_task_continuation(
                    &next,
                    &attachment,
                    self.owned
                        .goal_continuation_context(&next)?
                        .unwrap_or_default(),
                ),
            )
            .await?;
        } else if action == "cancel" {
            Box::pin(self.cancel_agent_task_resources(&next)).await?;
        } else if action == "pause" || action == "edit" {
            self.stop_goal_turn(&task).await?;
        }
        self.retract_stale_task_owner_interactions(&next.room_id, std::slice::from_ref(&next))
            .await;
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
