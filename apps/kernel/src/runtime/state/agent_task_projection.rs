//! MP-08 / MP-09 / MP-10 / MP-11 A02: bounded public task answer projection.
use super::*;
use crate::durable_state::agent_lifecycle::{self as ledger, AgentTaskExecution, ExecutionState};
/// The task whose live turn an artifact change counts as progress for. A completion check
/// cannot mutate work, so the change it happens to observe is not progress for it (the ledger
/// would refuse the receipt and the refusal would fail the whole output pump).
pub(super) fn artifact_progress_task(
    tasks: Vec<AgentTaskExecution>,
    prompt: &str,
    run: &str,
) -> Option<AgentTaskExecution> {
    tasks.into_iter().find(|t| {
        t.prompt_id == prompt
            && t.provider_run_id.as_deref() == Some(run)
            && t.state == ExecutionState::Working
            && t.pending_prompt_id.is_none()
            && !ledger::goals::is_check(t, prompt)
    })
}
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
            && snapshot.files_unchanged
        {
            return Ok(());
        }
        let Some(task) = artifact_progress_task(
            self.durable_state_store
                .agent_tasks(Some(room), Some(agent))?,
            prompt.id(),
            run,
        ) else {
            return Ok(());
        };
        let receipt = snapshot.artifact_receipt.clone();
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

    pub(super) fn public_agent_task_answer(
        &self,
        task: &AgentTaskExecution,
    ) -> Result<Option<serde_json::Value>, DaemonError> {
        self.public_agent_task_answer_after(task, None)
    }

    /// `failure` is the outcome of the turn that just ended the task, when the
    /// caller settled it.
    pub(super) fn public_agent_task_answer_after(
        &self,
        task: &AgentTaskExecution,
        failure: Option<&crate::provider::TurnOutcome>,
    ) -> Result<Option<serde_json::Value>, DaemonError> {
        if task.state == ExecutionState::Cancelled {
            return self.failed_task_answer(task, failure);
        }
        if task.state != ExecutionState::Done {
            return Ok(None);
        }
        if let Some(goal) = &task.goal {
            return Ok(Some(
                serde_json::json!({"goal_id":goal.id,"task_id":task.task_id,"excerpt":goal.final_answer,"evidence":goal.evidence}),
            ));
        }
        let entries = self
            .operational_history_store
            .load_session_history_entries(&task.room_id, Some(&task.agent_id))?
            .into_iter()
            .map(|entry| {
                self.room_secret_observations
                    .protect_transcript_entry(entry)
            })
            .collect::<Vec<_>>();
        // Output entries are streamed deltas; the conclusion is at the end.
        let output = task_public_outputs(
            &entries,
            &task.prompt_id,
            task_answer_history_run(task.provider_run_id.as_deref()),
        )
        .concat();
        let excerpt = answer_tail(output.trim(), 1_024);
        let answer = serde_json::json!({"agent_id":task.agent_id,"task_id":task.task_id,"prompt_id":task.prompt_id,"excerpt":excerpt});
        let mut answer = self.answer_with_running_background_note(task, Some(answer))?;
        if let Some(answer) = answer.as_mut() {
            crate::secret_redaction::redact_json_secrets(answer);
        }
        Ok(answer)
    }

    /// What a waiter reads when a task ends because its turn failed: the typed
    /// outcome the turn's history carries (so every settlement path reads the
    /// same one), what to do next and, for a leased helper, where it ran. A
    /// task the owner cancelled has no failed turn and no answer.
    fn failed_task_answer(
        &self,
        task: &AgentTaskExecution,
        failure: Option<&crate::provider::TurnOutcome>,
    ) -> Result<Option<serde_json::Value>, DaemonError> {
        // The settlement in hand wins; otherwise the turn's history has it. The
        // history append can trail the settlement by a moment.
        let recorded = if failure.is_some() {
            None
        } else {
            self.operational_history_store
                .load_prompt_settlement_event(&task.room_id, &task.agent_id, &task.prompt_id)?
                .and_then(|settlement| {
                    settlement
                        .metadata
                        .get(crate::history::TURN_OUTCOME_METADATA_KEY)
                        .and_then(|outcome| {
                            serde_json::from_value::<crate::provider::TurnOutcome>(outcome.clone())
                                .ok()
                        })
                })
        };
        let Some(outcome) = failure
            .cloned()
            .or(recorded)
            .filter(crate::provider::TurnOutcome::is_error)
        else {
            return Ok(None);
        };
        let mut answer = serde_json::json!({
            "agent_id": task.agent_id,
            "task_id": task.task_id,
            "prompt_id": task.prompt_id,
            "outcome": outcome,
            "recovery": outcome.notice(),
        });
        if let Some(binding) = self
            .agent_store
            .get_agent(&task.agent_id)
            .ok()
            .and_then(|agent| agent.remote_execution().cloned())
        {
            answer["host"] = self.leased_host_name(&binding).into();
        }
        crate::secret_redaction::redact_json_secrets(&mut answer);
        Ok(Some(answer))
    }

    /// The slice or worker kernel that hosts a leased agent, as users read it.
    pub(super) fn leased_host_name(&self, binding: &crate::agent::RemoteAgentBinding) -> String {
        match self
            .slice_store
            .resolve_by_worker_kernel_ref(&binding.worker_kernel_id)
        {
            Some(slice) => format!("slice `{}`", slice.name),
            None => format!("worker kernel `{}`", binding.worker_kernel_id),
        }
    }
}

