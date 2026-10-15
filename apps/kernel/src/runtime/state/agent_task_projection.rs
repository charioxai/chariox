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

    pub(super) fn public_agent_task_answer(
        &self,
        task: &AgentTaskExecution,
    ) -> Result<Option<serde_json::Value>, DaemonError> {
        if task.state != ExecutionState::Done {
            return Ok(None);
        }
        if let Some(goal) = &task.goal {
            return Ok(Some(
                serde_json::json!({"goal_id":goal.id,"task_id":task.task_id,"excerpt":goal.final_answer,"evidence":goal.evidence}),
            ));
        }
        if let Some(check) = &task.completion_check {
            let mut answer = serde_json::json!({"agent_id":task.agent_id,"task_id":task.task_id,"prompt_id":check.original_prompt_id,"excerpt":check.final_answer,"evidence":check.evidence});
            crate::secret_redaction::redact_json_secrets(&mut answer);
            return Ok(Some(answer));
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
        let mut answer = serde_json::json!({"agent_id":task.agent_id,"task_id":task.task_id,"prompt_id":task.prompt_id,"excerpt":excerpt});
        crate::secret_redaction::redact_json_secrets(&mut answer);
        Ok(Some(answer))
    }
}

// Leased history retains the worker run ID. Resolve it from the task's durable
// execution binding, so reconciliation does not depend on the current lease or
// active run. This is a history lookup only; the task's authority stays projected.
pub(super) fn task_answer_history_run(run: Option<&str>) -> Option<&str> {
    run.map(|run| {
        run.strip_prefix("leased:")
            .and_then(|binding| binding.split_once(':'))
            .filter(|(lease, worker)| !lease.is_empty() && !worker.is_empty())
            .map_or(run, |(_, worker)| worker)
    })
}

fn answer_tail(text: &str, limit: usize) -> String {
    let count = text.chars().count();
    if count <= limit {
        return text.into();
    }
    let tail = text.chars().skip(count - limit + 1).collect::<String>();
    format!("…{tail}")
}

// The single correction for a turn that ended without a valid wait or answer.
// Only unresolved obligations are actionable; with none, the agent just answers.
pub(super) fn task_correction_text(task: &AgentTaskExecution) -> String {
    let open = task
        .obligations
        .iter()
        .filter(|o| matches!(o.status.as_str(), "open" | "settling" | "failed"))
        .collect::<Vec<_>>();
    if open.is_empty() {
        return "Every tracked obligation is already resolved and the prior turn ended without a final answer. Reply now with the final answer to the original request. No chariox.events call is needed.".into();
    }
    format!("The prior answer is progress, not completion. These obligations are unresolved: {}. Finish or cancel them, or call chariox.events.yield with their registration_ids (chariox.events.subscriptions lists them) and a future deadline_ms. Otherwise call chariox.events.blocked with the exact owner action. If they have all resolved by now, reply with the final answer instead. One correction is allowed.",serde_json::to_string(&open).unwrap_or_default())
}

