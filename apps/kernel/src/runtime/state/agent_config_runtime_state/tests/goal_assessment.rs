// MP-08 / MP-09 / MP-10 / MP-11 A12-S02: actual native context includes safe actionable receipts.
use super::*;

// MP-08 / MP-09 / MP-10 / MP-11: settled refusals still contradict invented success.
#[tokio::test]
async fn assessment_retains_bounded_rejected_dispatch_evidence() {
    use crate::durable_state::agent_lifecycle::Operation;
    use crate::history::SessionHistoryEntry;
    use crate::terminal::TerminalOutputKind;
    let (_app, runtime, room, agent) = agent_config_runtime().await;
    let store = &runtime.owned.durable_state_store;
    let apply = |op| store.agent_lifecycle(op).unwrap();
    let mut request = SessionHistoryEntry::user_prompt(
        &room,
        "owner",
        &agent,
        "Have cold-peer summarize the README",
    );
    request.merge_key = Some("prompt:refused-work".into());
    runtime
        .owned
        .append_operational_history_entry(&request, None, None, None);
    let answer = SessionHistoryEntry::provider_output(
        &room,
        "run",
        Some(&agent),
        TerminalOutputKind::ProviderOutput,
        None,
        "cold-peer summarized the README successfully",
    );
    runtime
        .owned
        .append_operational_history_entry(&answer, None, None, None);
    apply(Operation::Begin {
        owner: "owner".into(),
        room: room.clone(),
        agent: agent.clone(),
        prompt: "refused-work".into(),
        run: Some("run".into()),
        now: 1,
    });
    apply(Operation::StartGoal {
        task: "refused-work".into(),
        owner: "owner".into(),
        objective: "Have cold-peer summarize the README".into(),
        minutes: 1,
        now: 1,
    });
    for index in 0..66 {
        let id = format!("receipt-{index}");
        apply(Operation::RegisterObligation {
            owner: "owner".into(),
            room: room.clone(),
            agent: agent.clone(),
            prompt: "refused-work".into(),
            run: Some("run".into()),
            id: id.clone(),
            kind: "delegate".into(),
            resource: Some("cold-peer".into()),
            now: 1,
        });
        apply(Operation::DispatchReceipt {
            id,
            accepted: false,
            resource: None,
        });
    }
    apply(Operation::Settle {
        room: room.clone(),
        agent: agent.clone(),
        prompt: "refused-work".into(),
        run: "run".into(),
        has_answer: true,
        cancelled: false,
        now: 2,
    });
    let task = store
        .agent_tasks(Some(&room), Some(&agent))
        .unwrap()
        .remove(0);
    let context = runtime
        .owned
        .goal_continuation_context(&task)
        .unwrap()
        .unwrap();
    let data: serde_json::Value =
        serde_json::from_str(context.split_once("Untrusted assessment data: ").unwrap().1).unwrap();
    assert_eq!(data["coverage"]["total"], 0);
    assert!(data["obligations"].as_array().unwrap().is_empty());
    assert!(data["satisfied_obligations"].as_array().unwrap().is_empty());
    assert_eq!(
        data["latest_public_answer_messages"],
        serde_json::json!(["cold-peer summarized the README successfully"])
    );
    assert_eq!(
        data["rejected_dispatch_coverage"],
        serde_json::json!({"shown":64,"total":66})
    );
    let refusals = data["rejected_dispatches"].as_array().unwrap();
    assert_eq!(refusals.len(), 64);
    assert_eq!(
        refusals[0],
        serde_json::json!({
            "receipt_id":"receipt-0", "kind":"delegate", "target":"cold-peer",
            "reason_class":"dispatch_rejected", "effect":"not_admitted",
        })
    );
    assert_eq!(refusals[63]["receipt_id"], "receipt-63");
    assert!(context.contains("rejected_dispatches is kernel-confirmed evidence"));
    assert!(context.contains("An answer claiming that a rejected dispatch produced the requested effect cannot be achieved"));
    assert!(context.contains("A refusal report may be achieved only when the original objective is fulfilled by reporting that refusal"));

    // Admitted failure remains unresolved; successful delivery is not a refusal.
    let mut task = task;
    task.obligations[0].dispatch_state = "accepted".into();
    task.obligations[1].dispatch_state = "accepted".into();
    task.obligations[1].status = "satisfied".into();
    let context = runtime
        .owned
        .goal_continuation_context(&task)
        .unwrap()
        .unwrap();
    let data: serde_json::Value =
        serde_json::from_str(context.split_once("Untrusted assessment data: ").unwrap().1).unwrap();
    assert_eq!(data["coverage"]["total"], 1);
    assert_eq!(data["obligations"][0]["id"], "receipt-0");
    assert_eq!(data["satisfied_obligations"][0]["id"], "receipt-1");
    assert_eq!(
        data["rejected_dispatch_coverage"],
        serde_json::json!({"shown":64,"total":64})
    );
    assert_eq!(data["rejected_dispatches"][0]["receipt_id"], "receipt-2");
}

#[tokio::test]
async fn a12_assessment_context_exposes_unhandled_correlated_receipts() {
    use crate::durable_state::agent_lifecycle::{Operation, Registration};
    let (_app, runtime, room, agent) = agent_config_runtime().await;
    let store = &runtime.owned.durable_state_store;
    let apply = |op| store.agent_lifecycle(op).unwrap();
    apply(Operation::Begin {
        owner: "owner".into(),
        room: room.clone(),
        agent: agent.clone(),
        prompt: "goal-work".into(),
        run: Some("run".into()),
        now: 1,
    });
    apply(Operation::StartGoal {
        task: "goal-work".into(),
        owner: "owner".into(),
        objective: "Verify delegate artifacts".into(),
        minutes: 1,
        now: 1,
    });
    apply(Operation::RegisterObligation {
        owner: "owner".into(),
        room: room.clone(),
        agent: agent.clone(),
        prompt: "goal-work".into(),
        run: Some("run".into()),
        id: "delegate".into(),
        kind: "delegate".into(),
        resource: Some("child".into()),
        now: 1,
    });
    apply(Operation::Subscribe {
        task: "goal-work".into(),
        prompt: "goal-work".into(),
        registration: Registration {
            id: "completion".into(),
            task_id: "goal-work".into(),
            source_id: "child".into(),
            obligation_id: Some("delegate".into()),
            source_cursor: 0,
            live: true,
        },
    });
    apply(Operation::SourceOutcome {
        room: room.clone(),
        source: "child".into(),
        occurrence: "finished".into(),
        success: true,
        public_answer: Some(serde_json::json!({"private_body":"DO_NOT_INCLUDE_RAW_RECEIPT_BODY"})),
        now: 2,
    });
    let events = store
        .agent_work_inbox(&room, &agent, 0, Some("goal-work"))
        .unwrap();
    assert_eq!(events.len(), 1);
    apply(Operation::Attempt {
        room: room.clone(),
        agent: agent.clone(),
        sequence: events[0].sequence,
        prompt: "event-turn".into(),
        target: Some("goal-work".into()),
        work: Some("goal-work".into()),
        run: Some("run".into()),
        now: 2,
    });
    apply(Operation::Receipt {
        room: room.clone(),
        agent: agent.clone(),
        sequence: events[0].sequence,
        state: "accepted".into(),
        now: 2,
    });
    apply(Operation::Settle {
        room: room.clone(),
        agent: agent.clone(),
        prompt: "goal-work".into(),
        run: "run".into(),
        has_answer: true,
        cancelled: false,
        now: 3,
    });
    let task = store
        .agent_tasks(Some(&room), Some(&agent))
        .unwrap()
        .into_iter()
        .find(|t| t.goal.is_some())
        .unwrap();
    let context = runtime
        .owned
        .goal_continuation_context(&task)
        .unwrap()
        .unwrap();
    assert!(
        context.contains("\"unhandled_receipts\""),
        "native assessment needs pending public receipt refs after sources become non-live"
    );
    assert!(context.contains("\"state\":\"accepted\""));
    assert!(context.contains("\"sequence\":"));
    assert!(
        !context.contains("DO_NOT_INCLUDE_RAW_RECEIPT_BODY"),
        "only safe receipt metadata/references may enter assessment context"
    );
    assert!(
        context.contains("handled:true"),
        "continue normal handling/verification before owner-blocking on terminal sources"
    );
    apply(Operation::Ack {
        room: room.clone(),
        agent: agent.clone(),
        sequence: events[0].sequence,
        handled: true,
        now: 4,
    });
    let task = store
        .agent_tasks(Some(&room), Some(&agent))
        .unwrap()
        .into_iter()
        .find(|t| t.goal.is_some())
        .unwrap();
    let context = runtime
        .owned
        .goal_continuation_context(&task)
        .unwrap()
        .unwrap();
    let data: serde_json::Value =
        serde_json::from_str(context.split_once("Untrusted assessment data: ").unwrap().1).unwrap();
    assert_eq!(
        data["handled_source_receipts"][0]["sequence"],
        events[0].sequence
    );
    assert_eq!(
        data["handled_source_receipts"][0]["kind"],
        "source_completed"
    );
    assert_eq!(data["handled_source_receipts"][0]["state"], "handled");
    assert_eq!(data["satisfied_obligations"][0]["id"], "delegate");
    assert_eq!(data["satisfied_obligations"][0]["status"], "satisfied");
    assert!(data["unhandled_receipts"].as_array().unwrap().is_empty());
    assert!(!context.contains("DO_NOT_INCLUDE_RAW_RECEIPT_BODY"));
}

