//! MP-08 / MP-09 / MP-10 / MP-11 A02: bounded public task answer projection.
use super::*;
use crate::durable_state::agent_lifecycle::{AgentTaskExecution, ExecutionState};
impl KernelRuntimeOwnedState {
    pub(super) fn record_agent_artifact_progress(
        &self,
        room: &str,
        agent: &str,
        prompt: &crate::session::PromptQueueItem,
        run: &str,
    ) -> Result<(), DaemonError> {
        let Some(snapshot) = self
            .completed_git_turn_snapshots
            .resolve(room, agent, Some(prompt.id()))
            .filter(|s| s.before.prompt_id == prompt.id() && s.before.provider_run_id == run)
        else {
            return Ok(());
        };
        if snapshot.before.head_sha == snapshot.after.head_sha
            && snapshot.before.status_fingerprint == snapshot.after.status_fingerprint
            && snapshot.before.workspace_live_sync_file_snapshots
                == snapshot.after.workspace_live_sync_file_snapshots
        {
            return Ok(());
        }
        let Some(task) = self
            .durable_state_store
            .agent_tasks(Some(room), Some(agent))?
            .into_iter()
            .find(|t| {
                t.prompt_id == prompt.id()
                    && t.provider_run_id.as_deref() == Some(run)
                    && t.state == ExecutionState::Working
                    && t.pending_prompt_id.is_none()
            })
        else {
            return Ok(());
        };
        use sha2::{Digest, Sha256};
        let normalized = serde_json::to_vec(&(
            &snapshot.after.head_sha,
            &snapshot.after.status_fingerprint,
            &snapshot.after.workspace_live_sync_file_snapshots,
        ))
        .map_err(|_| {
            crate::durable_state::agent_lifecycle::error("artifact receipt normalization failed")
        })?;
        let receipt = format!("git-artifact:{:x}", Sha256::digest(normalized));
        self.durable_state_store.agent_lifecycle(
            crate::durable_state::agent_lifecycle::Operation::Progress {
                task: task.task_id,
                prompt: prompt.id().into(),
                receipt,
                now: crate::session::unix_epoch_ms(),
            },
        )?;
        Ok(())
    }
}

pub(super) fn task_status_notice(task: &AgentTaskExecution, name: &str) -> String {
    let state = match task.state {
        ExecutionState::Working => "Working",
        ExecutionState::Waiting => "Waiting",
        ExecutionState::Done => "Done",
        ExecutionState::Blocked => "Needs your help",
        ExecutionState::Cancelled => "Stopped",
    };
    format!("{name}: {state}.")
}

// The single correction for a turn that ended without a valid wait or answer.
// Only unresolved obligations are actionable, each with the step that settles
// it: a completion that already arrived is acknowledged, a running source is
// waited on, a failure is handled. With none left, the agent just answers.
pub(super) fn task_correction_text(task: &AgentTaskExecution) -> String {
    let with = |status: &str| {
        task.obligations
            .iter()
            .filter(|o| o.status == status)
            .collect::<Vec<_>>()
    };
    let (settling, running, failed) = (with("settling"), with("open"), with("failed"));
    if settling.is_empty() && running.is_empty() && failed.is_empty() {
        return "Every tracked obligation is already resolved. Reply now with the final answer to the original request. No chariox.events call is needed.".into();
    }
    let mut text = String::from("The prior answer is progress, not completion.");
    if !settling.is_empty() {
        text.push_str(&format!(" Completion has already arrived for these obligations: {}. Read chariox.events.inbox and call chariox.events.ack with handled:true for each completion receipt you have reviewed; that settles them.", serde_json::to_string(&settling).unwrap_or_default()));
    }
    if !running.is_empty() {
        text.push_str(&format!(" These are still running: {}. Finish or cancel them, or call chariox.events.yield with their registration_ids (chariox.events.wakes lists them) and a future deadline_ms. A provider Idle or assistant-message-completed does not close these obligations. While those registrations are live, yield on them instead of answering. After the registered source_completed receipt arrives, review it in chariox.events.inbox and call chariox.events.ack with handled:true; only then report the final answer.", serde_json::to_string(&running).unwrap_or_default()));
    }
    if !failed.is_empty() {
        text.push_str(&format!(" These failed: {}. Handle the failure, or call chariox.events.blocked with the exact owner action.", serde_json::to_string(&failed).unwrap_or_default()));
    }
    text.push_str(" If everything has resolved by now, reply with the final answer instead. One correction is allowed.");
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    fn correction_task(statuses: &[&str]) -> AgentTaskExecution {
        let obligations = statuses
            .iter()
            .enumerate()
            .map(|(i, status)| serde_json::json!({"id":format!("ob-{i}"),"kind":"delegate","resource_id":format!("agent-{i}"),"status":status,"dispatch_state":"accepted"}))
            .collect::<Vec<_>>();
        serde_json::from_value(serde_json::json!({"task_id":"t","room_id":"r","agent_id":"a","prompt_id":"t","provider_run_id":"run","revision":1,"state":"working","reason":"","obligations":obligations,"wait":null,"last_progress_at_ms":1,"progress_sequence":0,"no_progress_wakes":0,"correction_used":true,"pending_prompt_id":"task-correction-t"})).unwrap()
    }
    #[test]
    fn r16_task_notice_uses_the_agent_name_without_protocol_details() {
        let mut task = correction_task(&["open"]);
        task.reason = "Evaluating original goal objective".into();
        assert_eq!(
            task_status_notice(&task, "Researcher"),
            "Researcher: Working."
        );
        task.state = ExecutionState::Done;
        assert_eq!(task_status_notice(&task, "Researcher"), "Researcher: Done.");
    }
    #[test]
    fn correction_with_no_open_obligation_asks_only_for_the_final_answer() {
        let text = task_correction_text(&correction_task(&["satisfied", "cancelled"]));
        assert!(text.contains("final"), "{text}");
        assert!(
            !text.contains("yield") && !text.contains("blocked") && !text.contains("ob-0"),
            "{text}"
        );
    }
    // MP-08 / MP-10 / MP-11: a real parent was blocked with a reply obligation
    // stuck settling after its child answered; the correction must name the ack.
    #[test]
    fn correction_tells_a_settling_obligation_to_acknowledge_its_receipt() {
        let text = task_correction_text(&correction_task(&["satisfied", "settling"]));
        assert!(text.contains("ob-1") && !text.contains("ob-0"), "{text}");
        assert!(
            text.contains("chariox.events.inbox") && text.contains("chariox.events.ack"),
            "{text}"
        );
        assert!(!text.contains("chariox.events.yield"), "{text}");
    }
    // MP-08 / MP-10 / MP-11: live Codex read the reply while the child
    // was still confirming completion, then ended its sole correction early.
    #[test]
    fn r16_running_source_correction_covers_the_later_completion_receipt() {
        let text = task_correction_text(&correction_task(&["open"]));
        assert!(text.contains("provider Idle") && text.contains("registrations are live"));
        assert!(text.contains("source_completed") && text.contains("chariox.events.inbox"));
        assert!(text.contains("chariox.events.ack with handled:true"));
    }
    #[test]
    fn correction_lists_only_unresolved_obligations_and_names_the_wait_route() {
        let text = task_correction_text(&correction_task(&["satisfied", "open"]));
        assert!(text.contains("ob-1") && !text.contains("ob-0"), "{text}");
        assert!(
            text.contains("chariox.events.yield") && text.contains("registration"),
            "{text}"
        );
    }
}
