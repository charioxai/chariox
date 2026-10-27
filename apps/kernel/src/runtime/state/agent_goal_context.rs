//! MP-08 / MP-09 / MP-10 / MP-11 A12-S02: safe native assessment and continuation context.
use super::*;
use crate::durable_state::agent_lifecycle::{self as ledger, AgentTaskExecution};

struct GoalWorkEvidence {
    prompt: String,
    run: Option<String>,
    blocked: Option<serde_json::Value>,
}

impl KernelRuntimeOwnedState {
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
        let page = self.durable_state_store.agent_work_inbox(
            &task.room_id,
            &task.agent_id,
            0,
            Some(&task.task_id),
        )?;
        let next_after = page.last().map_or(0, |e| e.sequence);
        let actionable = |event: &&ledger::InboxEvent| {
            !matches!(event.state.as_str(), "handled" | "cancelled" | "expired")
                || (event.state == "expired" && ledger::unresolved_source_outcome(task, event))
        };
        let receipt_metadata = |e: &ledger::InboxEvent| {
            serde_json::json!({
                "sequence": e.sequence, "source_id": e.source_id,
                "occurrence_id": e.occurrence_id, "kind": e.kind, "state": e.state,
                "prompt_id": e.prompt_id, "target_prompt_id": e.target_prompt_id,
            })
        };
        let receipts = page
            .iter()
            .filter(actionable)
            .take(64)
            .map(receipt_metadata)
            .collect::<Vec<_>>();
        let receipt_total = page.iter().filter(actionable).count();
        let receipt_coverage = serde_json::json!({
            "shown":receipts.len(), "page_total":receipt_total,
            "page_complete":page.len() < 128, "next_after":next_after,
            "public_ref":format!("events:{}:{}:{}", task.room_id, task.agent_id, task.task_id),
        });
        let context = if let Some(check) = evaluation {
            let handled_source =
                |e: &&ledger::InboxEvent| e.state == "handled" && e.kind == "source_completed";
            let handled_source_receipts = page
                .iter()
                .filter(handled_source)
                .take(64)
                .map(receipt_metadata)
                .collect::<Vec<_>>();
            let satisfied_obligations = task
                .obligations
                .iter()
                .filter(|o| o.status == "satisfied")
                .take(64)
                .collect::<Vec<_>>();
            let completed_coverage = serde_json::json!({
                "receipts_shown": handled_source_receipts.len(),
                "receipts_total": page.iter().filter(handled_source).count(),
                "obligations_shown": satisfied_obligations.len(),
                "obligations_total": task.obligations.iter().filter(|o| o.status == "satisfied").count(),
            });
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
            let original = task_objective(task, &public, true).ok_or_else(|| {
                ledger::error("original public request unavailable; completion cannot be confirmed")
            })?;
            let original_total = original.chars().count();
            let original_excerpt = original.clone();
            let answer = super::agent_task_projection::task_public_answer(
                &public,
                &check.work_prompt_id,
                check.work_provider_run_id.as_deref(),
                4096,
            );
            let (messages, shown_chars, total_chars) =
                (answer.messages, answer.shown_chars, answer.total_chars);
            let events = self.durable_state_store.agent_goal_evidence_events(task)?;
            let journal_complete = events.len() < 129;
            let mut prior_turns = Vec::new();
            let mut owner_recovery = Vec::new();
            for event in events.iter().rev().take(128).rev() {
                let data = &event.payload;
                match event.kind.as_str() {
                    "goal.evaluation" => {
                        let previous: ledger::goals::GoalEvaluation =
                            serde_json::from_value(data["evaluation"].clone()).map_err(|_| ledger::error("goal evidence binding is corrupt; inspect the ledger"))?;
                        if previous.result.is_some()
                            && previous.objective_revision == check.objective_revision
                            && previous.ledger_revision <= task.revision
                            && previous.work_prompt_id != check.work_prompt_id
                        {
                            prior_turns.push(GoalWorkEvidence {
                                prompt: previous.work_prompt_id,
                                run: previous.work_provider_run_id,
                                blocked: None,
                            });
                        }
                    }
                    "goal.blocked" => {
                        let prompt = data["work_prompt_id"].as_str().ok_or_else(|| ledger::error("blocked goal evidence binding is corrupt; inspect the ledger"))?;
                        if prompt != check.work_prompt_id {
                            prior_turns.push(GoalWorkEvidence {
                                prompt: prompt.to_owned(),
                                run: data["work_provider_run_id"].as_str().map(str::to_owned),
                                blocked: Some(serde_json::json!({
                                    "reason":data["reason"], "reason_complete":data["reason_complete"],
                                    "blocked_revision":data["blocked_revision"],
                                    "public_ref":format!("ledger:{}",event.sequence),
                                })),
                            });
                        }
                    },
                    "goal.owner_resume" => owner_recovery.push(serde_json::json!({
                        "action":"resume", "prompt_id":data["prompt_id"],
                        "blocked_revision":data["blocked_revision"], "resumed_revision":data["resumed_revision"],
                        "at_ms":data["at_ms"], "public_ref":format!("ledger:{}",event.sequence),
                    })),
                    "kernel_access.sudo" => owner_recovery.push(serde_json::json!({
                        "action":"sudo_started", "prompt_id":data["turn"]["prompt_id"],
                        "public_ref":format!("ledger:{}",event.sequence),
                    })),
                    _ => {},
                }
            }
            let mut seen_turns = std::collections::HashSet::new();
            prior_turns.reverse();
            prior_turns.retain(|turn| seen_turns.insert((turn.prompt.clone(), turn.run.clone())));
            let prior_total = prior_turns.len();
            prior_turns.truncate(32);
            prior_turns.reverse();
            let protected = self.room_secret_observations.protects_bytes(&task.room_id);
            let evidence_policy = if protected {
                "Assess the ORIGINAL objective once using the safe metadata below and exact kernel-private preceding result supplied separately to this neutral tool-free utility. No prior work-provider thread or workspace context is available here. If that private evidence is absent or incomplete, return blocked with the missing context. Privacy withholding alone is never evidence of success. Never quote protected values or private prose. Protected leased outcomes absent from the home provider context cannot establish success. Unmet objectives and unresolved obligations still forbid achieved."
            } else {
                "Assess the ORIGINAL objective once using only public evidence below."
            };
            // Coverage is a kernel fact; a short requested result is not a cut-off tail.
            let answer_complete = !protected && shown_chars == total_chars;
            let answer_truncated = (!protected).then_some(shown_chars < total_chars);
            let mut prior_work_results = Vec::new();
            let mut completed_calls = Vec::new();
            let mut omitted_calls = 0;
            let mut prior_budget = 4096;
            if !protected {
                // Allocate the shared prior-answer budget newest first, then
                // present work in chronological order. Receipts use the same bindings.
                for turn in prior_turns.iter().rev() {
                    let answer = super::agent_task_projection::task_public_answer(
                        &public,
                        &turn.prompt,
                        turn.run.as_deref(),
                        prior_budget,
                    );
                    prior_budget -= answer.shown_chars;
                    prior_work_results.push(serde_json::json!({
                        "work_prompt_id":turn.prompt, "answer_messages":answer.messages,
                        "blocked":turn.blocked,
                        "answer_coverage":{"shown_chars":answer.shown_chars,"total_chars":answer.total_chars,
                            "complete":answer.shown_chars==answer.total_chars},
                        "public_ref":format!("history:{}:prompt:{}",task.room_id,turn.prompt),
                    }));
                }
                prior_work_results.reverse();
                for (prompt, run) in prior_turns
                    .iter()
                    .map(|t| (t.prompt.as_str(), t.run.as_deref()))
                    .chain(std::iter::once((
                        check.work_prompt_id.as_str(),
                        check.work_provider_run_id.as_deref(),
                    )))
                {
                    let (calls, omitted) = completed_kernel_calls(&public, prompt, run);
                    omitted_calls += omitted;
                    completed_calls.extend(calls);
                }
                if completed_calls.len() > 64 {
                    omitted_calls += completed_calls.len() - 64;
                    completed_calls.drain(..completed_calls.len() - 64);
                }
            }
            let omitted_calls =
                (!protected && journal_complete && prior_turns.len() == prior_total)
                    .then_some(omitted_calls);
            let obligations = task
                .obligations
                .iter()
                .filter(|o| o.requires_resolution())
                .take(64)
                .collect::<Vec<_>>();
            // A refusal is settled work, but remains evidence against a claimed effect.
            // The ledger records rejection, not the underlying private provider error.
            let rejected = task
                .obligations
                .iter()
                .filter(|o| o.dispatch_state == "rejected");
            let rejected_total = rejected.clone().count();
            let rejected_dispatches = rejected
                .take(64)
                .map(|o| {
                    serde_json::json!({
                        "receipt_id": o.id, "kind": o.kind, "target": o.resource_id,
                        "reason_class": "dispatch_rejected", "effect": "not_admitted",
                    })
                })
                .collect::<Vec<_>>();
            let registrations = self
                .durable_state_store
                .agent_registrations(&task.task_id)?;
            let registration_total = registrations.len();
            let registrations = registrations.into_iter().take(64).collect::<Vec<_>>();
            self.trace_assessment_control(&task.room_id, "goal_assessment_input", serde_json::json!({
                "task_id":task.task_id, "prompt_id":check.prompt_id,
                "objective_revision":check.objective_revision,
                "registrations":registrations.iter().map(|r| serde_json::json!({
                    "id":(r.id.chars().count()<=256).then_some(&r.id),
                    "id_omitted":r.id.chars().count()>256, "live":r.live,
                })).collect::<Vec<_>>(),
                "registration_coverage":{"shown":registrations.len(),"total":registration_total},
            }));
            let total = task
                .obligations
                .iter()
                .filter(|o| o.requires_resolution())
                .count();
            let now_ms = crate::session::unix_epoch_ms();
            let waiting_deadline_ms =
                now_ms.saturating_add(u64::from(goal.map_or(10, |g| g.check_in_minutes)) * 60_000);
            format!(
                "This turn has origin completion_check. {evidence_policy} Do not perform work, invoke tools, delegate or mint authority. Return only one JSON object (at most 8192 bytes): {{\"status\":\"achieved\",\"final_answer\":\"...\",\"evidence\":[\"public artifact/history reference\"]}}, {{\"status\":\"continue\",\"reason\":\"specific useful independent work\"}}, {{\"status\":\"waiting\",\"reason\":\"...\",\"registration_ids\":[\"live registration\"],\"deadline_ms\":{waiting_deadline_ms}}}, or {{\"status\":\"blocked\",\"reason\":\"exact required owner action\"}}. Achieved requires no unresolved obligations or failed admitted work. rejected_dispatches is kernel-confirmed evidence of settled refusals: those dispatches admitted no work and produced no requested effect. An answer claiming that a rejected dispatch produced the requested effect cannot be achieved. A refusal report may be achieved only when the original objective is fulfilled by reporting that refusal and the answer does not claim the rejected effect succeeded. Otherwise choose continue for useful remaining work or blocked with the exact required owner action. rejected_dispatch_coverage reports shown and total refusals; omitted refusals cannot establish success. Rejection never establishes that the requested effect succeeded. This is the same agent's bounded confirmation of its preceding task, not an independent audit of every tool invocation. Private runtime/MCP tool records are intentionally omitted from public evidence; their absence does not make public answer coverage incomplete. Assess the returned public outcome and tracked obligations. Inactive registrations describe completed sources; they do not create unresolved work. Use the unresolved obligations and unhandled receipts below as the authority for what remains. Satisfied obligations and handled source-completion receipts are kernel-confirmed completion metadata; use them with the public outcome to verify that the requested source finished and was handled. Their raw tool payloads are intentionally omitted. A newly reported timestamp and exact accepted child-task reference can establish fresh completed work; do not demand another native tool call solely because private tool traces are omitted. A complete public result can support achieved without an additional public-tool-trace gate. For public answer_coverage, complete=true means shown_chars equals total_chars: the entire public answer is present; never treat it as truncated. A short answer requested by the owner, even one character, is complete when nothing was cut; judge whether that full answer satisfies the objective. Only shown_chars < total_chars means the public answer was truncated. Complete coverage alone does not establish objective success; an empty result is not evidence of success. Protected visibility has unknown public coverage and cannot establish success. Judge the public result at the owner's requested level of detail. When the owner explicitly asks for only a count or another short result, that complete result is the answer, not a plan or a missing report. Do not infer a missing tool invocation solely because its intentionally private trace is omitted. A contradicting recorded failure or an unresolved obligation still forbids achieved. completed_kernel_calls contains up to 64 recent kernel-recorded completed typed-call receipts across the current goal revision, with request names, call IDs, authority outcomes and public references only. Request parameters and result payloads are intentionally omitted. prior_work_results preserves completed work from this same objective revision across continuations; use those results and receipts together with the latest answer. A continuation does not invalidate earlier completed work. A prior_work_results blocked receipt confirms the preceding work stopped for the worker-reported owner action; owner_recovery confirms the owner then resumed this goal revision. Use that bound history to assess conditional recovery even when blocked work had no evaluation. Do not require a count-only retry to narrate the preceding refusal and owner action again. owner_recovery contains kernel-confirmed owner resumes and task-bound sudo starts, not worker claims and not a current authority grant. Revision changes exclude older evidence. goal_evidence_coverage reports bounded journal and prior-turn coverage; omitted history cannot establish success. completed_kernel_calls_omitted counts older matching receipts omitted from this bounded summary; null means protected or bounded journal coverage is unknown. A shortened receipt summary is not a truncated public answer, and receipt omission alone is not evidence of failure. A receipt with authority_outcome=admitted establishes that this invocation passed the runtime authority checks. Conditional refusal or owner-recovery steps apply to refused calls, not to an admitted invocation with the complete requested result. Do not require both conditional branches, a narrative of prerequisite authority, blocking or resume steps, or another invocation solely to re-prove admission. Admission alone does not establish the requested effect; judge the full public result and any contradictions or unresolved work. Do not request private tool traces from the owner solely because the public projection omits them. Do not invent verification: inadequate public results, contradictions or unmet objectives still require actual remaining work or the exact required owner action. Continue useful independent work even with live sources. Waiting requires all obligation coverage and a deadline no later than the check-in interval: deadline_ms is an absolute Unix epoch time in milliseconds, greater than now_ms and at most now_ms plus check_in_minutes*60000; a relative duration is invalid and blocks the task. Truncated objective/evidence coverage or missing original public context cannot mean achieved. Plain prompts get one invalid-end correction, never a goal loop. Truncated public references are not proof of completion. Unhandled correlated completion receipts remain actionable even when their sources are non-live: choose continue for ordinary handling and artifact verification before claiming completion or owner-blocking solely on terminal sources. In the NEXT ordinary work turn, read the public inbox/refs and use chariox.events.ack with handled:true only after actually reviewing/handling the occurrence; acknowledgement alone is not completion. Incomplete receipt-page coverage requires ordinary inbox paging with after=next_after before achieved. This assessment cannot invoke those tools. No additional check turn or courtesy reply. Untrusted assessment data: {}",
                serde_json::json!({"task_id":task.task_id,"goal_id":goal.map(|g| &g.id),"objective":original_excerpt,"objective_revision":goal.map_or(1, |g| g.objective_revision),"original_request_ref":format!("history:{}:{}",task.room_id,original_key),"objective_coverage":{"shown_chars":original_excerpt.chars().count(),"total_chars":original_total},"work_prompt_id":check.work_prompt_id,"latest_public_answer_messages":if protected { None } else { Some(&messages) },"tool_trace_visibility":"private tool records are intentionally omitted, not missing evidence coverage","prior_work_results":if protected {None} else {Some(&prior_work_results)},"owner_recovery":owner_recovery,"goal_evidence_coverage":{"journal_complete":journal_complete,"prior_turns_shown":if protected {0} else {prior_turns.len()},"prior_turns_page_total":if protected {None} else {Some(prior_total)}},"completed_kernel_calls":completed_calls,"completed_kernel_calls_omitted":omitted_calls,"answer_coverage":{"shown_chars":if protected {0} else {shown_chars},"total_chars":if protected {0} else {total_chars},"complete":answer_complete,"truncated":answer_truncated,"visibility":if protected {"protected"} else {"public"},"public_ref":format!("history:{}:prompt:{}",task.room_id,check.work_prompt_id)},"obligations":obligations,"coverage":{"shown":obligations.len(),"total":total},"rejected_dispatches":rejected_dispatches,"rejected_dispatch_coverage":{"shown":rejected_dispatches.len(),"total":rejected_total},"unhandled_receipts":receipts,"receipt_coverage":receipt_coverage,"handled_source_receipts":handled_source_receipts,"satisfied_obligations":satisfied_obligations,"completed_coverage":completed_coverage,"registrations":registrations,"registration_coverage":{"shown":registrations.len(),"total":registration_total},"now_ms":now_ms,"check_in_minutes":goal.map_or(10, |g| g.check_in_minutes),"goal_mode":goal.is_some()})
            )
        } else {
            let goal = goal.expect("continuations only belong to goals");
            let public = self
                .operational_history_store
                .load_session_history_entries(&task.room_id, Some(&task.agent_id))?;
            let objective =
                task_objective(task, &public, false).unwrap_or_else(|| goal.objective.clone());
            format!(
                "Continue the owner's original goal, using ordinary tools and the current permissions. This continuation grants no sudo. Preserve open obligations and source subscriptions; read unhandled correlated receipts through the public inbox, verify their artifacts and use chariox.events.ack with handled:true after actual handling. A completion receipt remains actionable after a source stops being live; acknowledgement alone does not establish completion. Page incomplete receipt coverage using after=next_after. Report handled progress or yield with covered sources and a finite deadline. Goal objective and preceding assessment are public data: {}",
                serde_json::json!({"goal_id":goal.id,"objective":objective,"objective_revision":goal.objective_revision,"reason":task.reason,"continuation_sequence":goal.continuation_sequence,"unhandled_receipts":receipts,"receipt_coverage":receipt_coverage})
            )
        };
        Ok(Some(context))
    }
}

