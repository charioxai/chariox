//! The correction, blocked decision and Resume describe the same unresolved work.
use super::*;
use crate::durable_state::agent_lifecycle::AgentTaskExecution;

impl KernelRuntimeOwnedState {
    fn task_settle_steps(&self, task: &AgentTaskExecution) -> Result<Vec<String>, DaemonError> {
        let registrations = self
            .durable_state_store
            .agent_registrations(&task.task_id)?;
        let wakes = self
            .durable_state_store
            .agent_wakes(Some(&task.room_id), Some(&task.agent_id))?;
        let inbox = self
            .durable_state_store
            .agent_inbox(&task.room_id, &task.agent_id, 0)?;
        let mut steps = Vec::new();
        for obligation in task.obligations.iter().filter(|o| o.requires_resolution()) {
            let resource = obligation.resource_id.as_deref().unwrap_or(&obligation.id);
            let wake = wakes.iter().find(|wake| wake.id == resource);
            let label = wake.map(|wake| wake.label.clone()).or_else(|| {
                self.agent_store
                    .get_agent(resource)
                    .ok()
                    .and_then(|agent| agent.alias().map(str::to_owned))
            });
            let label = label.map_or(String::new(), |label| format!(" ({label:?})"));
            let what = format!(
                "obligation {}: {} {}{}",
                obligation.id, obligation.kind, resource, label
            );
            let step = if obligation.status == "settling" || obligation.status == "failed" {
                let receipts = inbox
                    .iter()
                    .filter(|event| {
                        obligation
                            .completion_source()
                            .is_some_and(|source| event.reports_source(source))
                            && !matches!(event.state.as_str(), "handled" | "expired" | "failed")
                    })
                    .map(|event| {
                        format!(
                            "chariox.events.ack with sequence {} and handled:true",
                            event.sequence
                        )
                    })
                    .collect::<Vec<_>>();
                let ack = if receipts.is_empty() {
                    "chariox.events.ack with the reviewed receipt's sequence and handled:true"
                        .into()
                } else {
                    receipts.join("; ")
                };
                format!("{what} has an outcome: call chariox.events.inbox, handle its result or failure, then call {ack}")
            } else if let Some(wake) = wake {
                format!("{what} is still open: call chariox.events.cancel_wake with wake_id {:?}; review and acknowledge any unhandled cancellation receipt. To keep it running, use the wait below", wake.id)
            } else if registrations
                .iter()
                .any(|r| r.live && r.obligation_id.as_deref() == Some(&obligation.id))
            {
                format!("{what} is still open: use the wait below, then review and acknowledge its completion receipt")
            } else {
                let source = obligation.completion_source().unwrap_or(resource);
                format!("{what} has no live completion registration: call chariox.events.subscribe with source_id {source:?} and obligation_id {:?}, then include its returned registration_id in the wait", obligation.id)
            };
            steps.push(step);
        }
        if task.obligations.iter().any(|o| o.needs_live_source()) {
            let ids = registrations
                .iter()
                .filter(|r| {
                    r.live
                        && r.obligation_id.as_deref().is_some_and(|id| {
                            task.obligations
                                .iter()
                                .any(|o| o.id == id && o.needs_live_source())
                        })
                })
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>();
            if !ids.is_empty() {
                let deadline = task
                    .goal
                    .as_ref()
                    .map_or(String::new(), |_| " and a future deadline_ms".into());
                steps.push(format!(
                    "To wait, call chariox.events.yield with registration_ids {}, inbox_cursor {} and reason \"Wait for unresolved work\"{deadline}; include any newly subscribed registrations so every open obligation is covered. End the turn immediately after yield",
                    serde_json::to_string(&ids).expect("registration ids serialize"),
                    task.wait.as_ref().map_or(0, |wait| wait.inbox_cursor)
                ));
            }
        }
        Ok(steps)
    }

    pub(super) fn task_correction_text(
        &self,
        task: &AgentTaskExecution,
    ) -> Result<String, DaemonError> {
        let mut text = super::agent_task_projection::task_correction_text(task);
        let steps = self.task_settle_steps(task)?;
        if !steps.is_empty() {
            text.push_str(&format!(" Settle each by these exact calls: {}. Use this turn's task_id and origin_prompt_id for every events call.", steps.join("; ")));
        }
        Ok(text)
    }

    pub(super) fn task_resume_text(
        &self,
        task: &AgentTaskExecution,
        reason: &str,
    ) -> Result<String, DaemonError> {
        let reason = crate::terminal::public_task_failure_reason(reason);
        Ok(format!(
            "This task was blocked ({reason}) and has now been resumed. {}",
            self.task_correction_text(task)?
        ))
    }

    pub(super) fn blocked_card_message(
        &self,
        alias: Option<&str>,
        task: &AgentTaskExecution,
    ) -> Result<String, DaemonError> {
        let reason = crate::terminal::public_task_failure_reason(&task.reason);
        let mut text = alias.map_or_else(|| reason.clone(), |alias| format!("{alias}: {reason}"));
        match self.task_settle_steps(task) {
            Ok(steps) if !steps.is_empty() => {
                text.push_str(&format!(" Open: {}.", steps.join("; ")));
            }
            Ok(_) => {}
            Err(error) => {
                // Damaged registrations must not take away the task's Cancel choice.
                let reason =
                    crate::secret_redaction::redact_secrets(&error.to_string()).into_owned();
                text.push_str(&format!(" Recovery instructions could not be read: {reason}. Cancel this task or restore its authoritative receipts before Resume."));
            }
        }
        Ok(text)
    }
}