// Steering belongs to the current turn; an independent prompt closes its answer boundary.
pub(super) fn task_public_outputs<'a>(
    entries: &'a [crate::history::SessionHistoryEntry],
    prompt: &str,
    run: Option<&str>,
) -> Vec<&'a str> {
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
            e.kind == Kind::ProviderOutput && run.is_some() && e.provider_run_id.as_deref() == run
        })
        .filter(|e| {
            !e.is_external_provider_observed()
                || e.merge_key
                    .as_deref()
                    .and_then(crate::history::parse_external_provider_observed_id)
                    .is_none_or(|id| !managed_items.contains(id.provider_turn_id.as_str()))
        })
        .map(|e| e.text.as_str())
        .collect()
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
    fn correction_with_no_open_obligation_asks_only_for_the_final_answer() {
        let text = task_correction_text(&correction_task(&["satisfied", "cancelled"]));
        assert!(text.contains("final"), "{text}");
        assert!(
            !text.contains("yield") && !text.contains("blocked") && !text.contains("ob-0"),
            "{text}"
        );
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
    // MP-08 / MP-09 / MP-10 / MP-11 #971 R4: delegated plain completion receipts.
    #[tokio::test]
    async fn review971_verified_plain_answer_reaches_parent_without_assessment_json_truncation() {
        use super::super::browser_controller_action_execution_runtime_state::computer_input_reconcile_test_support::TestRoom;
        use crate::durable_state::agent_lifecycle::{Operation, Outcome};
        let room = TestRoom::new("amgoal-verified-delegate-receipt");
        let store = &room.runtime.owned.durable_state_store;
        let begin = |agent: &str, prompt: &str, now| Operation::Begin {
            owner: "owner".into(),
            room: room.session_id.clone(),
            agent: agent.into(),
            prompt: prompt.into(),
            run: Some("run".into()),
            now,
        };
        store
            .agent_lifecycle(begin("parent", "parent-prompt", 1))
            .unwrap();
        store
            .agent_lifecycle(Operation::RegisterObligation {
                owner: "owner".into(),
                room: room.session_id.clone(),
                agent: "parent".into(),
                prompt: "parent-prompt".into(),
                run: Some("run".into()),
                id: "delegate".into(),
                kind: "delegate".into(),
                resource: Some("child-prompt".into()),
                now: 1,
            })
            .unwrap();
        store
            .agent_lifecycle(begin(&room.agent_id, "child-prompt", 1))
            .unwrap();
        store
            .agent_lifecycle(Operation::EnableCompletionCheck {
                task: "child-prompt".into(),
            })
            .unwrap();
        let Outcome::Settled { task, .. } = store
            .agent_lifecycle(Operation::Settle {
                room: room.session_id.clone(),
                agent: room.agent_id.clone(),
                prompt: "child-prompt".into(),
                run: "run".into(),
                has_answer: true,
                cancelled: false,
                now: 2,
            })
            .unwrap()
        else {
            panic!()
        };
        let check = task.pending_prompt_id.unwrap();
        store
            .agent_lifecycle(begin(&room.agent_id, &check, 3))
            .unwrap();
        let evidence = (0..32)
            .map(|i| format!("public:verified-artifact-{i:02}-{}", "x".repeat(60)))
            .collect::<Vec<_>>();
        let answer = "Verified the requested cancellation behavior and all delegated evidence.";
        let raw =
            serde_json::json!({"status":"achieved","final_answer":answer,"evidence":evidence})
                .to_string();
        assert!(raw.len() > 1024 && raw.len() < 8192);
        let mut prompt = crate::history::SessionHistoryEntry::user_prompt(
            &room.session_id,
            "client",
            &room.agent_id,
            "check",
        );
        prompt.merge_key = Some(format!("prompt:{check}"));
        let output = crate::history::SessionHistoryEntry::provider_output(
            &room.session_id,
            "run",
            Some(&room.agent_id),
            crate::terminal::TerminalOutputKind::ProviderOutput,
            None,
            &raw,
        );
        for e in [prompt, output] {
            room.runtime
                .owned
                .operational_history_store
                .append_transcript(&e, Default::default())
                .unwrap();
        }
        let Outcome::Settled { task, .. } = store
            .agent_lifecycle(Operation::EvaluateGoal {
                task: "child-prompt".into(),
                prompt: check.clone(),
                result: raw,
                now: 4,
            })
            .unwrap()
        else {
            panic!()
        };
        assert!(task.goal.is_none());
        let source = crate::session::PromptQueueItem::new(
            check,
            "client",
            &room.agent_id,
            "assessment",
            crate::session::PromptStatus::Running,
        );
        room.runtime
            .finish_agent_task_settlement(Some((task, false)), &source)
            .await
            .unwrap();
        let events = store
            .agent_work_inbox(&room.session_id, "parent", 0, Some("parent-prompt"))
            .unwrap();
        let receipt = events
            .iter()
            .find(|e| e.payload.get("public_answer").is_some())
            .expect("delegated source outcome receipt");
        assert_eq!(receipt.payload["public_answer"]["excerpt"], answer);
        assert_eq!(
            receipt.payload["public_answer"]["evidence"],
            serde_json::json!(evidence)
        );
        assert_eq!(
            receipt.payload["public_answer"]["prompt_id"],
            "child-prompt"
        );
    }
}