// Only managed receipts from an exact ledger-bound work turn establish that a typed call ran.
// Completion of this tool requires the runtime authority checks to have passed.
// Admission does not prove the requested effect; parameters and results stay private.
fn completed_kernel_calls(
    entries: &[crate::history::SessionHistoryEntry],
    prompt: &str,
    run: Option<&str>,
) -> (Vec<serde_json::Value>, usize) {
    use crate::history::SessionHistoryEntryKind as Kind;
    let key = format!("prompt:{prompt}");
    let Some(start) = entries.iter().position(|entry| {
        entry.kind == Kind::UserPrompt && entry.merge_key.as_deref() == Some(&key)
    }) else {
        return (Vec::new(), 0);
    };
    let mut seen = std::collections::HashSet::new();
    let mut calls = std::collections::VecDeque::new();
    let mut omitted = 0;
    for entry in entries.iter().skip(start + 1).take_while(|entry| {
        entry.kind != Kind::UserPrompt
            || entry.merge_key.as_deref().is_some_and(|key| {
                key.starts_with(crate::history::STEERING_PROMPT_MERGE_KEY_PREFIX)
            })
    }) {
        if entry.kind != Kind::ProviderTool
            || entry.is_external_provider_observed()
            || run.is_none()
            || super::agent_task_projection::task_answer_history_run(
                entry.provider_run_id.as_deref(),
            ) != super::agent_task_projection::task_answer_history_run(run)
        {
            continue;
        }
        let Some((id, request)) = (|| {
            let tool: serde_json::Value = serde_json::from_str(&entry.text).ok()?;
            let name = tool.get("tool")?.as_str()?;
            let name = name.strip_prefix("mcp__chariox__").unwrap_or(name);
            if name != "chariox_kernel_request" || tool["status"] != "completed" {
                return None;
            }
            let request = tool.get("input")?.get("request")?;
            // Validate the operation tag without exposing any of its arguments.
            serde_json::from_value::<crate::local::LocalDaemonRequest>(request.clone()).ok()?;
            let request = request.as_object()?;
            if request.len() != 1 {
                return None;
            }
            Some((
                tool.get("id")?.as_str()?.to_owned(),
                request.keys().next()?.to_owned(),
            ))
        })() else {
            continue;
        };
        if !seen.insert(id.clone()) {
            continue;
        }
        calls.push_back(serde_json::json!({
            "request":request, "status":"completed", "call_id":id,
            "authority_outcome":"admitted",
            "public_ref":format!("history:{}:prompt:{prompt}", entry.session_id),
        }));
        if calls.len() > 64 {
            let _ = calls.pop_front();
            omitted += 1;
        }
    }
    (calls.into_iter().collect(), omitted)
}