/// Finished tasks a snapshot still carries; a long room must not grow every
/// snapshot by one task per turn.
pub(super) const VISIBLE_FINISHED_TASKS: usize = 32;

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

// Leased history entries carry either the worker's run ID or the projected
// `leased:<lease>:<worker run>` one. Resolve both to the worker run ID from the
// task's durable execution binding, so reconciliation does not depend on the
// current lease or active run. This is a history lookup only; the task's
// authority stays projected.
pub(super) fn task_answer_history_run(run: Option<&str>) -> Option<&str> {
    run.map(|run| {
        run.strip_prefix("leased:")
            .and_then(|binding| binding.split_once(':'))
            .filter(|(lease, worker)| !lease.is_empty() && !worker.is_empty())
            .map_or(run, |(_, worker)| worker)
    })
}

/// Assessment control is bound by the worker to the exact home check. A later
/// history boundary can contain delayed work bytes or an earlier check's reply.
pub(super) fn leased_assessment_reply(
    entries: &[crate::history::SessionHistoryEntry],
    prompt: &str,
    run: &str,
) -> String {
    let key = format!("completion-assessment:home:{prompt}");
    entries
        .iter()
        .filter(|entry| {
            entry.kind == crate::history::SessionHistoryEntryKind::ProviderOutput
                && entry.merge_key.as_deref() == Some(key.as_str())
                && task_answer_history_run(entry.provider_run_id.as_deref()) == Some(run)
        })
        .map(|entry| entry.text.as_str())
        .collect()
}

pub(super) fn answer_tail(text: &str, limit: usize) -> String {
    let count = text.chars().count();
    if count <= limit {
        return text.into();
    }
    let tail = text.chars().skip(count - limit + 1).collect::<String>();
    format!("…{tail}")
}

// The single correction for a turn that ended without a valid wait or answer.
// Only unresolved obligations are actionable, each with the step that settles
// it: a completion that already arrived is acknowledged, a running source is
// waited on, a failure is handled. With none left, the agent just answers.
pub(super) fn task_correction_text(task: &AgentTaskExecution) -> String {
    let with = |status: &str| {
        task.obligations
            .iter()
            .filter(|o| o.status == status && o.requires_resolution())
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
        text.push_str(&format!(" These are still running: {}. Finish or cancel them, or call chariox.events.yield with their registration_ids (chariox.events.subscriptions lists them) (a goal task adds a future deadline_ms). A child reply, provider Idle or assistant-message-completed does not close these obligations: the child may still be confirming its result. While those registrations are live, yield on them instead of answering. Their completion wakes you: it arrives as a child_finished or source_completed event, so do not poll. After the registered source_completed receipt arrives, review its public result in chariox.events.inbox and call chariox.events.ack with handled:true; only then report the final answer.", serde_json::to_string(&running).unwrap_or_default()));
    }
    if !failed.is_empty() {
        text.push_str(&format!(" These failed: {}. Handle the failure, or call chariox.events.blocked with the exact owner action.", serde_json::to_string(&failed).unwrap_or_default()));
    }
    text.push_str(&format!(" Settle each by this exact call: {}.", task_settle_steps(task).join("; ")));
    text.push_str(" If everything has resolved by now, reply with the final answer instead. One correction is allowed.");
    text
}