#[tokio::test]
async fn a12_resume_queues_behind_independent_work_but_fences_unsettled_goal_turns() {
    use crate::durable_state::agent_lifecycle::Operation;
    use crate::runtime::state::agent_goal_runtime::goal_resume_has_unsettled_turn;
    let (_app, runtime, room, agent) = agent_config_runtime().await;
    let store = &runtime.owned.durable_state_store;
    store
        .agent_lifecycle(Operation::Begin {
            owner: "owner".into(),
            room: room.clone(),
            agent: agent.clone(),
            prompt: "goal-original".into(),
            run: Some("run".into()),
            now: 1,
        })
        .unwrap();
    store
        .agent_lifecycle(Operation::StartGoal {
            task: "goal-original".into(),
            owner: "owner".into(),
            objective: "Verify public artifacts".into(),
            minutes: 1,
            now: 1,
        })
        .unwrap();
    let mut task = store
        .agent_tasks(Some(&room), Some(&agent))
        .unwrap()
        .remove(0);
    assert!(
        !goal_resume_has_unsettled_turn(&task, Some("independent-peer-message")),
        "Resume must enter the shared queue behind independent work"
    );
    assert!(!goal_resume_has_unsettled_turn(&task, None));
    assert!(goal_resume_has_unsettled_turn(&task, Some("goal-original")));
    task.prompt_id = "goal-later-work".into();
    task.pending_prompt_id = Some("goal-check".into());
    task.goal.as_mut().unwrap().evaluation = Some(
        crate::durable_state::agent_lifecycle::goals::GoalEvaluation {
            origin: "completion_check".into(),
            prompt_id: "goal-check".into(),
            work_prompt_id: "goal-later-work".into(),
            work_provider_run_id: Some("run".into()),
            objective_revision: 1,
            ledger_revision: task.revision,
            started_at_ms: 2,
            deadline_ms: 60002,
            paused_at_ms: None,
            lease: None,
            retried: false,
            result: None,
        },
    );
    for id in ["goal-original", "goal-later-work", "goal-check"] {
        assert!(
            goal_resume_has_unsettled_turn(&task, Some(id)),
            "stopped goal-owned work/check must settle first"
        );
    }
    assert!(!goal_resume_has_unsettled_turn(
        &task,
        Some("independent-peer-message")
    ));
}

#[tokio::test]
async fn a12_assessment_exposes_expired_wake_with_unresolved_source_outcome() {
    use crate::durable_state::agent_lifecycle::{Operation, Registration};
    let (_app, runtime, room, agent) = agent_config_runtime().await;
    let store = &runtime.owned.durable_state_store;
    let apply = |op| store.agent_lifecycle(op).unwrap();
    apply(Operation::Begin {
        owner: "owner".into(),
        room: room.clone(),
        agent: agent.clone(),
        prompt: "p".into(),
        run: Some("run".into()),
        now: 1,
    });
    apply(Operation::StartGoal {
        task: "p".into(),
        owner: "owner".into(),
        objective: "Review public peer evidence".into(),
        minutes: 1,
        now: 1,
    });
    apply(Operation::RegisterObligation {
        owner: "owner".into(),
        room: room.clone(),
        agent: agent.clone(),
        prompt: "p".into(),
        run: Some("run".into()),
        id: "peer".into(),
        kind: "delegate".into(),
        resource: Some("child".into()),
        now: 1,
    });
    apply(Operation::Subscribe {
        task: "p".into(),
        prompt: "p".into(),
        registration: Registration {
            id: "completion".into(),
            task_id: "p".into(),
            source_id: "child".into(),
            obligation_id: Some("peer".into()),
            source_cursor: 0,
            live: true,
        },
    });
    apply(Operation::SourceOutcome {
        room: room.clone(),
        source: "child".into(),
        occurrence: "result".into(),
        success: true,
        public_answer: Some(serde_json::json!({"private_body":"DO_NOT_INCLUDE_EXPIRED_BODY"})),
        now: 2,
    });
    let event = store
        .agent_work_inbox(&room, &agent, 0, Some("p"))
        .unwrap()
        .remove(0);
    apply(Operation::Expire {
        room: room.clone(),
        agent: agent.clone(),
        sequence: event.sequence,
    });
    let task = store
        .agent_tasks(Some(&room), Some(&agent))
        .unwrap()
        .remove(0);
    let context = runtime
        .owned
        .goal_continuation_context(&task)
        .unwrap()
        .unwrap();
    assert!(
        context.contains("\"state\":\"expired\""),
        "ordinary continuation must be able to review and handle the retained exact source outcome"
    );
    assert!(!context.contains("DO_NOT_INCLUDE_EXPIRED_BODY"));
}