/// The request the check and continuations judge against: the goal objective (or the typed
/// prompt), plus the notes that came with the original prompt when `with_notes` (the check
/// turn; continuations carry the notes as hidden context instead).
fn task_objective(
    task: &AgentTaskExecution,
    public: &[crate::history::SessionHistoryEntry],
    with_notes: bool,
) -> Option<String> {
    let goal = task.goal.as_ref().map(|g| g.objective.as_str());
    objective_of(&task.task_id, goal, public, with_notes)
}

fn original_prompt<'a>(
    task_id: &str,
    public: &'a [crate::history::SessionHistoryEntry],
) -> Option<&'a crate::history::SessionHistoryEntry> {
    let key = format!("prompt:{task_id}");
    public.iter().find(|e| {
        e.kind == crate::history::SessionHistoryEntryKind::UserPrompt
            && e.merge_key.as_deref() == Some(key.as_str())
    })
}

fn objective_of(
    task_id: &str,
    goal_objective: Option<&str>,
    public: &[crate::history::SessionHistoryEntry],
    with_notes: bool,
) -> Option<String> {
    let entry = original_prompt(task_id, public);
    let text = match (goal_objective, entry) {
        (Some(goal), _) => goal,
        (None, Some(entry)) => entry.text.as_str(),
        (None, None) => return None,
    };
    Some(match entry {
        Some(entry) if with_notes => {
            crate::prompt_notes::objective_with_notes(text, &entry.attachments)
        }
        _ => text.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::{completed_kernel_calls, objective_of};
    use base64::Engine;

    fn prompt(text: &str) -> crate::history::SessionHistoryEntry {
        let note = crate::session::PromptAttachment::new(
            "chariox-terminal://note/1",
            crate::prompt_notes::NOTE_ATTACHMENT_MIME,
            Some("Note".into()),
        )
        .with_contents_base64(
            base64::engine::general_purpose::STANDARD
                .encode(r#"{"quote":"old name","comment":"rename it to new_name"}"#),
        );
        let mut entry = crate::history::SessionHistoryEntry::user_prompt_with_attachments(
            "room",
            "att",
            "agent",
            text,
            &[note],
        );
        entry.merge_key = Some("prompt:task".into());
        entry
    }

    #[test]
    fn goal_and_plain_objectives_carry_the_notes_of_the_original_prompt() {
        let history = [prompt("apply my note")];
        for goal in [Some("/goal objective"), None] {
            let objective = objective_of("task", goal, &history, true).unwrap();
            assert!(objective.starts_with(goal.unwrap_or("apply my note")));
            assert!(objective.contains("rename it to new_name"));
            // A continuation's visible objective stays the plain request; the notes go hidden.
            let plain = objective_of("task", goal, &history, false).unwrap();
            assert_eq!(plain, goal.unwrap_or("apply my note"));
        }
        assert_eq!(
            objective_of("task", Some("typed goal"), &[], true).as_deref(),
            Some("typed goal")
        );
        assert_eq!(objective_of("task", None, &[], true), None);
    }
    #[test]
    fn completed_claude_kernel_call_has_a_typed_receipt() {
        // Minimal metadata from a recorded real Claude ListSessions receipt.
        let mut receipt: serde_json::Value =
            serde_json::from_str(include_str!("agent_goal_context_claude_call.json")).unwrap();
        receipt["output"] = serde_json::json!("PRIVATE_RESULT");
        let entry = crate::history::SessionHistoryEntry::provider_output(
            "room",
            "run",
            Some("agent"),
            crate::terminal::TerminalOutputKind::ProviderTool,
            None,
            receipt.to_string(),
        );
        let (calls, omitted) =
            completed_kernel_calls(&[prompt("count sessions"), entry], "task", Some("run"));
        assert_eq!(omitted, 0);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["request"], "ListSessions");
        assert_eq!(calls[0]["call_id"], "toolu_0127BXRSY51N3hMSVtn31aox");
        assert_eq!(calls[0]["authority_outcome"], "admitted");
        assert_eq!(calls[0].as_object().unwrap().len(), 5);
        assert!(!serde_json::to_string(&calls)
            .unwrap()
            .contains("PRIVATE_RESULT"));
    }
    #[test]
    fn completed_call_metadata_is_bound_deduplicated_and_has_no_payloads() {
        use crate::history::SessionHistoryEntry;
        use crate::terminal::TerminalOutputKind;
        let call = |id: &str, status: &str, run: &str, request: serde_json::Value| {
            SessionHistoryEntry::provider_output("room", run, Some("agent"), TerminalOutputKind::ProviderTool, None,
                serde_json::json!({"id":id,"tool":"chariox_kernel_request","status":status,"input":{"request":request},"output":"PRIVATE_RESULT"}).to_string())
        };
        let request = serde_json::json!({"GetDaemonHealth":null});
        let valid = call("count", "completed", "run", request.clone());
        let mut next = prompt("next work");
        next.merge_key = Some("prompt:next".into());
        let entries = vec![
            prompt("count sessions"),
            call("other-run", "completed", "other", request.clone()),
            call("failed", "error", "run", request.clone()),
            call(
                "invalid",
                "completed",
                "run",
                serde_json::json!({"UnknownRequest":null}),
            ),
            valid.clone(),
            valid,
            next,
            call("later", "completed", "run", request.clone()),
        ];
        let (calls, omitted) = completed_kernel_calls(&entries, "task", Some("run"));
        assert_eq!(omitted, 0);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["request"], "GetDaemonHealth");
        assert_eq!(calls[0]["call_id"], "count");
        assert_eq!(calls[0]["authority_outcome"], "admitted");
        assert_eq!(calls[0].as_object().unwrap().len(), 5);
        assert!(!serde_json::to_string(&calls)
            .unwrap()
            .contains("PRIVATE_RESULT"));
        assert!(completed_kernel_calls(&entries, "task", None).0.is_empty());
        let mut many = vec![prompt("count sessions")];
        many.extend(
            (0..66).map(|i| call(&format!("call-{i}"), "completed", "run", request.clone())),
        );
        let (calls, omitted) = completed_kernel_calls(&many, "task", Some("run"));
        assert_eq!(omitted, 2);
        assert_eq!(calls.len(), 64);
        assert_eq!(calls[0]["call_id"], "call-2");
        assert_eq!(calls[63]["call_id"], "call-65");
    }
}