/// What the task still owes, one step per unresolved obligation with the exact call that settles it.
/// The correction prompt, the blocked card and the Resume instruction all say it the same way.
fn task_settle_steps(task: &AgentTaskExecution) -> Vec<String> {
    task.obligations
        .iter()
        .filter(|o| o.requires_resolution())
        .map(|o| {
            let resource = o.resource_id.as_deref().unwrap_or(&o.id);
            let what = format!("{} {resource}", o.kind);
            let wait = format!(
                r#"call chariox.events.yield with registration_ids ["completion-{}"]"#,
                o.id
            );
            match (o.status.as_str(), o.kind.as_str()) {
                ("settling", _) => format!("{what} has completed: call chariox.events.inbox, review it, then call chariox.events.ack with handled:true"),
                ("failed", _) => format!("{what} failed: handle the failure, or call chariox.events.blocked with the exact owner action"),
                (_, "timer" | "process") => format!(r#"{what} is still running: {wait}, or end it with chariox.events.cancel_wake with wake_id "{resource}""#),
                _ => format!("{what} is still running: {wait}, or stop it with the room command agent cancel"),
            }
        })
        .collect()
}

fn missing_wake_handling(reason: &str) -> &'static str {
    if reason.starts_with("Three consecutive wakes") {
        " Missing: the wake events were not handled. Read chariox.events.inbox and call chariox.events.ack with handled:true for each wake you have handled."
    } else {
        ""
    }
}

/// What a task that its owner resumed is told: why it was blocked and the step that settles what it owes.
pub(super) fn task_resume_text(task: &AgentTaskExecution, reason: &str) -> String {
    let steps = task_settle_steps(task);
    let mut text = format!(
        "This task was blocked ({}) and has now been resumed.",
        crate::terminal::public_task_failure_reason(reason)
    );
    if steps.is_empty() {
        text.push_str(" Nothing is left open: reply now with the final answer to the original request; no chariox.events call is needed.");
    } else {
        text.push_str(&format!(
            " It still owes: {}. Settle each by this exact call, then reply with the final answer.",
            steps.join("; ")
        ));
    }
    text.push_str(missing_wake_handling(reason));
    text
}

/// The text of a blocked task's Resume/Cancel card.
pub(super) fn blocked_card_message(alias: Option<&str>, task: &AgentTaskExecution) -> String {
    let reason = crate::terminal::public_task_failure_reason(&task.reason);
    let mut text = match alias {
        Some(alias) => format!("{alias}: {reason}"),
        None => reason,
    };
    let owed: Vec<_> = task
        .obligations
        .iter()
        .filter(|o| o.requires_resolution())
        .map(|o| format!("{} {}", o.kind, o.resource_id.as_deref().unwrap_or(&o.id)))
        .collect();
    if !owed.is_empty() {
        text.push_str(&format!(" Open: {}.", owed.join(", ")));
    }
    text.push_str(missing_wake_handling(&task.reason));
    text
}

// Steering belongs to the current turn; an independent prompt closes its answer boundary.
pub(super) fn task_public_outputs<'a>(
    entries: &'a [crate::history::SessionHistoryEntry],
    prompt: &str,
    run: Option<&str>,
) -> Vec<&'a str> {
    task_public_output_entries(entries, prompt, run)
        .into_iter()
        .map(|entry| entry.text.as_str())
        .collect()
}