// MP-08 / MP-10 / MP-11 A10/A12: the home evaluates a leased task's
// public result; private MCP records are deliberately not part of that proof.
#[tokio::test]
async fn a10_completion_assesses_leased_public_result_without_requesting_private_trace() {
    use crate::durable_state::agent_lifecycle::Operation;
    use crate::history::SessionHistoryEntry;
    use crate::terminal::TerminalOutputKind;
    let (_app, runtime, room, agent) = agent_config_runtime().await;
    let store = &runtime.owned.durable_state_store;
    let prompt = "leased-health";
    let run = "leased:lease:worker-run";
    let mut request = SessionHistoryEntry::user_prompt(
        &room,
        "owner",
        &agent,
        "Call GetDaemonHealth once and report the returned health status; no other tools or edits.",
    );
    request.merge_key = Some(format!("prompt:{prompt}"));
    let answer = SessionHistoryEntry::provider_output(
        &room,
        "worker-run",
        Some(&agent),
        TerminalOutputKind::ProviderOutput,
        None,
        "GetDaemonHealth returned a healthy kernel with no projection mismatches.",
    );
    let private_tool = SessionHistoryEntry::provider_output(
        &room,
        "worker-run",
        Some(&agent),
        TerminalOutputKind::ProviderTool,
        None,
        r#"{"tool":"chariox_kernel_request","input":"PRIVATE_MCP_ARGUMENTS","output":"PRIVATE_MCP_OUTPUT"}"#,
    );
    for entry in [request, private_tool, answer] {
        runtime
            .owned
            .append_operational_history_entry(&entry, None, None, None);
    }
    for op in [
        Operation::Begin {
            owner: "owner".into(),
            room: room.clone(),
            agent: agent.clone(),
            prompt: prompt.into(),
            run: Some(run.into()),
            now: 1,
        },
        Operation::StartGoal {
            task: prompt.into(),
            owner: "owner".into(),
            objective: "Check the kernel health".into(),
            minutes: 10,
            now: 1,
        },
        Operation::Settle {
            room: room.clone(),
            agent: agent.clone(),
            prompt: prompt.into(),
            run: run.into(),
            has_answer: true,
            cancelled: false,
            now: 2,
        },
    ] {
        store.agent_lifecycle(op).unwrap();
    }
    let task = store
        .agent_tasks(Some(&room), Some(&agent))
        .unwrap()
        .remove(0);
    let context = runtime
        .owned
        .goal_continuation_context(&task)
        .unwrap()
        .unwrap();
    assert!(!context.contains("PRIVATE_MCP_ARGUMENTS"));
    assert!(!context.contains("PRIVATE_MCP_OUTPUT"));
    let data: serde_json::Value =
        serde_json::from_str(context.split_once("Untrusted assessment data: ").unwrap().1).unwrap();
    assert_eq!(
        data["latest_public_answer_messages"],
        serde_json::json!([
            "GetDaemonHealth returned a healthy kernel with no projection mismatches."
        ])
    );
    assert_eq!(
        data["answer_coverage"]["shown_chars"],
        data["answer_coverage"]["total_chars"]
    );
    assert_eq!(data["coverage"]["total"], 0);
    assert_eq!(
        data["tool_trace_visibility"],
        "private tool records are intentionally omitted, not missing evidence coverage"
    );
    assert!(context.contains("Do not request private tool traces from the owner solely because the public projection omits them"));
    assert!(context.contains("Do not invent verification"));
    // MP-08 / MP-10 / MP-11: a real Haiku assessment answered `waiting` with a
    // relative `deadline_ms` (300000) and the kernel blocked the parent. The
    // guidance must say the deadline is an absolute epoch time.
    assert!(
        context.contains("deadline_ms is an absolute Unix epoch time in milliseconds"),
        "{context}"
    );
}

// A full one-character answer is the A12-S04 result, not a cut-off excerpt.
async fn count_answer_assessment_context(answer: &str) -> String {
    count_messages_assessment_context(&[(None, answer)]).await
}

async fn count_messages_assessment_context(answers: &[(Option<&str>, &str)]) -> String {
    use crate::durable_state::agent_lifecycle::Operation;
    use crate::history::SessionHistoryEntry;
    use crate::terminal::TerminalOutputKind;
    let (_app, runtime, room, agent) = agent_config_runtime().await;
    let store = &runtime.owned.durable_state_store;
    let prompt = "count-answer";
    let run = "leased:lease:worker-run";
    let mut request = SessionHistoryEntry::user_prompt(
        &room,
        "owner",
        &agent,
        "Call ListSessions once and report only the count; no other tools or edits.",
    );
    request.merge_key = Some(format!("prompt:{prompt}"));
    let answers = answers.iter().map(|(key, text)| {
        SessionHistoryEntry::provider_output(
            &room,
            "worker-run",
            Some(&agent),
            TerminalOutputKind::ProviderOutput,
            key.map(str::to_owned),
            *text,
        )
    });
    let private_tool = SessionHistoryEntry::provider_output(
        &room,
        "worker-run",
        Some(&agent),
        TerminalOutputKind::ProviderTool,
        None,
        r#"{"id":"count-call","tool":"chariox_kernel_request","status":"completed","input":{"request":{"ListSessions":null}},"output":"PRIVATE_MCP_OUTPUT"}"#,
    );
    for entry in [request, private_tool].into_iter().chain(answers) {
        runtime
            .owned
            .append_operational_history_entry(&entry, None, None, None);
    }
    for op in [
        Operation::Begin {
            owner: "owner".into(),
            room: room.clone(),
            agent: agent.clone(),
            prompt: prompt.into(),
            run: Some(run.into()),
            now: 1,
        },
        Operation::StartGoal {
            task: prompt.into(),
            owner: "owner".into(),
            objective: "Call ListSessions once and report only the count".into(),
            minutes: 10,
            now: 1,
        },
        Operation::Settle {
            room: room.clone(),
            agent: agent.clone(),
            prompt: prompt.into(),
            run: run.into(),
            has_answer: true,
            cancelled: false,
            now: 2,
        },
    ] {
        store.agent_lifecycle(op).unwrap();
    }
    let task = store
        .agent_tasks(Some(&room), Some(&agent))
        .unwrap()
        .remove(0);
    runtime
        .owned
        .goal_continuation_context(&task)
        .unwrap()
        .unwrap()
}

// MP-08 / MP-09 / MP-10 / MP-11: r11 check-1 received 91 characters ending
// in `sudo window.2`; the last message must stay distinct from the retry plan.
#[tokio::test]
async fn assessment_preserves_captured_assistant_message_boundaries() {
    let captured: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("goal_assessment_count_messages.json")).unwrap();
    let answers = captured
        .iter()
        .map(|item| (item["merge_key"].as_str(), item["text"].as_str().unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(
        answers
            .iter()
            .map(|(_, text)| *text)
            .collect::<String>()
            .chars()
            .count(),
        91
    );
    let context = count_messages_assessment_context(&answers).await;
    let data: serde_json::Value =
        serde_json::from_str(context.split_once("Untrusted assessment data: ").unwrap().1).unwrap();
    assert_eq!(data["latest_public_answer_messages"], serde_json::json!([
        "I’m retrying the required typed request once under the resumed task’s current sudo window.",
        "2"
    ]));
    assert_eq!(data["answer_coverage"]["shown_chars"], 91);
    assert_eq!(data["answer_coverage"]["total_chars"], 91);
    assert_eq!(data["answer_coverage"]["complete"], true);
    assert_eq!(data["completed_kernel_calls"][0]["request"], "ListSessions");
    assert!(data.get("latest_public_answer_tail").is_none());
}

#[tokio::test]
async fn assessment_keeps_provider_merged_text_in_one_message() {
    let captured: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("goal_assessment_count_messages.json")).unwrap();
    // These adjacent captured deltas all belong to the provider's first message.
    let answers = captured
        .iter()
        .take(captured.len() - 1)
        .map(|item| (item["merge_key"].as_str(), item["text"].as_str().unwrap()))
        .collect::<Vec<_>>();
    let context = count_messages_assessment_context(&answers).await;
    let data: serde_json::Value =
        serde_json::from_str(context.split_once("Untrusted assessment data: ").unwrap().1).unwrap();
    assert_eq!(
        data["latest_public_answer_messages"],
        serde_json::json!([
        "I’m retrying the required typed request once under the resumed task’s current sudo window."
    ])
    );
    // Text itself never creates a boundary, including a short trailing number.
    let context = count_messages_assessment_context(&[
        (Some("same-message"), "Retrying."),
        (Some("same-message"), "2"),
    ])
    .await;
    let data: serde_json::Value =
        serde_json::from_str(context.split_once("Untrusted assessment data: ").unwrap().1).unwrap();
    assert_eq!(
        data["latest_public_answer_messages"],
        serde_json::json!(["Retrying.2"])
    );
}

