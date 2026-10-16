// MP-08 / MP-09 / MP-10 / MP-11 A12-S02: actual native context includes safe actionable receipts.
use super::*;
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
        Operation::EnableCompletionCheck {
            task: prompt.into(),
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
        data["latest_public_answer_tail"],
        "GetDaemonHealth returned a healthy kernel with no projection mismatches."
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