pub(super) fn task_public_output_entries<'a>(
    entries: &'a [crate::history::SessionHistoryEntry],
    prompt: &str,
    run: Option<&str>,
) -> Vec<&'a crate::history::SessionHistoryEntry> {
    use crate::history::SessionHistoryEntryKind as Kind;
    let key = format!("prompt:{prompt}");
    let Some(start) = entries
        .iter()
        .position(|e| e.kind == Kind::UserPrompt && e.merge_key.as_deref() == Some(&key))
    else {
        return vec![];
    };
    // MP-08 / MP-09 / MP-10 / MP-11 A12: an official-provider history
    // import can replay a previous item under the current reused run. Prefer
    // its managed stream receipt, wherever its original prompt boundary was.
    // Keep genuine native-TUI imports whose item has no managed counterpart.
    let managed_items = entries
        .iter()
        .filter(|e| e.kind == Kind::ProviderOutput && !e.is_external_provider_observed())
        .filter_map(|e| e.merge_key.as_deref())
        .collect::<std::collections::HashSet<_>>();
    entries
        .iter()
        .skip(start + 1)
        .take_while(|e| {
            e.kind != Kind::UserPrompt
                || e.merge_key.as_deref().is_some_and(|key| {
                    key.starts_with(crate::history::STEERING_PROMPT_MERGE_KEY_PREFIX)
                })
        })
        .filter(|e| {
            e.kind == Kind::ProviderOutput
                && run.is_some()
                && task_answer_history_run(e.provider_run_id.as_deref())
                    == task_answer_history_run(run)
        })
        .filter(|e| {
            !e.is_external_provider_observed()
                || e.merge_key
                    .as_deref()
                    .and_then(crate::history::parse_external_provider_observed_id)
                    .is_none_or(|id| !managed_items.contains(id.provider_turn_id.as_str()))
        })
        .collect()
}

/// The same bounded message projection is used for current and prior goal work.
pub(super) struct PublicTaskAnswer {
    pub messages: Vec<String>,
    pub shown_chars: usize,
    pub total_chars: usize,
}