#[tokio::test]
async fn assessment_marks_the_full_one_character_count_as_complete() {
    let context = count_answer_assessment_context("2").await;
    let data: serde_json::Value =
        serde_json::from_str(context.split_once("Untrusted assessment data: ").unwrap().1).unwrap();
    assert_eq!(
        data["latest_public_answer_messages"],
        serde_json::json!(["2"])
    );
    assert_eq!(data["answer_coverage"]["shown_chars"], 1);
    assert_eq!(data["answer_coverage"]["total_chars"], 1);
    assert_eq!(data["answer_coverage"]["complete"], true);
    assert_eq!(data["answer_coverage"]["truncated"], false);
    assert_eq!(data["completed_kernel_calls"][0]["request"], "ListSessions");
    assert_eq!(data["completed_kernel_calls_omitted"], 0);
    assert!(!context.contains("PRIVATE_MCP_OUTPUT"));
    assert!(context.contains("even one character"));
    assert!(context.contains("never treat it as truncated"));
    assert!(context.contains("that complete result is the answer, not a plan or a missing report"));
    assert!(context.contains(
        "Do not require a narrative of prerequisite authority, blocking or resume steps"
    ));
}

#[tokio::test]
async fn assessment_empty_public_output_is_uncut_but_not_evidence_of_success() {
    let context = count_answer_assessment_context("").await;
    let data: serde_json::Value =
        serde_json::from_str(context.split_once("Untrusted assessment data: ").unwrap().1).unwrap();
    assert_eq!(data["latest_public_answer_messages"], serde_json::json!([]));
    assert_eq!(data["answer_coverage"]["shown_chars"], 0);
    assert_eq!(data["answer_coverage"]["total_chars"], 0);
    assert_eq!(data["answer_coverage"]["complete"], true);
    assert_eq!(data["answer_coverage"]["truncated"], false);
    assert!(context.contains("an empty result is not evidence of success"));
}

#[tokio::test]
async fn assessment_keeps_a_genuinely_truncated_answer_incomplete() {
    let context = count_answer_assessment_context(&"2".repeat(4097)).await;
    let data: serde_json::Value =
        serde_json::from_str(context.split_once("Untrusted assessment data: ").unwrap().1).unwrap();
    assert_eq!(data["answer_coverage"]["shown_chars"], 4096);
    assert_eq!(data["answer_coverage"]["total_chars"], 4097);
    assert_eq!(data["answer_coverage"]["complete"], false);
    assert_eq!(data["answer_coverage"]["truncated"], true);
    assert!(context.contains("Truncated objective/evidence coverage or missing original public context cannot mean achieved"));
}

#[tokio::test]
async fn assessment_bounds_message_text_without_inventing_unknown_boundaries() {
    let long = "界".repeat(4096);
    let context =
        count_messages_assessment_context(&[(Some("commentary"), &long), (Some("answer"), "2")])
            .await;
    let data: serde_json::Value =
        serde_json::from_str(context.split_once("Untrusted assessment data: ").unwrap().1).unwrap();
    assert_eq!(
        data["latest_public_answer_messages"],
        serde_json::json!(["界".repeat(4095), "2"])
    );
    assert_eq!(data["answer_coverage"]["shown_chars"], 4096);
    assert_eq!(data["answer_coverage"]["total_chars"], 4097);
    assert_eq!(data["answer_coverage"]["truncated"], true);
    let context = count_messages_assessment_context(&[(None, "Retrying."), (None, "2")]).await;
    let data: serde_json::Value =
        serde_json::from_str(context.split_once("Untrusted assessment data: ").unwrap().1).unwrap();
    assert_eq!(
        data["latest_public_answer_messages"],
        serde_json::json!(["Retrying.2"])
    );
}

