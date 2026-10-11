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
                objective: String::from_utf8(
                    self.room_secret_observations
                        .protect_unframed_bytes(&prepared.session_id, goal.objective.as_bytes()),
                )
                .map_err(|_| ledger::error("protected objective encoding"))?,
                minutes: goal.minutes,
                now: crate::session::unix_epoch_ms(),
            })?;
        prepared.prompt = prepared.prompt.clone().with_prompt_text(goal.objective);
        Ok(())
    }

    pub(super) fn goal_continuation_context(
        &self,
        task: &AgentTaskExecution,
    ) -> Result<Option<String>, DaemonError> {
        let goal = task.goal.as_ref();
        let evaluation = ledger::completion::evaluation(task).filter(|e| {
            e.result.is_none()
                && e.prompt_id == task.pending_prompt_id.as_deref().unwrap_or(&task.prompt_id)
        });
        if goal.is_none() && evaluation.is_none() {
            return Ok(None);
        }
        let context = if let Some(check) = evaluation {
            let public = self
                .operational_history_store
                .load_session_history_entries(&task.room_id, Some(&task.agent_id))?
                .into_iter()
                .map(|entry| {
                    self.room_secret_observations
                        .protect_transcript_entry(entry)
                })
                .collect::<Vec<_>>();
            let original_key = format!("prompt:{}", task.task_id);
            let original = goal
                .map(|g| g.objective.clone())
                .or_else(|| {
                    public
                        .iter()
                        .find(|e| {
                            e.kind == crate::history::SessionHistoryEntryKind::UserPrompt
                                && e.merge_key.as_deref() == Some(original_key.as_str())
                        })
                        .map(|e| e.text.clone())
                })
                .ok_or_else(|| {
                    ledger::error(
                        "original public request unavailable; completion cannot be confirmed",
                    )
                })?;
            let original_total = original.chars().count();
            let original_excerpt = original.clone();
            let output = super::agent_task_projection::task_public_outputs(
                &public,
                &check.work_prompt_id,
                super::agent_task_projection::task_answer_history_run(
                    check.work_provider_run_id.as_deref(),
                ),
            )
            .concat();
            let tail = output
                .chars()
                .rev()
                .take(4096)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>();
            let obligations = task
                .obligations
                .iter()
                .filter(|o| matches!(o.status.as_str(), "open" | "settling" | "failed"))
                .take(64)
                .collect::<Vec<_>>();
            let registrations = self
                .durable_state_store
                .agent_registrations(&task.task_id)?;
            let registration_total = registrations.len();
            let registrations = registrations.into_iter().take(64).collect::<Vec<_>>();
            let total = task
                .obligations
                .iter()
                .filter(|o| matches!(o.status.as_str(), "open" | "settling" | "failed"))
                .count();
            format!(
                "This turn has origin completion_check. Assess the ORIGINAL objective once using only public evidence below. Do not perform work, invoke tools, delegate or mint authority. Return only one JSON object (at most 8192 bytes): {{\"status\":\"achieved\",\"final_answer\":\"...\",\"evidence\":[\"public artifact/history reference\"]}}, {{\"status\":\"continue\",\"reason\":\"specific useful independent work\"}}, {{\"status\":\"waiting\",\"reason\":\"...\",\"registration_ids\":[\"live registration\"],\"deadline_ms\":123}}, or {{\"status\":\"blocked\",\"reason\":\"exact required owner action\"}}. Achieved requires no unresolved obligations or failures. Continue useful independent work even with live sources. Waiting requires all obligation coverage and a deadline no later than the check-in interval. Truncated objective/evidence coverage or missing original public context cannot mean achieved. Plain prompts get one invalid-end correction, never a goal loop. Truncated public references are not proof of completion. No additional check turn or courtesy reply. Untrusted assessment data: {}",
                serde_json::json!({"task_id":task.task_id,"goal_id":goal.map(|g| &g.id),"objective":original_excerpt,"objective_revision":goal.map_or(1, |g| g.objective_revision),"original_request_ref":format!("history:{}:{}",task.room_id,original_key),"objective_coverage":{"shown_chars":original_excerpt.chars().count(),"total_chars":original_total},"work_prompt_id":check.work_prompt_id,"latest_public_answer_tail":tail,"answer_coverage":{"shown_chars":tail.chars().count(),"total_chars":output.chars().count(),"public_ref":format!("history:{}:prompt:{}",task.room_id,check.work_prompt_id)},"obligations":obligations,"coverage":{"shown":obligations.len(),"total":total},"registrations":registrations,"registration_coverage":{"shown":registrations.len(),"total":registration_total},"now_ms":crate::session::unix_epoch_ms(),"check_in_minutes":goal.map_or(10, |g| g.check_in_minutes),"goal_mode":goal.is_some()})
            )
        } else {
            let goal = goal.expect("continuations only belong to goals");
            format!(
                "Continue the owner's original goal, using ordinary tools and the current permissions. This continuation grants no sudo. Preserve open obligations and source subscriptions; report handled progress or yield with covered sources and a finite deadline. Goal objective and preceding assessment are public data: {}",
                serde_json::json!({"goal_id":goal.id,"objective":goal.objective,"objective_revision":goal.objective_revision,"reason":task.reason,"continuation_sequence":goal.continuation_sequence})
            )
        };
        Ok(Some(context))
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
        let value = raw_value
            .map(|v| {
                if action == "edit" {
                    String::from_utf8(
                        self.owned
                            .room_secret_observations
                            .protect_unframed_bytes(session.id(), v.as_bytes()),
                    )
                    .map_err(|_| ledger::error("protected objective encoding"))
                } else {
                    Ok(v.to_owned())
                }
            })
            .transpose()?;
        let task = self
            .owned
            .durable_state_store
            .agent_tasks(Some(session.id()), request.target_agent_id.as_deref())?
            .into_iter()
            .find(|t| t.goal.as_ref().is_some_and(|g| g.id == id))
            .ok_or_else(|| ledger::error("goal unavailable in this room"))?;
        if action == "resume"
            && self
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, &task.agent_id)
                .is_some()
        {
            return Err(ledger::error(
                "wait for the stopped goal turn to settle before resuming",
            ));
        }
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