pub(super) fn task_public_answer(
    entries: &[crate::history::SessionHistoryEntry],
    prompt: &str,
    run: Option<&str>,
    limit: usize,
) -> PublicTaskAnswer {
    let output = task_public_output_entries(entries, prompt, task_answer_history_run(run));
    let total_chars = output.iter().map(|e| e.text.chars().count()).sum();
    let mut remaining = limit;
    let mut messages: Vec<(Option<&str>, String)> = Vec::new();
    // IDs distinguish commentary from a later answer. Without an ID, keep
    // adjacent deltas together instead of inventing message boundaries.
    for entry in output.into_iter().rev() {
        if remaining == 0 {
            break;
        }
        let count = entry.text.chars().count();
        let shown = count.min(remaining);
        if shown == 0 {
            continue;
        }
        let tail = entry.text.chars().skip(count - shown).collect::<String>();
        let key = entry.merge_key.as_deref();
        match messages.last_mut() {
            Some((last_key, text)) if *last_key == key => text.insert_str(0, &tail),
            _ => messages.push((key, tail)),
        }
        remaining -= shown;
    }
    PublicTaskAnswer {
        messages: messages.into_iter().rev().map(|(_, text)| text).collect(),
        shown_chars: limit - remaining,
        total_chars,
    }
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
    fn owing_task(obligations: serde_json::Value, reason: &str) -> AgentTaskExecution {
        serde_json::from_value(serde_json::json!({"task_id":"t","room_id":"r","agent_id":"a","prompt_id":"t","provider_run_id":"run","revision":1,"state":"blocked","reason":reason,"obligations":obligations,"wait":null,"last_progress_at_ms":1,"progress_sequence":0,"no_progress_wakes":3,"correction_used":true,"pending_prompt_id":null})).unwrap()
    }
    fn timer_and_child() -> serde_json::Value {
        serde_json::json!([
            {"id":"wake-7","kind":"timer","resource_id":"wake-7","status":"open","dispatch_state":"accepted"},
            {"id":"obligation-9","kind":"delegate","resource_id":"agent-9","status":"open","dispatch_state":"accepted"},
            {"id":"wake-1","kind":"timer","resource_id":"wake-1","status":"satisfied","dispatch_state":"accepted"}
        ])
    }
    // MP-08 / MP-10 / MP-11 (L10 item 4): the correction names what is owed and the call that settles it.
    #[test]
    fn correction_names_each_open_obligation_with_the_exact_call() {
        let text = task_correction_text(&owing_task(timer_and_child(), ""));
        assert!(text.contains("wake-7") && text.contains("obligation-9"), "{text}");
        assert!(!text.contains("wake-1"), "{text}");
        assert!(
            text.contains(r#"chariox.events.yield with registration_ids ["completion-wake-7"]"#),
            "{text}"
        );
        assert!(
            text.contains(r#"chariox.events.cancel_wake with wake_id "wake-7""#),
            "{text}"
        );
        assert!(
            text.contains(r#"registration_ids ["completion-obligation-9"]"#),
            "{text}"
        );
    }
    #[test]
    fn resume_text_carries_the_reason_and_the_exact_step() {
        let text = task_resume_text(
            &owing_task(timer_and_child(), ""),
            "Repeated invalid turn end; owner must resume or cancel",
        );
        assert!(text.contains("Repeated invalid turn end"), "{text}");
        assert!(text.contains("wake-7") && text.contains("chariox.events.yield"), "{text}");
        let none = task_resume_text(&owing_task(serde_json::json!([]), ""), "blocked");
        assert!(none.contains("final answer") && !none.contains("chariox.events.yield"), "{none}");
    }
    #[test]
    fn the_blocked_card_names_what_is_owed_and_what_is_missing() {
        let card = blocked_card_message(Some("c1"), &owing_task(timer_and_child(), "Repeated invalid turn end; owner must resume or cancel"));
        assert!(card.starts_with("c1: Repeated invalid turn end"), "{card}");
        assert!(card.contains("wake-7") && card.contains("obligation-9"), "{card}");
        let wakes = blocked_card_message(
            None,
            &owing_task(
                serde_json::json!([]),
                "Three consecutive wakes without handled progress; owner must resume or cancel",
            ),
        );
        assert!(
            wakes.contains("chariox.events.inbox") && wakes.contains("chariox.events.ack"),
            "{wakes}"
        );
    }
    #[test]
    fn a02_public_answer_survives_steering_without_borrowing_later_tasks() {
        let entry = |kind: &str, key: Option<&str>, run: Option<&str>, text: &str| {
            serde_json::from_value(serde_json::json!({"session_id":"room", "kind":kind,
                "merge_key":key,"provider_run_id":run,"text":text,"timestamp_ms":1}))
            .unwrap()
        };
        let entries = vec![
            entry("user_prompt", Some("prompt:current"), None, "task"),
            entry(
                "provider_output",
                None,
                Some("run"),
                "answer before steering",
            ),
            entry(
                "user_prompt",
                Some("steering-prompt:urgent"),
                None,
                "urgent",
            ),
            entry(
                "provider_output",
                None,
                Some("foreign-run"),
                "foreign answer",
            ),
            entry("user_prompt", Some("prompt:later"), None, "later task"),
            entry("provider_output", None, Some("run"), "later answer"),
        ];
        assert_eq!(
            task_public_outputs(&entries, "current", Some("run")),
            vec!["answer before steering"]
        );
        assert!(task_public_outputs(&entries, "absent", Some("run")).is_empty());
        assert!(task_public_outputs(&entries, "current", None).is_empty());
    }
    // MP-08 / MP-09 / MP-10 / MP-11 A12-S02: live d4 check contamination.
    #[test]
    fn a12_native_check_excludes_imported_replay_but_keeps_native_tui_answers() {
        let entry = |kind: &str, key: &str, text: &str, observed: bool| {
            serde_json::from_value(serde_json::json!({
                "session_id":"room", "kind":kind, "merge_key":key,
                "provider_run_id":"provider-run-2", "text":text,"timestamp_ms":1,
                "source":if observed {Some("external_provider_observed")} else {None},
            }))
            .unwrap()
        };
        let entries = vec![
            entry("user_prompt", "prompt:old", "original work", false),
            entry("provider_output", "msg_old", "preceding answer", false),
            entry("user_prompt", "prompt:check", "native assessment", false),
            entry(
                "provider_output",
                "external:codex:thread:msg_old",
                "preceding answer",
                true,
            ),
            entry(
                "provider_output",
                "msg_check",
                r#"{"status":"achieved"}"#,
                false,
            ),
            entry(
                "provider_output",
                "external:codex:thread:msg_check",
                r#"{"status":"achieved"}"#,
                true,
            ),
            entry(
                "user_prompt",
                "prompt:native",
                "real native TUI prompt",
                true,
            ),
            entry(
                "provider_output",
                "external:codex:thread:msg_native",
                "native answer",
                true,
            ),
        ];
        assert_eq!(
            task_public_outputs(&entries, "check", Some("provider-run-2")).concat(),
            r#"{"status":"achieved"}"#
        );
        assert_eq!(
            task_public_outputs(&entries, "native", Some("provider-run-2")),
            vec!["native answer"]
        );
    }

    // MP-08/MP-10/MP-11 R947: persisted completion/sweep lookup uses the
    // task's old worker run even after another lease or turn becomes current.
    #[test]
    fn a10_public_answer_resolves_durable_worker_run_without_live_binding() {
        assert_eq!(
            task_answer_history_run(Some("leased:old-lease:old-run")),
            Some("old-run")
        );
        assert_eq!(
            task_answer_history_run(Some("leased:old-lease:worker:run")),
            Some("worker:run")
        );
        for run in ["local-run", "leased:", "leased::worker", "leased:lease:"] {
            assert_eq!(task_answer_history_run(Some(run)), Some(run));
        }
        assert_eq!(task_answer_history_run(None), None);
        let entry = |kind: &str, key: Option<&str>, run: Option<&str>, text: &str| {
            serde_json::from_value(serde_json::json!({"session_id":"room", "kind":kind,
                "merge_key":key,"provider_run_id":run,"text":text,"timestamp_ms":1}))
            .unwrap()
        };
        let entries = vec![
            entry("user_prompt", Some("prompt:completed"), None, "task"),
            entry(
                "provider_output",
                None,
                Some("old-run"),
                "verified worker answer",
            ),
            entry(
                "provider_output",
                None,
                Some("current-run"),
                "foreign answer",
            ),
            entry("user_prompt", Some("prompt:later"), None, "later task"),
            entry("provider_output", None, Some("old-run"), "later answer"),
        ];
        assert_eq!(
            task_public_outputs(
                &entries,
                "completed",
                task_answer_history_run(Some("leased:old-lease:old-run"))
            ),
            vec!["verified worker answer"]
        );
    }

    // A leased turn's home history may carry the projected run id the task binds; the answer
    // must be found under it too, or every leased turn ends Blocked "Repeated invalid turn end".
    #[test]
    fn leased_answer_is_found_under_the_projected_run_id() {
        let projected = "leased:old-lease:old-run";
        let entry = |kind: &str, key: Option<&str>, run: Option<&str>, text: &str| {
            serde_json::from_value(serde_json::json!({"session_id":"room", "kind":kind,
                "merge_key":key,"provider_run_id":run,"text":text,"timestamp_ms":1}))
            .unwrap()
        };
        let entries = vec![
            entry("user_prompt", Some("prompt:completed"), None, "task"),
            entry("provider_output", None, Some(projected), "projected answer"),
            entry("provider_output", None, Some("old-run"), "worker answer"),
            entry(
                "provider_output",
                None,
                Some("leased:other:other-run"),
                "foreign",
            ),
        ];
        for bound in [projected, "old-run"] {
            assert_eq!(
                task_public_outputs(&entries, "completed", Some(bound)),
                vec!["projected answer", "worker answer"]
            );
        }
    }

    #[test]
    fn a02_public_answer_keeps_streamed_conclusion() {
        let streamed = ["I", "’ll", " check.", " Done: all tools exist."].concat();
        assert_eq!(
            answer_tail(&streamed, 64),
            "I’ll check. Done: all tools exist."
        );
        let tail = answer_tail(&streamed, 12);
        assert_eq!(tail, "…ools exist.");
        assert_eq!(tail.chars().count(), 12);
    }
}