// MP-08/MP-10/MP-11: a privacy projection is never evidence for a native check.
#[tokio::test]
async fn mp11_protected_assessment_context_omits_public_withheld_answer() {
    use crate::durable_state::agent_lifecycle::Operation;
    use crate::history::SessionHistoryEntry;
    use crate::terminal::TerminalOutputKind;
    let (_app, runtime, room, agent) = agent_config_runtime().await;
    let store = &runtime.owned.durable_state_store;
    let mut request = SessionHistoryEntry::user_prompt(
        &room,
        "owner",
        &agent,
        "Complete the approved native insertion without revealing its value.",
    );
    request.merge_key = Some("prompt:protected-work".into());
    runtime
        .owned
        .append_operational_history_entry(&request, None, None, None);
    runtime
        .owned
        .room_secret_observations
        .register(&room, "PRIVATE_CONTEXT_CANARY")
        .unwrap();
    let answer = SessionHistoryEntry::provider_output(
        &room,
        "run",
        Some(&agent),
        TerminalOutputKind::ProviderOutput,
        None,
        "PRIVATE_CONTEXT_CANARY PRIVATE_PROVIDER_PROSE",
    );
    runtime
        .owned
        .append_operational_history_entry(&answer, None, None, None);
    for op in [
        Operation::Begin {
            owner: "owner".into(),
            room: room.clone(),
            agent: agent.clone(),
            prompt: "protected-work".into(),
            run: Some("run".into()),
            now: 1,
        },
        Operation::StartGoal {
            task: "protected-work".into(),
            owner: "owner".into(),
            objective: "Check the kernel health".into(),
            minutes: 10,
            now: 1,
        },
        Operation::Settle {
            room: room.clone(),
            agent: agent.clone(),
            prompt: "protected-work".into(),
            run: "run".into(),
            has_answer: true,
            cancelled: false,
            now: 2,
        },
    ] {
        store.agent_lifecycle(op).unwrap();
    }
    let task = store
        .agent_tasks(Some(&room), Some(&agent))
        .unwrap()
        .remove(0);
    let context = runtime
        .owned
        .goal_continuation_context(&task)
        .unwrap()
        .unwrap();
    assert!(
        !context.contains("[sensitive Room terminal stream withheld]"),
        "MP-11 public privacy marker must never be assessment input"
    );
    assert!(
        !context.contains("PRIVATE_CONTEXT_CANARY") && !context.contains("PRIVATE_PROVIDER_PROSE"),
        "MP-11 private result bytes must never be copied into an agent prompt"
    );
    assert!(!context.contains("using only public evidence below"));
    assert!(context.contains("exact kernel-private preceding result supplied separately"));
    assert!(context.contains("Privacy withholding alone is never evidence of success"));
    let data: serde_json::Value =
        serde_json::from_str(context.split_once("Untrusted assessment data: ").unwrap().1).unwrap();
    assert!(data["latest_public_answer_messages"].is_null());
    assert_eq!(data["answer_coverage"]["visibility"], "protected");
    assert!(data["completed_kernel_calls"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(data["answer_coverage"]["complete"], false);
    assert!(data["answer_coverage"]["truncated"].is_null());
    let mut leased = task.clone();
    leased
        .goal
        .as_mut()
        .unwrap()
        .evaluation
        .as_mut()
        .unwrap()
        .work_provider_run_id = Some("leased:worker:run".into());
    let context = runtime
        .owned
        .goal_continuation_context(&leased)
        .unwrap()
        .unwrap();
    assert!(!context.contains("already available in this same official-provider thread"));
    assert!(context.contains(
        "Protected leased outcomes absent from the home provider context cannot establish success"
    ));
    assert!(!context.contains("[sensitive Room terminal stream withheld]"));
}

// MP-08/MP-10/MP-11: private completion control must survive the public byte fence.
#[tokio::test]
async fn mp11_protected_completion_reads_private_result_and_keeps_history_withheld() {
    use crate::durable_state::agent_lifecycle::{ExecutionState, Operation};
    use crate::history::SessionHistoryEntry;
    use crate::terminal::TerminalOutputKind;
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let (_app, runtime, room, agent) = agent_config_runtime_with_config(config).await;
    assert!(
        runtime.owned.config_projection.snapshot().room_agent_tools,
        "MP-11 fixture must enable task settlement"
    );
    let store = &runtime.owned.durable_state_store;
    let now = crate::session::unix_epoch_ms();
    for op in [
        Operation::Begin {
            owner: "local-user".into(),
            room: room.clone(),
            agent: agent.clone(),
            prompt: "private-work".into(),
            run: Some("run".into()),
            now,
        },
        Operation::StartGoal {
            task: "private-work".into(),
            owner: "local-user".into(),
            objective: "Check the kernel health".into(),
            minutes: 10,
            now: 1,
        },
        Operation::Settle {
            room: room.clone(),
            agent: agent.clone(),
            prompt: "private-work".into(),
            run: "run".into(),
            has_answer: true,
            cancelled: false,
            now,
        },
    ] {
        store.agent_lifecycle(op).unwrap();
    }
    let task = store
        .agent_tasks(Some(&room), Some(&agent))
        .unwrap()
        .remove(0);
    let id = task.pending_prompt_id.unwrap();
    store
        .agent_lifecycle(Operation::BeginContinuation {
            task: task.task_id,
            room: room.clone(),
            agent: agent.clone(),
            prompt: id.clone(),
            run: Some("run".into()),
            now,
        })
        .unwrap();
    let prompt = crate::session::PromptQueueItem::new(
        &id,
        "attachment-1",
        &agent,
        "check",
        crate::session::PromptStatus::Running,
    )
    .with_prompt_origin(crate::session::PromptOrigin::CompletionCheck);
    runtime
        .owned
        .prompt_state_owner
        .sync_external_active_prompt(
            &runtime.owned.session_store.get_session(&room).unwrap(),
            &agent,
            Some(prompt.clone()),
        );
    runtime
        .owned
        .room_secret_observations
        .register(&room, "synthetic-private-fill")
        .unwrap();
    let mut request = SessionHistoryEntry::user_prompt(&room, "local-user", &agent, "check");
    request.merge_key = Some(format!("prompt:{id}"));
    runtime.owned.append_history_entries(&room, vec![request]);
    let raw = r#"{"status":"achieved","final_answer":"Filled synthetic-private-fill; PRIVATE_COMPLETION_PROSE","evidence":["PRIVATE_COMPLETION_EVIDENCE"]}"#;
    runtime.owned.fan_out_terminal_outputs(
        &room,
        vec![
            crate::runtime::state::prompt_transcript_owned_state::TerminalOutputBatchAppend {
                sequence: None,
                provider_run_id: "run".into(),
                agent_id: Some(agent.clone()),
                kind: TerminalOutputKind::ProviderOutput,
                merge_key: Some("assessment".into()),
                bytes: raw.as_bytes().to_vec(),
            },
        ],
    );
    runtime.owned.append_history_entries(
        &room,
        vec![SessionHistoryEntry::provider_output(
            &room,
            "run",
            Some(&agent),
            TerminalOutputKind::ProviderOutput,
            Some("assessment".into()),
            raw,
        )],
    );
    let entries = runtime
        .owned
        .operational_history_store
        .load_session_history_entries(&room, Some(&agent))
        .unwrap();
    assert!(entries
        .iter()
        .any(|e| e.text == "[sensitive Room terminal stream withheld]"));
    assert!(!entries
        .iter()
        .any(|e| e.text.contains("synthetic-private-fill")));
    let (settled, _) = runtime
        .owned
        .settle_agent_task(&room, &agent, &prompt, "run", false)
        .unwrap()
        .unwrap();
    assert_eq!(
        settled.state,
        ExecutionState::Done,
        "MP-11 withheld public marker must never be parsed as the private assessment"
    );
    let receipt = serde_json::to_string(&settled).unwrap();
    assert!(
        !receipt.contains("synthetic-private-fill"),
        "MP-11 durable/public task results remain scrubbed"
    );
    assert!(!receipt.contains("terminal stream withheld"));
    for private_detail in ["PRIVATE_COMPLETION_PROSE", "PRIVATE_COMPLETION_EVIDENCE"] {
        assert!(
            !receipt.contains(private_detail),
            "MP-11 private assessment prose must not enter durable/public receipts"
        );
    }
}

// MP-08/MP-10/MP-11: the git observer reports an artifact change at every turn end. A change it
// sees during a completion check (an unrelated edit in the workspace) is not progress of the
// check; offering it to the ledger failed the output pump and lost the verdict (60 s bound).
#[tokio::test]
async fn artifact_progress_never_targets_a_completion_check_turn() {
    use crate::durable_state::agent_lifecycle::Operation;
    use crate::runtime::state::agent_task_projection::artifact_progress_task;
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let (_app, runtime, room, agent) = agent_config_runtime_with_config(config).await;
    let store = &runtime.owned.durable_state_store;
    let begin = |prompt: &str| Operation::Begin {
        owner: "local-user".into(),
        room: room.clone(),
        agent: agent.clone(),
        prompt: prompt.into(),
        run: Some("run".into()),
        now: crate::session::unix_epoch_ms(),
    };
    store.agent_lifecycle(begin("work-turn")).unwrap();
    store
        .agent_lifecycle(Operation::StartGoal {
            task: "work-turn".into(),
            owner: "local-user".into(),
            objective: "Answer the request".into(),
            minutes: 1,
            now: 1,
        })
        .unwrap();
    let tasks = || store.agent_tasks(Some(&room), Some(&agent)).unwrap();
    assert!(
        artifact_progress_task(tasks(), "work-turn", "run").is_some(),
        "a live work turn still takes artifact progress"
    );
    store
        .agent_lifecycle(Operation::Settle {
            room: room.clone(),
            agent: agent.clone(),
            prompt: "work-turn".into(),
            run: "run".into(),
            has_answer: true,
            cancelled: false,
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    let check_prompt = tasks()
        .into_iter()
        .find_map(|task| task.pending_prompt_id)
        .expect("the answered turn is waiting for its completion check");
    store.agent_lifecycle(begin(&check_prompt)).unwrap();
    let checking = tasks()
        .into_iter()
        .find(|task| task.prompt_id == check_prompt)
        .expect("the check turn runs on the task");
    assert!(crate::durable_state::agent_lifecycle::goals::is_check(
        &checking,
        &check_prompt
    ));
    assert!(
        artifact_progress_task(tasks(), &check_prompt, "run").is_none(),
        "the completion check turn takes no artifact progress"
    );
}

// MP-08/MP-10/MP-11: real RED47 neutral utility had no work-thread context.
#[tokio::test]
async fn mp11_neutral_assessment_uses_private_work_result_without_public_or_durable_injection() {
    use crate::durable_state::agent_lifecycle::Operation;
    use crate::history::SessionHistoryEntry;
    use crate::terminal::TerminalOutputKind;
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let (_app, runtime, room, agent) = agent_config_runtime_with_config(config).await;
    let store = &runtime.owned.durable_state_store;
    let now = crate::session::unix_epoch_ms();
    store
        .agent_lifecycle(Operation::Begin {
            owner: "local-user".into(),
            room: room.clone(),
            agent: agent.clone(),
            prompt: "private-work-input".into(),
            run: Some("run".into()),
            now,
        })
        .unwrap();
    store
        .agent_lifecycle(Operation::StartGoal {
            task: "private-work-input".into(),
            owner: "local-user".into(),
            objective: "Check the kernel health".into(),
            minutes: 10,
            now: 1,
        })
        .unwrap();
    let work = crate::session::PromptQueueItem::new(
        "private-work-input",
        "attachment-1",
        &agent,
        "Fill the approved control without revealing its value.",
        crate::session::PromptStatus::Running,
    );
    runtime
        .owned
        .prompt_state_owner
        .sync_external_active_prompt(
            &runtime.owned.session_store.get_session(&room).unwrap(),
            &agent,
            Some(work.clone()),
        );
    let mut request = SessionHistoryEntry::user_prompt(&room, "local-user", &agent, work.prompt());
    request.merge_key = Some("prompt:private-work-input".into());
    runtime.owned.append_history_entries(&room, vec![request]);
    runtime
        .owned
        .private_work_results
        .begin_work(&room, &agent, work.id(), "run");
    runtime
        .owned
        .room_secret_observations
        .register(&room, "PRIVATE_VALUE_CANARY")
        .unwrap();
    runtime.owned.fan_out_terminal_outputs(
        &room,
        vec![
            crate::runtime::state::prompt_transcript_owned_state::TerminalOutputBatchAppend {
                sequence: None,
                provider_run_id: "run".into(),
                agent_id: Some(agent.clone()),
                kind: TerminalOutputKind::ProviderOutput,
                merge_key: Some("final".into()),
                bytes: b"PRIVATE_WORK_OUTCOME: inserted PRIVATE_VALUE_CANARY; form not submitted"
                    .to_vec(),
            },
        ],
    );
    runtime.owned.append_history_entries(
        &room,
        vec![SessionHistoryEntry::provider_output(
            &room,
            "run",
            Some(&agent),
            TerminalOutputKind::ProviderOutput,
            Some("final".into()),
            "PRIVATE_WORK_OUTCOME: inserted PRIVATE_VALUE_CANARY; form not submitted",
        )],
    );
    let (task, _) = runtime
        .owned
        .settle_agent_task(&room, &agent, &work, "run", false)
        .unwrap()
        .unwrap();
    let check = task.pending_prompt_id.clone().unwrap();
    let public = runtime
        .owned
        .goal_continuation_context(&task)
        .unwrap()
        .unwrap();
    assert!(
        !public.contains("PRIVATE_WORK_OUTCOME")
            && !public.contains("PRIVATE_VALUE_CANARY")
            && !public.contains("[sensitive Room terminal stream withheld]")
    );
    store
        .agent_lifecycle(Operation::BeginContinuation {
            task: task.task_id.clone(),
            room: room.clone(),
            agent: agent.clone(),
            prompt: check.clone(),
            run: Some("run".into()),
            now,
        })
        .unwrap();
    for (r, a, p, run) in [
        (room.as_str(), agent.as_str(), check.as_str(), "foreign"),
        (room.as_str(), agent.as_str(), "wrong-check", "run"),
    ] {
        assert!(runtime
            .owned
            .take_private_assessment_evidence(r, a, p, run)
            .is_none());
    }
    let evidence = {
        let app = _app.lock().await;
        app.private_assessment_dispatch
            .as_ref()
            .unwrap()
            .for_app_dispatch(
                &app.sessions().get_session(&room).unwrap(),
                &agent,
                &check,
                "run",
                crate::session::PromptOrigin::CompletionCheck,
            )
            .unwrap()
            .expect("MP-11 App completion must consume the exact shared private result")
    };
    assert!(evidence.as_str().contains("PRIVATE_WORK_OUTCOME"));
    assert!(!evidence.as_str().contains("PRIVATE_VALUE_CANARY"));
    assert!(runtime
        .owned
        .take_private_assessment_evidence(&room, &agent, &check, "run")
        .is_none());
    let mut envelope = crate::prompt_assembly::PromptEnvelope::new(
        &public,
        "",
        vec![],
        crate::prompt_assembly::PromptManifest::current(),
    );
    envelope.assessment_only = true;
    envelope.private_assessment_evidence = Some(evidence);
    assert!(envelope
        .private_assessment_prompt()
        .unwrap()
        .as_str()
        .contains("PRIVATE_WORK_OUTCOME"));
    for value in [
        serde_json::to_string(&envelope).unwrap(),
        format!("{envelope:?}"),
        serde_json::to_string(&runtime.owned.session_store.get_session(&room).unwrap()).unwrap(),
    ] {
        assert!(!value.contains("PRIVATE_WORK_OUTCOME") && !value.contains("PRIVATE_VALUE_CANARY"));
    }
    let history = runtime
        .owned
        .operational_history_store
        .load_session_history_entries(&room, Some(&agent))
        .unwrap();
    assert!(!serde_json::to_string(&history)
        .unwrap()
        .contains("PRIVATE_WORK_OUTCOME"));
}

// MP-08/MP-10: an achieved check publishes only its final answer as ordinary
// agent text; other receipts and protected rooms publish nothing.
#[tokio::test]
async fn achieved_check_publishes_only_the_final_answer() {
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let (_app, runtime, room, agent) = agent_config_runtime_with_config(config).await;
    let owned = &runtime.owned;
    let history = || {
        owned
            .operational_history_store
            .load_session_history_entries(&room, Some(&agent))
            .unwrap()
    };
    for receipt in [
        r#"{"status":"continue","reason":"more work"}"#,
        r#"{"status":"blocked","reason":"needs owner"}"#,
        r#"{"status":"achieved","final_answer":"  ","evidence":["e"]}"#,
        "",
    ] {
        owned.publish_check_answer(&room, &agent, "run", receipt, "");
    }
    assert!(history().is_empty());
    owned.prompt_state_owner.sync_external_active_prompt(
        &owned.session_store.get_session(&room).unwrap(),
        &agent,
        Some(crate::session::PromptQueueItem::new(
            "goal-check-task-1-1-1",
            "attachment-1",
            &agent,
            "check",
            crate::session::PromptStatus::Running,
        )),
    );
    let achieved =
        r#"{"status":"achieved","final_answer":"Opened two tabs","evidence":["history:x"]}"#;
    // The work turn already showed this answer: the receipt must not repeat it.
    owned.publish_check_answer(&room, &agent, "run", achieved, "Done. Opened two tabs.");
    assert!(history().is_empty());
    owned.publish_check_answer(&room, &agent, "run", achieved, "");
    let entries = history();
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0].text, "Opened two tabs");
    assert_eq!(
        entries[0].prompt_origin,
        Some(crate::session::PromptOrigin::Chariox)
    );
}

// A goal continuation shows a short visible prompt; the original prompt's notes ride as hidden context.
#[tokio::test]
async fn goal_continuation_keeps_the_original_notes_in_hidden_context_not_visible_text() {
    use crate::durable_state::agent_lifecycle::Operation;
    use base64::Engine;
    let worktree = crate::test_support::TestWorktree::new("goal-continuation-notes");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let (_app, runtime, room, agent) = agent_config_runtime_in_worktree(
        config,
        crate::session::DEFAULT_LOCAL_USER_ID,
        worktree
            .session_request()
            .with_agent_defaults(crate::session::SessionAgentDefaults::new("dev-stub")),
    )
    .await;
    let store = &runtime.owned.durable_state_store;
    let now = crate::session::unix_epoch_ms();
    for op in [
        Operation::Begin {
            owner: "local-user".into(),
            room: room.clone(),
            agent: agent.clone(),
            prompt: "goal-notes".into(),
            run: Some("run".into()),
            now,
        },
        Operation::StartGoal {
            task: "goal-notes".into(),
            owner: "local-user".into(),
            objective: "Apply my note".into(),
            minutes: 1,
            now,
        },
    ] {
        store.agent_lifecycle(op).unwrap();
    }
    let note = crate::session::PromptAttachment::new(
        "chariox-terminal://note/1",
        crate::prompt_notes::NOTE_ATTACHMENT_MIME,
        Some("Note".into()),
    )
    .with_contents_base64(
        base64::engine::general_purpose::STANDARD
            .encode(r#"{"quote":"QUOTED_OLD_NAME","comment":"COMMENT_RENAME_IT"}"#),
    );
    let mut entry = crate::history::SessionHistoryEntry::user_prompt_with_attachments(
        &room,
        "att",
        &agent,
        "Apply my note",
        &[note],
    );
    entry.merge_key = Some("prompt:goal-notes".into());
    runtime.owned.append_history_entries(&room, vec![entry]);
    // The work turn settles, the check turn runs and answers "continue".
    store
        .agent_lifecycle(Operation::Settle {
            room: room.clone(),
            agent: agent.clone(),
            prompt: "goal-notes".into(),
            run: "run".into(),
            has_answer: true,
            cancelled: false,
            now,
        })
        .unwrap();
    let check = store
        .agent_tasks(Some(&room), Some(&agent))
        .unwrap()
        .remove(0)
        .pending_prompt_id
        .unwrap();
    store
        .agent_lifecycle(Operation::Begin {
            owner: "local-user".into(),
            room: room.clone(),
            agent: agent.clone(),
            prompt: check.clone(),
            run: Some("check-run".into()),
            now,
        })
        .unwrap();
    store
        .agent_lifecycle(Operation::EvaluateGoal {
            task: "goal-notes".into(),
            prompt: check,
            result: r#"{"status":"continue","reason":"more work remains"}"#.into(),
            now,
        })
        .unwrap();
    let task = store
        .agent_tasks(Some(&room), Some(&agent))
        .unwrap()
        .remove(0);
    assert!(task.pending_prompt_id.is_some());
    let attachment = runtime
        .ensure_agent_message_attachment(
            &room,
            &runtime.owned.agent_store.get_agent(&agent).unwrap(),
        )
        .unwrap();
    let text = runtime
        .owned
        .goal_continuation_context(&task)
        .unwrap()
        .unwrap();
    assert!(text.contains("Apply my note"));
    for private in ["QUOTED_OLD_NAME", "COMMENT_RENAME_IT"] {
        assert!(!text.contains(private), "visible continuation: {text}");
    }
    runtime
        .dispatch_task_continuation(&task, &attachment, text)
        .await
        .unwrap();
    let session = runtime.owned.session_store.get_session(&room).unwrap();
    let queued = session
        .active_prompt_for_agent(&agent)
        .cloned()
        .or_else(|| {
            session
                .queued_prompts_for_agent(&agent)
                .and_then(|queue| queue.front().cloned())
        })
        .expect("continuation prompt submitted");
    let envelope = recovered_continuation_envelope(&runtime, &room, &agent, &queued);
    for private in ["QUOTED_OLD_NAME", "COMMENT_RENAME_IT"] {
        assert!(!queued.prompt().contains(private));
        assert!(!envelope.visible_user_prompt.contains(private));
        assert!(envelope.hidden_system_context.contains(private));
    }
}

// MP-08 / MP-10 / MP-11: catalog instructions and original note-only requirements both survive a fresh provider run.
#[tokio::test]
async fn catalog_continuation_merges_original_notes_with_supplied_hidden_context() {
    use crate::durable_state::agent_lifecycle::Operation;
    use base64::Engine;
    let worktree = crate::test_support::TestWorktree::new("catalog-continuation-notes");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let (_app, runtime, room, agent) = agent_config_runtime_in_worktree(
        config,
        crate::session::DEFAULT_LOCAL_USER_ID,
        worktree
            .session_request()
            .with_agent_defaults(crate::session::SessionAgentDefaults::new("dev-stub")),
    )
    .await;
    let store = &runtime.owned.durable_state_store;
    let now = crate::session::unix_epoch_ms();
    store
        .agent_lifecycle(Operation::Begin {
            owner: "local-user".into(),
            room: room.clone(),
            agent: agent.clone(),
            prompt: "catalog-notes".into(),
            run: Some("run".into()),
            now,
        })
        .unwrap();
    let note = crate::session::PromptAttachment::new(
        "chariox-terminal://note/1",
        crate::prompt_notes::NOTE_ATTACHMENT_MIME,
        Some("Note".into()),
    )
    .with_contents_base64(
        base64::engine::general_purpose::STANDARD
            .encode(r#"{"quote":"QUOTED_OLD_NAME","comment":"COMMENT_RENAME_IT"}"#),
    );
    let mut entry = crate::history::SessionHistoryEntry::user_prompt_with_attachments(
        &room,
        "att",
        &agent,
        "",
        &[note],
    );
    entry.merge_key = Some("prompt:catalog-notes".into());
    runtime.owned.append_history_entries(&room, vec![entry]);
    store
        .agent_lifecycle(Operation::PrepareCatalogContinuation {
            room: room.clone(),
            agent: agent.clone(),
            prompt: "catalog-notes".into(),
            run: "run".into(),
            next_prompt: "task-catalog:catalog-notes".into(),
            now,
        })
        .unwrap();
    store
        .agent_lifecycle(Operation::Settle {
            room: room.clone(),
            agent: agent.clone(),
            prompt: "catalog-notes".into(),
            run: "run".into(),
            has_answer: false,
            cancelled: false,
            now,
        })
        .unwrap();
    let (instructions, _) = crate::prompt_assembly::PromptAssemblyService::from_env()
        .unwrap()
        .assemble_mcp_skill_continuation_context("chariox-runtime")
        .unwrap();
    runtime
        .dispatch_catalog_task_continuation(
            PendingMcpContinuation {
                session_id: room.clone(),
                agent_id: agent.clone(),
                source_prompt_id: Some("catalog-notes".into()),
                mcp_name: "chariox-runtime".into(),
                previous_prompt: "Continue the request".into(),
                reload_reason: ProviderReloadReason::RuntimeToolCatalog,
            },
            None,
            instructions.clone(),
        )
        .await
        .unwrap();
    let session = runtime.owned.session_store.get_session(&room).unwrap();
    let queued = session
        .active_prompt_for_agent(&agent)
        .cloned()
        .or_else(|| {
            session
                .queued_prompts_for_agent(&agent)
                .and_then(|queue| queue.front().cloned())
        })
        .expect("continuation prompt submitted");
    let envelope = recovered_continuation_envelope(&runtime, &room, &agent, &queued);
    assert!(envelope.hidden_system_context.contains(&instructions));
    for private in ["QUOTED_OLD_NAME", "COMMENT_RENAME_IT"] {
        assert!(!envelope.visible_user_prompt.contains(private));
        assert!(envelope.hidden_system_context.contains(private));
    }
}

// MP-08 / MP-10 / MP-11: a correlated wake must recover note-only requirements
// even when no provider thread exists to remember the original submission.
#[tokio::test]
async fn inbox_wake_recovers_original_notes_in_hidden_provider_context() {
    use crate::durable_state::agent_lifecycle::{ExecutionState, Operation, Registration};
    use base64::Engine;
    let worktree = crate::test_support::TestWorktree::new("inbox-wake-notes");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let (_app, runtime, room, agent) = agent_config_runtime_in_worktree(
        config,
        crate::session::DEFAULT_LOCAL_USER_ID,
        worktree
            .session_request()
            .with_agent_defaults(crate::session::SessionAgentDefaults::new("dev-stub")),
    )
    .await;
    let store = &runtime.owned.durable_state_store;
    let now = crate::session::unix_epoch_ms();
    let task_id = "waiting-notes";
    let note = crate::session::PromptAttachment::new(
        "chariox-terminal://note/1",
        crate::prompt_notes::NOTE_ATTACHMENT_MIME,
        Some("Note".into()),
    )
    .with_contents_base64(
        base64::engine::general_purpose::STANDARD
            .encode(r#"{"quote":"QUOTED_WAITING_REQUIREMENT","comment":"COMMENT_AFTER_WAKE"}"#),
    );
    let mut entry = crate::history::SessionHistoryEntry::user_prompt_with_attachments(
        &room,
        "att",
        &agent,
        "Please read the attached notes.",
        &[note],
    );
    entry.merge_key = Some(format!("prompt:{task_id}"));
    runtime.owned.append_history_entries(&room, vec![entry]);
    for op in [
        Operation::Begin {
            owner: "local-user".into(),
            room: room.clone(),
            agent: agent.clone(),
            prompt: task_id.into(),
            run: Some("retired-run".into()),
            now,
        },
        Operation::Subscribe {
            task: task_id.into(),
            prompt: task_id.into(),
            registration: Registration {
                id: "wait-source".into(),
                task_id: task_id.into(),
                source_id: "source".into(),
                obligation_id: None,
                source_cursor: 0,
                live: true,
            },
        },
        Operation::Yield {
            task: task_id.into(),
            prompt: task_id.into(),
            registrations: vec!["wait-source".into()],
            cursor: 0,
            deadline: now + 60_000,
            reason: "Waiting for the source".into(),
            now,
        },
        Operation::Settle {
            room: room.clone(),
            agent: agent.clone(),
            prompt: task_id.into(),
            run: "retired-run".into(),
            has_answer: false,
            cancelled: false,
            now,
        },
    ] {
        store.agent_lifecycle(op).unwrap();
    }
    assert_eq!(
        store.agent_tasks(Some(&room), Some(&agent)).unwrap()[0].state,
        ExecutionState::Waiting
    );
    store
        .agent_lifecycle(Operation::SourceOutcome {
            now,
            room: room.clone(),
            source: "source".into(),
            occurrence: "finished".into(),
            success: true,
            public_answer: None,
        })
        .unwrap();
    Box::pin(runtime.deliver_agent_inbox(&room, &agent))
        .await
        .unwrap();
    let event = store.agent_inbox(&room, &agent, 0).unwrap().remove(0);
    assert_eq!(event.state, "accepted");
    let session = runtime.owned.session_store.get_session(&room).unwrap();
    let prompt = session
        .active_prompt_for_agent(&agent)
        .expect("wake submitted");
    assert_eq!(Some(prompt.id()), event.prompt_id.as_deref());
    let envelope = recovered_continuation_envelope(&runtime, &room, &agent, prompt);
    for private in ["QUOTED_WAITING_REQUIREMENT", "COMMENT_AFTER_WAKE"] {
        assert!(
            !prompt.hidden_system_context().contains(private),
            "recovery belongs to shared task context"
        );
        assert!(!envelope.visible_user_prompt.contains(private));
        assert!(
            envelope.hidden_system_context.contains(private),
            "wake lost {private}"
        );
    }
}

// The production local/leased dispatchers append this shared context immediately
// before provider assembly, rather than persisting notes on a continuation prompt.
fn recovered_continuation_envelope(
    runtime: &KernelRuntimeState,
    room: &str,
    agent: &str,
    prompt: &crate::session::PromptQueueItem,
) -> crate::prompt_assembly::PromptEnvelope {
    let run = runtime
        .owned
        .provider_store
        .get_run_for_agent(room, agent)
        .unwrap();
    assert!(run.resume_state().is_empty());
    assert!(run.provider_session_id().is_none());
    let hint = runtime
        .owned
        .agent_task_context_hint(room, agent, prompt.id())
        .unwrap()
        .unwrap();
    let hidden = format!("{}\n{hint}", prompt.hidden_system_context());
    crate::prompt_assembly::PromptAssemblyService::from_env()
        .unwrap()
        .assemble_provider_turn(
            &run,
            prompt.prompt(),
            Some(&hidden),
            prompt.attachments().to_vec(),
            crate::prompt_assembly::PromptAssemblyMode::McpSkillContinuationTurn
                .with_prompt_origin(prompt.prompt_origin()),
        )
        .unwrap()
}

// MP-08 / MP-10 / MP-11: completion checks get the objective through the same
// hidden dispatch context; their visible labels must not replay attached notes.
#[tokio::test]
async fn completion_check_keeps_notes_and_current_deadlines_in_hidden_context() {
    use crate::durable_state::agent_lifecycle::Operation;
    use base64::Engine;
    let worktree = crate::test_support::TestWorktree::new("completion-check-notes");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let (_app, runtime, room, agent) = agent_config_runtime_in_worktree(
        config,
        crate::session::DEFAULT_LOCAL_USER_ID,
        worktree
            .session_request()
            .with_agent_defaults(crate::session::SessionAgentDefaults::new("dev-stub")),
    )
    .await;
    let store = &runtime.owned.durable_state_store;
    let now = crate::session::unix_epoch_ms();
    let task_id = "check-notes";
    let note = crate::session::PromptAttachment::new(
        "chariox-terminal://note/1",
        crate::prompt_notes::NOTE_ATTACHMENT_MIME,
        Some("Note".into()),
    )
    .with_contents_base64(
        base64::engine::general_purpose::STANDARD
            .encode(r#"{"quote":"QUOTED_CHECK_REQUIREMENT","comment":"COMMENT_FOR_CHECK"}"#),
    );
    let mut entry = crate::history::SessionHistoryEntry::user_prompt_with_attachments(
        &room,
        "att",
        &agent,
        "Please read the attached notes.",
        &[note],
    );
    entry.merge_key = Some(format!("prompt:{task_id}"));
    runtime.owned.append_history_entries(&room, vec![entry]);

    for op in [
        Operation::Begin {
            owner: "local-user".into(),
            room: room.clone(),
            agent: agent.clone(),
            prompt: task_id.into(),
            run: Some("retired-run".into()),
            now,
        },
        Operation::StartGoal {
            task: task_id.into(),
            owner: "local-user".into(),
            objective: "Read the attached notes".into(),
            minutes: 1,
            now,
        },
        Operation::Settle {
            room: room.clone(),
            agent: agent.clone(),
            prompt: task_id.into(),
            run: "retired-run".into(),
            has_answer: true,
            cancelled: false,
            now,
        },
    ] {
        store.agent_lifecycle(op).unwrap();
    }
    let task = store
        .agent_tasks(Some(&room), Some(&agent))
        .unwrap()
        .remove(0);
    let attachment = runtime
        .ensure_agent_message_attachment(
            &room,
            &runtime.owned.agent_store.get_agent(&agent).unwrap(),
        )
        .unwrap();
    let check_context = runtime
        .owned
        .goal_continuation_context(&task)
        .unwrap()
        .unwrap();
    runtime
        .dispatch_task_continuation(&task, &attachment, check_context)
        .await
        .unwrap();
    let session = runtime.owned.session_store.get_session(&room).unwrap();
    let prompt = session
        .active_prompt_for_agent(&agent)
        .expect("check submitted");
    assert_eq!(
        prompt.prompt_origin(),
        crate::session::PromptOrigin::CompletionCheck
    );
    for private in ["QUOTED_CHECK_REQUIREMENT", "COMMENT_FOR_CHECK"] {
        assert!(
            !prompt.prompt().contains(private),
            "visible completion check lost hidden-only contract"
        );
    }
    let envelope = recovered_continuation_envelope(&runtime, &room, &agent, prompt);
    assert!(
        envelope.assessment_only,
        "exercise the real neutral assessment boundary"
    );
    // MP-08 / MP-10 / MP-11: examples must obey the same clock/bound as replies.
    let context = &envelope.hidden_system_context;
    let start = context.find(r#"{"status":"waiting","#).unwrap();
    let end = start + context[start..].find('}').unwrap() + 1;
    let example: serde_json::Value = serde_json::from_str(&context[start..end]).unwrap();
    let data = context
        .rsplit_once("Untrusted assessment data: ")
        .unwrap()
        .1;
    let data = serde_json::Deserializer::from_str(data)
        .into_iter::<serde_json::Value>()
        .next()
        .unwrap()
        .unwrap();
    let now = data["now_ms"].as_u64().unwrap();
    let deadline = example["deadline_ms"].as_u64().unwrap();
    assert!(
        deadline > now,
        "the assessment prompt must not teach an expired deadline"
    );
    assert!(deadline <= now + data["check_in_minutes"].as_u64().unwrap() * 60_000);
    for private in ["QUOTED_CHECK_REQUIREMENT", "COMMENT_FOR_CHECK"] {
        assert!(!envelope.visible_user_prompt.contains(private));
        assert!(envelope.hidden_system_context.contains(private));
    }
}
