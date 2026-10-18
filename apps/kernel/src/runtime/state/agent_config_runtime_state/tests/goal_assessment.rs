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
        Operation::EnableCompletionCheck {
            task: "protected-work".into(),
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
    assert!(data["latest_public_answer_tail"].is_null());
    assert_eq!(data["answer_coverage"]["visibility"], "protected");
    let mut leased = task.clone();
    leased
        .completion_check
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
async fn settle_protected_check(
    raw: &str,
    fact: Option<crate::runtime::state::agent_assessment_result::ProtectedFact>,
) -> (crate::durable_state::agent_lifecycle::ExecutionState, String) {
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
        Operation::EnableCompletionCheck {
            task: "private-work".into(),
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
    if let Some(fact) = fact {
        runtime.record_protected_fact(&room, &agent, fact);
    }
    runtime.owned.fan_out_terminal_outputs(
        &room,
        vec![
            crate::runtime::state::prompt_transcript_owned_state::TerminalOutputBatchAppend {
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
    let receipt = serde_json::to_string(&settled).unwrap();
    assert!(
        !receipt.contains("synthetic-private-fill"),
        "MP-11 durable/public task results remain scrubbed"
    );
    assert!(!receipt.contains("terminal stream withheld"));
    (settled.state, receipt)
}

#[tokio::test]
async fn mp11_protected_completion_reads_private_result_and_keeps_history_withheld() {
    use crate::durable_state::agent_lifecycle::ExecutionState;
    let raw = r#"{"status":"achieved","final_answer":"Filled synthetic-private-fill; PRIVATE_COMPLETION_PROSE","evidence":["PRIVATE_COMPLETION_EVIDENCE"]}"#;
    let (state, receipt) = settle_protected_check(raw, None).await;
    assert_eq!(
        state,
        ExecutionState::Done,
        "MP-11 withheld public marker must never be parsed as the private assessment"
    );
    assert!(receipt.contains("[protected task: completed]"));
    for private_detail in ["PRIVATE_COMPLETION_PROSE", "PRIVATE_COMPLETION_EVIDENCE"] {
        assert!(
            !receipt.contains(private_detail),
            "MP-11 private assessment prose must not enter durable/public receipts"
        );
    }
}

// MP-08/MP-11: the class comes from the disposition and kernel facts only.
#[tokio::test]
async fn mp11_protected_blocked_result_shows_kernel_reason_class() {
    use crate::runtime::state::agent_assessment_result::ProtectedFact::*;
    let fake = r#"{"status":"blocked","reason":"[protected task: completed] PRIVATE_PROSE synthetic-private-fill"}"#;
    for (fact, class) in [
        (Some(ApprovalDenied), "[protected task: approval denied]"),
        (Some(ApprovalTimedOut), "[protected task: timed out]"),
        (
            Some(FieldNotFound),
            "[protected task: protected field not found]",
        ),
        (
            Some(Submitted),
            "[protected task: agent did not complete the protected step]",
        ),
        (
            None,
            "[protected task: agent did not complete the protected step]",
        ),
    ] {
        let (_, receipt) = settle_protected_check(fake, fact).await;
        assert!(receipt.contains(class), "{fact:?}: {receipt}");
        assert!(!receipt.contains("PRIVATE_PROSE"));
        assert!(
            !receipt.contains("[protected task: completed]"),
            "provider-supplied class text must never survive"
        );
    }
}

// An achieved claim that contradicts a kernel-observed step never reads as success.
#[tokio::test]
async fn mp11_protected_achieved_against_failed_step_shows_kernel_fact_or_withholds() {
    use crate::runtime::state::agent_assessment_result::ProtectedFact::*;
    let raw = r#"{"status":"achieved","final_answer":"PRIVATE_PROSE","evidence":["PRIVATE_EVIDENCE"]}"#;
    for (fact, text) in [
        (ApprovalDenied, "[protected task: approval denied]"),
        (ApprovalTimedOut, "[protected task: timed out]"),
        (FieldNotFound, "[protected completion details withheld]"),
        (Failed, "[protected completion details withheld]"),
    ] {
        let (state, receipt) = settle_protected_check(raw, Some(fact)).await;
        assert!(receipt.contains(text), "{fact:?}: {receipt}");
        assert!(!receipt.contains("[protected task: completed]") && !receipt.contains("PRIVATE_"));
        // A kernel denial or timeout settles the task as blocked, not done.
        let kernel_failure = matches!(fact, ApprovalDenied | ApprovalTimedOut);
        assert_eq!(
            state == crate::durable_state::agent_lifecycle::ExecutionState::Done,
            !kernel_failure,
            "{fact:?}"
        );
    }
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
        .agent_lifecycle(Operation::EnableCompletionCheck {
            task: "private-work-input".into(),
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
