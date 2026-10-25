//! MP-08 / MP-09 / MP-10 / MP-11 A12 supplementary policy/restart regressions.
use super::tests::Fixture;
use super::*;

fn goal(f: &Fixture, minutes: u32) {
    f.begin("p");
    f.apply(Operation::StartGoal {
        task: "p".into(),
        owner: "owner".into(),
        objective: "Implement and verify real work".into(),
        minutes,
        now: 1,
    });
}
fn check(f: &Fixture, result: serde_json::Value) -> AgentTaskExecution {
    let t = f.task();
    let id = t.pending_prompt_id.unwrap();
    f.begin(&id);
    let Outcome::Settled { task, .. } = f.apply(Operation::EvaluateGoal {
        task: "p".into(),
        prompt: id,
        result: result.to_string(),
        now: 4,
    }) else {
        panic!()
    };
    task
}
fn assessment(status: &str) -> serde_json::Value {
    serde_json::json!({"status":status,"reason":"Useful independent implementation remains"})
}

// MP-08 / MP-09 / MP-10 / MP-11 A12-S02: old work age cannot consume a fresh check's service bound.
#[test]
fn a12_fresh_completion_intent_keeps_its_bound_after_long_work() {
    {
        let f = Fixture::new();
        goal(&f, 1);
        let started = DELIVERY_TIMEOUT_MS + 2;
        f.apply(Operation::Settle {
            room: "room".into(),
            agent: "parent".into(),
            prompt: "p".into(),
            run: "run".into(),
            has_answer: true,
            cancelled: false,
            now: started,
        });
        let task = f.task();
        let check = completion::evaluation(&task).unwrap();
        assert_eq!(check.deadline_ms, started + goals::CHECK_BOUND_MS);
        assert!(
            !lacks_live_executor(&task, started + 1, false, false),
            "a durable pending check has its own bounded admission window"
        );
        assert!(!lacks_live_executor(
            &task,
            check.deadline_ms - 1,
            false,
            false
        ));
        assert!(lacks_live_executor(&task, check.deadline_ms, false, false));
        let mut stale = task.clone();
        stale.pending_prompt_id = Some("other-turn".into());
        assert!(
            lacks_live_executor(&stale, started + 1, false, false),
            "an unrelated check receipt is not a live admission"
        );
        f.apply(Operation::TimeoutGoal {
            task: task.task_id.clone(),
            prompt: check.prompt_id.clone(),
            now: check.deadline_ms,
        });
        assert_eq!(f.task().state, ExecutionState::Blocked);
        assert_eq!(f.task().goal.unwrap().check_count, 1);
    }
}

#[test]
fn a12_each_work_turn_gets_one_check_and_only_goals_continue() {
    let ordinary = Fixture::new();
    ordinary.begin("p");
    ordinary.settle("p", true);
    assert_eq!(ordinary.task().state, ExecutionState::Done);
    let f = Fixture::new();
    goal(&f, 10);
    f.settle("p", true);
    let first = f.task();
    assert_eq!(first.state, ExecutionState::Working);
    assert_eq!(first.goal.as_ref().unwrap().check_count, 1);
    f.settle("p", true);
    assert_eq!(f.task(), first);
    let t = check(&f, assessment("continue"));
    assert_eq!(t.goal.as_ref().unwrap().continuation_sequence, 1);
    let id = t.pending_prompt_id.unwrap();
    f.begin(&id);
    f.settle(&id, true);
    assert_eq!(f.task().goal.unwrap().check_count, 2);
}

#[test]
fn a12_achieved_requires_zero_open_and_evidence_and_never_recurses() {
    let f = Fixture::new();
    goal(&f, 10);
    f.settle("p", true);
    let t = check(
        &f,
        serde_json::json!({"status":"achieved","final_answer":"Verified implementation","evidence":["artifact:commit"]}),
    );
    assert_eq!(t.state, ExecutionState::Done);
    assert!(t.pending_prompt_id.is_none());
    f.settle(&t.prompt_id, true);
    assert_eq!(f.task(), t);
    let f = Fixture::new();
    goal(&f, 10);
    f.register();
    f.settle("p", true);
    let claim =
        serde_json::json!({"status":"achieved","final_answer":"done","evidence":["history"]});
    let t = check(&f, claim.clone());
    assert!(t.correction_used);
    assert_ne!(t.state, ExecutionState::Done);
    let id = t.pending_prompt_id.unwrap();
    f.begin(&id);
    f.settle(&id, true);
    assert_eq!(check(&f, claim).state, ExecutionState::Blocked);
}

#[test]
fn a12_continue_keeps_live_sources_and_wait_checks_coverage_and_interval() {
    let f = Fixture::new();
    goal(&f, 1);
    f.register();
    f.subscribe();
    f.settle("p", true);
    let t = check(&f, assessment("continue"));
    assert_eq!(t.obligations.len(), 1);
    assert!(f.store.agent_registrations("p").unwrap()[0].live);
    let id = t.pending_prompt_id.unwrap();
    f.begin(&id);
    f.settle(&id, true);
    let t = check(
        &f,
        serde_json::json!({"status":"waiting","reason":"depends on delegate","registration_ids":["reg"],"deadline_ms":9_000_000}),
    );
    assert_eq!(t.state, ExecutionState::Waiting);
    assert_eq!(t.wait.unwrap().deadline_ms, 60_004);
    let f = Fixture::new();
    goal(&f, 60);
    f.register();
    f.settle("p", true);
    assert_eq!(check(&f,serde_json::json!({"status":"waiting","reason":"no coverage","registration_ids":["missing"],"deadline_ms":60_000})).state,ExecutionState::Blocked);
}

#[test]
fn a12_failed_eof_malformed_oversize_and_timeout_block_once() {
    let f = Fixture::new();
    goal(&f, 10);
    f.settle("p", false);
    assert_eq!(f.task().state, ExecutionState::Blocked);
    assert_eq!(f.task().goal.unwrap().check_count, 0);
    for (result, now) in [
        ("invalid".into(), 4),
        ("x".repeat(8193), 4),
        (assessment("continue").to_string(), 60_004),
    ] {
        let f = Fixture::new();
        goal(&f, 10);
        f.settle("p", true);
        let id = f.task().pending_prompt_id.unwrap();
        f.begin(&id);
        f.apply(Operation::EvaluateGoal {
            task: "p".into(),
            prompt: id.clone(),
            result: result.clone(),
            now,
        });
        let before = f.task();
        assert_eq!(before.state, ExecutionState::Blocked);
        f.apply(Operation::EvaluateGoal {
            task: "p".into(),
            prompt: id,
            result,
            now,
        });
        assert_eq!(f.task(), before);
    }
}

#[test]
fn a12_immediate_no_progress_guard_survives_restart() {
    let f = Fixture::new();
    goal(&f, 10);
    let mut id = "p".to_string();
    for n in 0..4 {
        f.settle(&id, true);
        let t = check(&f, assessment("continue"));
        let reopened = DurableKernelStateStore::open(f.root.join("state.sqlite")).unwrap();
        assert_eq!(
            reopened.agent_tasks(Some("room"), Some("parent")).unwrap()[0],
            t
        );
        if n == 3 {
            assert_eq!(t.state, ExecutionState::Blocked);
            break;
        }
        id = t.pending_prompt_id.unwrap();
        f.begin(&id);
    }
}

#[test]
fn a10_outdated_goal_revision_keeps_current_work_unchanged() {
    let f = Fixture::new();
    goal(&f, 10);
    let original = f.task();
    f.apply(Operation::ControlGoal {
        task: original.task_id.clone(),
        owner: "owner".into(),
        revision: original.revision,
        action: "interval".into(),
        value: Some("20".into()),
        now: 2,
    });
    let current = f.task();
    let refused = f
        .store
        .agent_lifecycle(Operation::ControlGoal {
            task: original.task_id,
            owner: "owner".into(),
            revision: original.revision,
            action: "edit".into(),
            value: Some("outdated request must not replace the objective".into()),
            now: 3,
        })
        .unwrap_err();
    assert!(
        refused.to_string().contains("no longer current"),
        "{refused}"
    );
    assert_eq!(f.task(), current);
}

#[test]
fn a12_owner_revision_pause_edit_and_cancel_fence_late_checks() {
    let f = Fixture::new();
    goal(&f, 10);
    f.settle("p", true);
    let id = f.task().pending_prompt_id.unwrap();
    f.begin(&id);
    let before = f.task();
    assert!(f
        .store
        .agent_lifecycle(Operation::ControlGoal {
            task: "p".into(),
            owner: "other".into(),
            revision: before.revision,
            action: "pause".into(),
            value: None,
            now: 4
        })
        .is_err());
    f.apply(Operation::ControlGoal {
        task: "p".into(),
        owner: "owner".into(),
        revision: before.revision,
        action: "edit".into(),
        value: Some("Changed owner objective".into()),
        now: 4,
    });
    let edited = f.task();
    assert_eq!(edited.goal.as_ref().unwrap().objective_revision, 2);
    f.apply(Operation::EvaluateGoal {
        task: "p".into(),
        prompt: id,
        result:
            serde_json::json!({"status":"achieved","final_answer":"stale","evidence":["history"]})
                .to_string(),
        now: 5,
    });
    assert_eq!(f.task(), edited);
    f.apply(Operation::ControlGoal {
        task: "p".into(),
        owner: "owner".into(),
        revision: edited.revision,
        action: "resume".into(),
        value: None,
        now: 6,
    });
    let resumed = f.task();
    assert_eq!(
        resumed.goal.as_ref().unwrap().continuation_sequence,
        edited.goal.as_ref().unwrap().continuation_sequence + 1
    );
    assert!(resumed.pending_prompt_id.is_some());
    assert!(f
        .store
        .agent_lifecycle(Operation::ControlGoal {
            task: "p".into(),
            owner: "owner".into(),
            revision: edited.revision,
            action: "resume".into(),
            value: None,
            now: 6
        })
        .is_err());
    f.apply(Operation::ControlGoal {
        task: "p".into(),
        owner: "owner".into(),
        revision: resumed.revision,
        action: "cancel".into(),
        value: None,
        now: 7,
    });
    assert_eq!(f.task().state, ExecutionState::Cancelled);
}

#[test]
fn a12_progressing_goals_have_no_turn_cap_and_evaluations_cannot_mutate_work() {
    let f = Fixture::new();
    goal(&f, 10);
    let mut id = "p".to_string();
    for n in 0..12 {
        f.apply(Operation::Progress {
            task: "p".into(),
            prompt: id.clone(),
            receipt: format!("verified-artifact-{n}"),
            now: 2,
        });
        f.settle(&id, true);
        let check_id = f.task().pending_prompt_id.clone().unwrap();
        f.begin(&check_id);
        assert!(f
            .store
            .agent_lifecycle(Operation::Progress {
                task: "p".into(),
                prompt: check_id.clone(),
                receipt: "assessment-is-not-work".into(),
                now: 4
            })
            .is_err());
        assert!(f
            .store
            .agent_lifecycle(Operation::Subscribe {
                task: "p".into(),
                prompt: check_id.clone(),
                registration: Registration {
                    id: "forged".into(),
                    task_id: "p".into(),
                    source_id: "source".into(),
                    obligation_id: None,
                    source_cursor: 0,
                    live: true
                }
            })
            .is_err());
        let Outcome::Settled { task, .. } = f.apply(Operation::EvaluateGoal {
            task: "p".into(),
            prompt: check_id,
            result: assessment("continue").to_string(),
            now: 4,
        }) else {
            panic!()
        };
        assert_eq!(task.state, ExecutionState::Working);
        assert_eq!(task.progress_sequence, n + 1);
        id = task.pending_prompt_id.unwrap();
        f.begin(&id);
    }
}

#[test]
fn a12_timeout_once_and_owner_resume_fences_its_late_timer_and_result() {
    let f = Fixture::new();
    goal(&f, 10);
    f.settle("p", true);
    let id = f.task().pending_prompt_id.clone().unwrap();
    f.begin(&id);
    f.apply(Operation::TimeoutGoal {
        task: "p".into(),
        prompt: id.clone(),
        now: 60_004,
    });
    let blocked = f.task();
    assert_eq!(blocked.state, ExecutionState::Blocked);
    assert!(matches!(
        f.apply(Operation::TimeoutGoal {
            task: "p".into(),
            prompt: id.clone(),
            now: 60_005
        }),
        Outcome::Saved
    ));
    assert_eq!(f.task(), blocked);
    f.apply(Operation::ControlGoal {
        task: "p".into(),
        owner: "owner".into(),
        revision: blocked.revision,
        action: "resume".into(),
        value: None,
        now: 60_005,
    });
    let resumed = f.task();
    f.apply(Operation::TimeoutGoal {
        task: "p".into(),
        prompt: id.clone(),
        now: 60_006,
    });
    f.apply(Operation::EvaluateGoal {
        task: "p".into(),
        prompt: id,
        result: assessment("continue").to_string(),
        now: 60_006,
    });
    assert_eq!(f.task(), resumed);
}

#[test]
fn a12_owner_adoption_is_atomic_idempotent_and_preserves_existing_nodes() {
    let f = Fixture::new();
    let op = Operation::AdoptGoal {
        owner: "owner".into(),
        room: "room".into(),
        agent: "parent".into(),
        run: "legacy-run".into(),
        nodes: vec!["planner".into(), "worker".into(), "reviewer".into()],
        objective: "Owner-confirmed legacy objective".into(),
        now: 1,
    };
    let Outcome::Task(t) = f.apply(op.clone()) else {
        panic!()
    };
    assert_eq!(t.state, ExecutionState::Waiting);
    assert!(t.pending_prompt_id.is_none());
    assert_eq!(t.obligations.len(), 4);
    assert!(t
        .obligations
        .iter()
        .all(|o| o.resource_id.as_deref() == Some("legacy-run")));
    let Outcome::Task(again) = f.apply(op) else {
        panic!()
    };
    assert_eq!(again, t);
    let reopened = DurableKernelStateStore::open(f.root.join("state.sqlite")).unwrap();
    assert_eq!(reopened.agent_tasks(None, None).unwrap(), vec![t]);
    assert_eq!(
        f.store
            .load_events_by_kind("goal.legacy_adopted")
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn a12_wait_deadlines_default_max_and_owner_interval_remain_finite() {
    for (minutes, interval) in [(10, 600_000), (60, 3_600_000)] {
        let f = Fixture::new();
        goal(&f, minutes);
        f.register();
        f.subscribe();
        f.settle("p", true);
        let t = check(
            &f,
            serde_json::json!({"status":"waiting","reason":"source required","registration_ids":["reg"],"deadline_ms":9_000_000}),
        );
        assert_eq!(t.wait.as_ref().unwrap().deadline_ms, 4 + interval);
        f.apply(Operation::ControlGoal {
            task: "p".into(),
            owner: "owner".into(),
            revision: t.revision,
            action: "interval".into(),
            value: Some("1".into()),
            now: 10,
        });
        assert_eq!(f.task().wait.unwrap().deadline_ms, 60_010);
    }
}

#[test]
fn a12_overdue_restart_coalesces_and_blocked_source_events_never_resume() {
    let f = Fixture::new();
    goal(&f, 1);
    f.register();
    f.subscribe();
    f.settle("p", true);
    let t = check(
        &f,
        serde_json::json!({"status":"waiting","reason":"live delegate needed","registration_ids":["reg"],"deadline_ms":60_004}),
    );
    assert_eq!(t.state, ExecutionState::Waiting);
    let reopened = DurableKernelStateStore::open(f.root.join("state.sqlite")).unwrap();
    for now in [600_000, 660_000, 720_000] {
        reopened
            .agent_lifecycle(Operation::Sweep {
                now,
                busy_recipients: vec![],
                held_work: vec![],
            })
            .unwrap();
    }
    assert_eq!(
        reopened
            .agent_inbox("room", "parent", 0)
            .unwrap()
            .iter()
            .filter(|e| e.kind == "deadline_reached")
            .count(),
        1
    );
    let t = f.task();
    f.apply(Operation::ControlGoal {
        task: "p".into(),
        owner: "owner".into(),
        revision: t.revision,
        action: "pause".into(),
        value: None,
        now: 720_001,
    });
    f.apply(Operation::SourceOutcome {
        room: "room".into(),
        source: "child".into(),
        occurrence: "source-lost".into(),
        success: false,
        public_answer: None,
        now: 720_002,
    });
    assert_eq!(f.task().state, ExecutionState::Blocked);
    assert!(f.task().pending_prompt_id.is_none());
    assert!(f.task().obligations.iter().any(|o| o.status == "failed"));
}

// MP-08 / MP-09 / MP-10 / MP-11 A12-S02: the real overdue restart
// retained an older uncertain message ahead of the Goal's check-in.
#[test]
fn a12_blocked_fifo_delivery_cannot_hide_a_goal_check_in() {
    for blocker_first in [true, false] {
        let f = Fixture::new();
        goal(&f, 1);
        f.register();
        f.subscribe();
        f.settle("p", true);
        let continued = check(&f, assessment("continue"));
        let work = continued.pending_prompt_id.unwrap();
        f.begin(&work);
        f.settle(&work, true);
        let waiting = check(
            &f,
            serde_json::json!({
                "status":"waiting","reason":"delegate needed",
                "registration_ids":["reg"],"deadline_ms":60_004
            }),
        );
        assert_eq!(waiting.no_progress_wakes, 1);
        f.begin("independent");
        let sweep = |now| {
            f.apply(Operation::Sweep {
                now,
                busy_recipients: vec![],
                held_work: vec![],
            })
        };
        if !blocker_first {
            sweep(60_005);
        }
        let mut message = occurrence(
            "room",
            "parent",
            "peer",
            "uncertain-message",
            "message",
            serde_json::json!({"message":"public evidence available"}),
        );
        // Only urgent steering can be admitted ahead of the older pending wake.
        // This exercises a later blocked attempt without bypassing FIFO admission.
        message.urgent = !blocker_first;
        let Outcome::Event(event) = f.apply(Operation::Occur(message)) else {
            panic!()
        };
        f.apply(Operation::Attempt {
            room: "room".into(),
            agent: "parent".into(),
            sequence: event.sequence,
            prompt: "event".into(),
            target: Some("independent".into()),
            run: Some("run".into()),
            now: 60_006,
            work: None,
        });
        sweep(180_006);
        let task = f.task();
        assert_eq!(
            task.state,
            ExecutionState::Blocked,
            "an unresolved recipient delivery must surface the undeliverable Goal check-in"
        );
        assert_eq!(task.no_progress_wakes, 1);
        assert_eq!(
            task.goal.as_ref().unwrap().id,
            waiting.goal.as_ref().unwrap().id
        );
        assert_eq!(task.obligations, waiting.obligations);
        assert_eq!(
            f.store
                .agent_inbox("room", "parent", 0)
                .unwrap()
                .iter()
                .filter(|e| e.kind == "deadline_reached")
                .count(),
            1
        );
        assert!(task
            .reason
            .contains(&format!("delivery {}", event.sequence)));
        assert!(task.goal.as_ref().unwrap().next_check_in_ms.is_none());
        sweep(240_006);
        assert_eq!(
            f.task(),
            task,
            "a repeat sweep cannot re-block or reset the guard"
        );
        f.apply(Operation::Receipt {
            room: "room".into(),
            agent: "parent".into(),
            sequence: event.sequence,
            state: "accepted".into(),
            now: 240_007,
        });
        sweep(300_006);
        assert_eq!(
            f.task(),
            task,
            "a late receipt cannot resume a blocked Goal"
        );
    }
}

#[test]
fn a12_current_ledger_is_rechecked_after_a_source_changes_during_assessment() {
    let f = Fixture::new();
    goal(&f, 10);
    f.register();
    f.subscribe();
    f.settle("p", true);
    let id = f.task().pending_prompt_id.unwrap();
    f.begin(&id);
    f.apply(Operation::SourceOutcome {
        room: "room".into(),
        source: "child".into(),
        occurrence: "real-source-failed".into(),
        success: false,
        public_answer: None,
        now: 4,
    });
    let Outcome::Settled { task, .. } = f.apply(Operation::EvaluateGoal {
        task: "p".into(),
        prompt: id,
        result:
            serde_json::json!({"status":"achieved","final_answer":"done","evidence":["artifact"]})
                .to_string(),
        now: 5,
    }) else {
        panic!()
    };
    assert_ne!(task.state, ExecutionState::Done);
    assert!(task.correction_used);
    assert!(task.obligations.iter().any(|o| o.status == "failed"));
}

#[test]
fn a12_rejected_fresh_admission_leaves_no_goal_or_supervised_work() {
    let f = Fixture::new();
    f.apply(Operation::Begin {
        owner: "owner".into(),
        room: "room".into(),
        agent: "parent".into(),
        prompt: "p".into(),
        run: None,
        now: 1,
    });
    f.apply(Operation::StartGoal {
        task: "p".into(),
        owner: "owner".into(),
        objective: "Owner work".into(),
        minutes: 10,
        now: 1,
    });
    f.apply(Operation::Withdraw { task: "p".into() });
    assert!(f.store.agent_tasks(None, None).unwrap().is_empty());
}

// MP-08 / MP-09 / MP-10 / MP-11 A12: rejection must bound the stored receipt too.
#[test]
fn a12_oversized_json_receipt_never_crosses_public_result_limit() {
    let f = Fixture::new();
    goal(&f, 1);
    f.settle("p", true);
    let t = check(
        &f,
        serde_json::json!({"status":"blocked","reason":"x".repeat(9000)}),
    );
    assert_eq!(t.state, ExecutionState::Blocked);
    let receipt = t.goal.unwrap().evaluation.unwrap().result.unwrap();
    assert!(
        receipt.to_string().len() <= goals::RESULT_LIMIT,
        "rejected result must not retain oversized JSON"
    );
}

// MP-08 / MP-09 / MP-10 / MP-11 A12: confirmation cannot extend work's wait.
#[test]
fn a12_check_preserves_an_earlier_work_deadline() {
    let f = Fixture::new();
    goal(&f, 1);
    f.register();
    f.subscribe();
    f.apply(Operation::Yield {
        task: "p".into(),
        prompt: "p".into(),
        registrations: vec!["reg".into()],
        cursor: 0,
        deadline: 10_000,
        reason: "earlier source deadline".into(),
        now: 2,
    });
    f.settle("p", true);
    let t = check(
        &f,
        serde_json::json!({"status":"waiting", "reason":"depends on delegate", "registration_ids":["reg"], "deadline_ms":60_004}),
    );
    assert_eq!(t.state, ExecutionState::Waiting);
    assert_eq!(t.wait.unwrap().deadline_ms, 10_000);
    assert_eq!(t.goal.unwrap().next_check_in_ms, Some(10_000));
}

#[test]
fn a12_check_preserves_an_earlier_registered_timer_deadline() {
    let f = Fixture::new();
    goal(&f, 1);
    f.apply(Operation::CreateWake {
        task: "p".into(),
        prompt: "p".into(),
        wake: AgentWake {
            id: "timer".into(),
            task_id: "p".into(),
            room_id: "room".into(),
            agent_id: "parent".into(),
            registration_id: "completion-timer".into(),
            kind: "timer".into(),
            label: "source deadline".into(),
            state: String::new(),
            created_at_ms: 2,
            verified_at_ms: None,
            next_due_ms: Some(10_000),
            interval_ms: None,
            command: vec![],
            match_text: None,
            matched_at_ms: None,
            pid: None,
            exit_code: None,
            summary: String::new(),
            detail: serde_json::Value::Null,
            fire_count: 0,
            missed_fires: 0,
            last_fired_at_ms: None,
            last_sequence: None,
            last_delivery: None,
            last_delivered_at_ms: None,
            last_acknowledged_at_ms: None,
            alerted_sequence: None,
        },
    });
    f.apply(Operation::VerifyWakes { now: 3 });
    f.settle("p", true);
    let t = check(
        &f,
        serde_json::json!({"status":"waiting","reason":"depends on timer", "registration_ids":["completion-timer"],"deadline_ms":60_004}),
    );
    assert_eq!(t.state, ExecutionState::Waiting);
    assert_eq!(t.wait.unwrap().deadline_ms, 10_000);
}

// MP-08 / MP-09 / MP-10 / MP-11 A12-S02: a paused loop cannot advertise a new check.
#[test]
fn a12_interval_change_cannot_schedule_a_paused_goal() {
    let f = Fixture::new();
    goal(&f, 10);
    f.register();
    f.subscribe();
    f.settle("p", true);
    let t = check(
        &f,
        serde_json::json!({"status":"waiting","reason":"source required","registration_ids":["reg"],"deadline_ms":600004}),
    );
    f.apply(Operation::ControlGoal {
        task: "p".into(),
        owner: "owner".into(),
        revision: t.revision,
        action: "pause".into(),
        value: None,
        now: 10,
    });
    let paused = f.task();
    f.apply(Operation::ControlGoal {
        task: "p".into(),
        owner: "owner".into(),
        revision: paused.revision,
        action: "interval".into(),
        value: Some("1".into()),
        now: 11,
    });
    let t = f.task();
    assert!(t.goal.as_ref().unwrap().paused);
    assert_eq!(
        t.goal.as_ref().unwrap().next_check_in_ms,
        None,
        "owner interval edits must not schedule checks on a paused goal"
    );
    assert_eq!(
        t.obligations, paused.obligations,
        "live obligations remain supervised while paused"
    );
    assert!(
        t.wait.is_some(),
        "retain source wait metadata for explicit owner revalidation"
    );
}

#[test]
fn a12_unadmitted_checkin_ack_cannot_consume_a_waiting_goal_deadline() {
    let f = Fixture::new();
    goal(&f, 1);
    f.register();
    f.subscribe();
    f.settle("p", true);
    let waiting = check(
        &f,
        serde_json::json!({"status":"waiting","reason":"required peer remains live","registration_ids":["reg"],"deadline_ms":60004}),
    );
    f.apply(Operation::Sweep {
        now: 60005,
        busy_recipients: vec![("room".into(), "parent".into())],
        held_work: vec![],
    });
    let event = f
        .store
        .agent_work_inbox("room", "parent", 0, Some("p"))
        .unwrap()
        .into_iter()
        .find(|e| e.kind == "deadline_reached")
        .unwrap();
    for handled in [false, true] {
        assert!(f.store.agent_lifecycle(Operation::Ack {room:"room".into(),agent:"parent".into(),sequence:event.sequence,handled,now:60006}).is_err(),"an independent turn reading the inbox cannot swallow a not-yet-delivered goal check-in");
    }
    assert_eq!(f.task().state, waiting.state);
    assert_eq!(f.task().no_progress_wakes, waiting.no_progress_wakes);
    assert_eq!(
        f.store
            .agent_delivery_front("room", "parent")
            .unwrap()
            .unwrap()
            .state,
        "pending"
    );
    f.apply(Operation::Attempt {
        room: "room".into(),
        agent: "parent".into(),
        sequence: event.sequence,
        prompt: "goal-wake".into(),
        target: None,
        work: Some("p".into()),
        run: None,
        now: 60007,
    });
    f.apply(Operation::Receipt {
        room: "room".into(),
        agent: "parent".into(),
        sequence: event.sequence,
        state: "accepted".into(),
        now: 60008,
    });
    f.apply(Operation::Ack {
        room: "room".into(),
        agent: "parent".into(),
        sequence: event.sequence,
        handled: true,
        now: 60009,
    });
    assert_eq!(f.task().state, ExecutionState::Working);
    assert_eq!(f.task().no_progress_wakes, waiting.no_progress_wakes + 1);
}

#[test]
fn a12_handled_paused_goal_outcome_preserves_nonzero_guard() {
    let f = Fixture::new();
    goal(&f, 1);
    f.register();
    f.subscribe();
    f.settle("p", true);
    check(
        &f,
        serde_json::json!({"status":"waiting","reason":"peer required","registration_ids":["reg"],"deadline_ms":60004}),
    );
    f.apply(Operation::Sweep {
        now: 60005,
        busy_recipients: vec![],
        held_work: vec![],
    });
    let event = f
        .store
        .agent_work_inbox("room", "parent", 0, Some("p"))
        .unwrap()
        .remove(0);
    f.apply(Operation::Attempt {
        room: "room".into(),
        agent: "parent".into(),
        sequence: event.sequence,
        prompt: "goal-wake".into(),
        target: None,
        work: Some("p".into()),
        run: None,
        now: 60006,
    });
    f.apply(Operation::Receipt {
        room: "room".into(),
        agent: "parent".into(),
        sequence: event.sequence,
        state: "accepted".into(),
        now: 60007,
    });
    f.apply(Operation::ControlGoal {
        task: "p".into(),
        owner: "owner".into(),
        revision: f.task().revision,
        action: "pause".into(),
        value: None,
        now: 60008,
    });
    f.apply(Operation::SourceOutcome {
        room: "room".into(),
        source: "child".into(),
        occurrence: "completed-while-paused".into(),
        success: true,
        public_answer: None,
        now: 60009,
    });
    let event = f
        .store
        .agent_work_inbox("room", "parent", 0, Some("p"))
        .unwrap()
        .into_iter()
        .find(|e| e.kind == "source_completed")
        .unwrap();
    f.apply(Operation::Expire {
        room: "room".into(),
        agent: "parent".into(),
        sequence: event.sequence,
    });
    let before = f.task();
    assert_eq!(before.no_progress_wakes, 1);
    f.apply(Operation::Ack {
        room: "room".into(),
        agent: "parent".into(),
        sequence: event.sequence,
        handled: true,
        now: 60010,
    });
    let after = f.task();
    assert_eq!(after.obligations[0].status, "satisfied");
    assert_eq!(after.state, ExecutionState::Blocked);
    assert!(after.goal.unwrap().paused);
    assert_eq!(after.no_progress_wakes, 1);
    assert_eq!(after.progress_sequence, before.progress_sequence);
    assert_eq!(after.blocked_revision, before.blocked_revision);
}
// MP-08 / MP-09 / MP-10 / MP-11 A12 (accam r3): a workflow node's goal
// check is a continuation of its bound task. It keeps that lineage and can
// neither rebind it nor block the B->C edge until the 60 s bound.
#[test]
fn a12_workflow_bound_check_keeps_its_lineage_and_settles() {
    let f = Fixture::new();
    goal(&f, 1);
    let bind = |prompt: &str, node: &str| {
        f.store.agent_lifecycle(Operation::BindWorkflow {
            task: "p".into(),
            prompt: prompt.into(),
            run: "run".into(),
            node: node.into(),
        })
    };
    bind("p", "node").unwrap();
    f.settle("p", true);
    let id = f.task().pending_prompt_id.clone().unwrap();
    f.begin(&id);
    bind(&id, "node").expect("the check continuation keeps its workflow lineage");
    assert!(bind(&id, "other-node").is_err());
    let Outcome::Settled { task, .. } = f.apply(Operation::EvaluateGoal {
        task: "p".into(),
        prompt: id,
        result: serde_json::json!({"status":"achieved","final_answer":"Built and tested","evidence":["public:artifact"]}).to_string(),
        now: 4,
    }) else {
        panic!()
    };
    assert_eq!(task.state, ExecutionState::Done);
    assert_eq!(
        f.store.agent_task_workflow("p").unwrap(),
        Some(("run".into(), "node".into()))
    );
}

// A turn admitted before it had a provider run (queued behind another turn)
// settles with the run that ended it; its check must read that run's answer.
#[test]
fn completion_check_names_the_run_that_ended_the_work_turn() {
    let f = Fixture::new();
    f.apply(Operation::Begin {
        owner: "owner".into(),
        room: "room".into(),
        agent: "parent".into(),
        prompt: "p".into(),
        run: None,
        now: 1,
    });
    f.apply(Operation::StartGoal {
        task: "p".into(),
        owner: "owner".into(),
        objective: "Implement and verify real work".into(),
        minutes: 1,
        now: 1,
    });
    f.settle("p", true);
    let task = f.task();
    let check = completion::evaluation(&task).unwrap();
    assert_eq!(check.work_provider_run_id.as_deref(), Some("run"));
}

// The 60 s check bound is a service bound, not an owner deadline: the time the check waits
// for the owner's Vault unlock is given back, and a refusal is a definitive Blocked.
fn wait_for_owner(f: &Fixture, at: u64) -> (String, goals::GoalEvaluation) {
    goal(f, 1);
    f.settle("p", true);
    let task = f.task();
    let check = completion::evaluation(&task).unwrap().clone();
    f.apply(Operation::BeginOwnerWait {
        task: task.task_id.clone(),
        prompt: check.prompt_id.clone(),
        now: at,
    });
    (task.task_id, check)
}

#[test]
fn owner_wait_stops_the_check_bound_and_an_unlock_gives_the_time_back() {
    let f = Fixture::new();
    let (task, check) = wait_for_owner(&f, 5_000);
    let waiting = f.task();
    assert_eq!(waiting.state, ExecutionState::Working);
    assert_eq!(waiting.reason, "Waiting for the Vault unlock");
    assert!(!lacks_live_executor(
        &waiting,
        check.deadline_ms * 10,
        false,
        false
    ));
    f.apply(Operation::TimeoutGoal {
        task: task.clone(),
        prompt: check.prompt_id.clone(),
        now: check.deadline_ms + 300_000,
    });
    assert_eq!(
        f.task().state,
        ExecutionState::Working,
        "the wait is not a timeout"
    );
    let paused_at = completion::evaluation(&f.task())
        .unwrap()
        .paused_at_ms
        .unwrap();
    let unlocked_at = paused_at + 420_000;
    f.apply(Operation::EndOwnerWait {
        task: task.clone(),
        prompt: check.prompt_id.clone(),
        refusal: None,
        now: unlocked_at,
    });
    let resumed = f.task();
    let after = completion::evaluation(&resumed).unwrap();
    assert_eq!(after.paused_at_ms, None);
    assert_eq!(after.deadline_ms, check.deadline_ms + 420_000);
    assert_eq!(
        after.deadline_ms,
        after.started_at_ms + goals::CHECK_BOUND_MS
    );
    assert_eq!(resumed.reason, "Evaluating original goal objective");
    f.apply(Operation::TimeoutGoal {
        task,
        prompt: check.prompt_id.clone(),
        now: after.deadline_ms,
    });
    assert_eq!(
        f.task().state,
        ExecutionState::Blocked,
        "the bound counts again after the unlock"
    );
}

#[test]
fn owner_refusal_blocks_with_the_reason_and_resume() {
    let f = Fixture::new();
    let (task, check) = wait_for_owner(&f, 1_000);
    f.apply(Operation::EndOwnerWait {
        task,
        prompt: check.prompt_id,
        refusal: Some("The Vault unlock was declined".into()),
        now: 9_000,
    });
    let blocked = f.task();
    assert_eq!(blocked.state, ExecutionState::Blocked);
    assert_eq!(
        blocked.reason,
        "The Vault unlock was declined; unlock the Vault and Resume"
    );
    assert_eq!(completion::evaluation(&blocked).unwrap().paused_at_ms, None);
}

// #1044 P1 on the goal ledger: an unacknowledged receipt of a clean source exit
// ("settling") must never let any verdict sequence save Done; the persisted task reloads valid.
#[test]
fn a12_settling_obligation_never_saves_done_on_repeated_verdicts() {
    let f = Fixture::new();
    goal(&f, 10);
    f.register();
    f.subscribe();
    f.apply(Operation::SourceOutcome {
        now: 2,
        public_answer: None,
        room: "room".into(),
        source: "child".into(),
        occurrence: "result".into(),
        success: true,
    });
    assert_eq!(f.task().obligations[0].status, "settling");
    f.settle("p", true);
    let claim =
        serde_json::json!({"status":"achieved","final_answer":"done","evidence":["history"]});
    for verdict in [
        assessment("continue"),
        assessment("continue"),
        claim.clone(),
        claim,
    ] {
        let t = check(&f, verdict);
        let saved = f.task();
        assert_eq!(saved, t);
        assert_ne!(saved.state, ExecutionState::Done);
        assert_eq!(saved.obligations[0].status, "settling");
        let Some(id) = saved.pending_prompt_id else {
            break;
        };
        f.begin(&id);
        f.settle(&id, true);
    }
    assert_eq!(f.task().state, ExecutionState::Blocked);
}

// A12-S02/S07: a continuation that made no handled progress and yielded again on
// live sources covering every obligation keeps that wait; a repeated continue
// must not discard it and spend the guard while the build is still running.
#[test]
fn a12_repeated_continue_keeps_the_agents_covered_wait() {
    let f = Fixture::new();
    goal(&f, 1);
    f.register();
    f.subscribe();
    let yield_on = |prompt: &str, now: u64| {
        f.apply(Operation::Yield {
            task: "p".into(),
            prompt: prompt.into(),
            registrations: vec!["reg".into()],
            cursor: 0,
            deadline: 9_000_000,
            reason: "build still running".into(),
            now,
        });
    };
    yield_on("p", 2);
    f.settle("p", true);
    let first = check(&f, assessment("continue"));
    assert_eq!(first.state, ExecutionState::Working);
    let id = first.pending_prompt_id.unwrap();
    f.begin(&id);
    yield_on(&id, 3);
    f.settle(&id, true);
    let t = check(&f, assessment("continue"));
    assert_eq!(t.state, ExecutionState::Waiting, "{t:?}");
    assert_eq!(t.no_progress_wakes, first.no_progress_wakes);
    let wait = t.wait.clone().unwrap();
    assert_eq!(wait.registration_ids, vec!["reg".to_string()]);
    assert_eq!(
        t.goal.as_ref().unwrap().next_check_in_ms,
        Some(wait.deadline_ms)
    );
    assert!(wait.deadline_ms <= 3 + 60_000);
    assert_eq!(f.task(), t);
}

// The kept wait never bypasses the guard: at the limit a covered wait still blocks.
#[test]
fn a12_kept_wait_still_ends_at_the_progress_guard() {
    let f = Fixture::new();
    goal(&f, 1);
    f.register();
    f.subscribe();
    f.settle("p", true);
    let mut t = check(&f, assessment("continue"));
    for _ in 1..NO_PROGRESS_LIMIT {
        let id = t.pending_prompt_id.clone().unwrap();
        f.begin(&id);
        f.settle(&id, true);
        t = check(&f, assessment("continue"));
    }
    assert_eq!(t.no_progress_wakes, NO_PROGRESS_LIMIT);
    let id = t.pending_prompt_id.clone().unwrap();
    f.begin(&id);
    f.apply(Operation::Yield {
        task: "p".into(),
        prompt: id.clone(),
        registrations: vec!["reg".into()],
        cursor: 0,
        deadline: 9_000_000,
        reason: "build still running".into(),
        now: 3,
    });
    f.settle(&id, true);
    assert_eq!(
        check(&f, assessment("continue")).state,
        ExecutionState::Blocked
    );
}

fn lease(id: &str) -> goals::CheckLease {
    goals::CheckLease {
        worker_kernel_id: "worker".into(),
        worker_machine_id: "machine".into(),
        execution_lease_id: id.into(),
        leased_agent_id: "leased".into(),
    }
}

fn bound_check(f: &Fixture, lease_id: &str) -> goals::GoalEvaluation {
    goal(f, 10);
    f.settle("p", true);
    let prompt = completion::evaluation(&f.task()).unwrap().prompt_id.clone();
    f.begin(&prompt);
    f.apply(Operation::BindGoalCheckLease {
        task: "p".into(),
        prompt,
        lease: lease(lease_id),
    });
    completion::evaluation(&f.task()).unwrap().clone()
}

fn retry(
    f: &Fixture,
    check: &goals::GoalEvaluation,
    lease: goals::CheckLease,
    now: u64,
) -> Outcome {
    f.apply(Operation::RetryGoalCheck {
        task: "p".into(),
        prompt: f
            .task()
            .pending_prompt_id
            .unwrap_or(check.prompt_id.clone()),
        lease,
        now,
    })
}

// MP-08 / MP-10 / MP-11 A12-S02: a leased worker that restarts inside a running check gets the check once more.
#[test]
fn a12_check_interrupted_by_a_worker_restart_is_asked_once_more() {
    let f = Fixture::new();
    let check = bound_check(&f, "lease-1");
    let Outcome::Task(task) = retry(&f, &check, lease("lease-2"), check.deadline_ms) else {
        panic!("a restarted worker must get its check back");
    };
    let again = completion::evaluation(&task).unwrap();
    assert_eq!(task.state, ExecutionState::Working);
    assert_eq!(again.prompt_id, format!("{}-retry", check.prompt_id));
    assert_eq!(
        task.pending_prompt_id.as_deref(),
        Some(again.prompt_id.as_str())
    );
    assert_eq!(again.deadline_ms, check.deadline_ms + goals::CHECK_BOUND_MS);
    assert!(again.retried && again.result.is_none());
    assert!(goals::is_completion_check_prompt_id(&again.prompt_id));
    // The retry is the last: a second restart (or any expiry) blocks the task.
    f.begin(&again.prompt_id);
    assert!(matches!(
        retry(&f, again, lease("lease-3"), again.deadline_ms),
        Outcome::Saved
    ));
    f.apply(Operation::TimeoutGoal {
        task: "p".into(),
        prompt: again.prompt_id.clone(),
        now: again.deadline_ms,
    });
    assert_eq!(f.task().state, ExecutionState::Blocked);
}

#[test]
fn a12_check_is_not_retried_unless_the_same_worker_came_back_after_the_bound() {
    let other_worker = goals::CheckLease {
        worker_kernel_id: "other".into(),
        ..lease("lease-2")
    };
    for (current, early, why) in [
        (
            lease("lease-1"),
            false,
            "same lease: the worker never restarted",
        ),
        (
            other_worker,
            false,
            "another worker never answers for this check",
        ),
        (lease("lease-2"), true, "the service bound has not run out"),
    ] {
        let f = Fixture::new();
        let check = bound_check(&f, "lease-1");
        let now = check.deadline_ms - u64::from(early);
        assert!(
            matches!(retry(&f, &check, current, now), Outcome::Saved),
            "{why}"
        );
        assert_eq!(completion::evaluation(&f.task()).unwrap(), &check);
    }
    let f = Fixture::new();
    goal(&f, 10);
    f.settle("p", true);
    let unleased = completion::evaluation(&f.task()).unwrap().clone();
    f.begin(&unleased.prompt_id);
    assert!(
        matches!(
            retry(&f, &unleased, lease("lease-2"), unleased.deadline_ms),
            Outcome::Saved
        ),
        "a check that never ran on a leased worker keeps the plain bound"
    );
}

// MP-08 / MP-09 / MP-10 / MP-11 A12-S04: an owner's native approval wait is not service time of
// the completion check that asked for it.
#[test]
fn a12_native_approval_wait_does_not_spend_the_completion_check_bound() {
    let f = Fixture::new();
    goal(&f, 1);
    f.settle("p", true);
    let pending = f.task();
    let check = completion::evaluation(&pending).unwrap().clone();
    f.begin(&check.prompt_id);
    let task = f.task();
    f.apply(Operation::BeginGoalApproval {
        task: "p".into(),
        prompt: check.prompt_id.clone(),
        now: check.started_at_ms + 5_000,
    });
    let blocked = f.task();
    assert_eq!(blocked.state, ExecutionState::Blocked);
    let answered_at = check.deadline_ms + 300_000;
    f.apply(Operation::AnswerGoalApproval {
        task: "p".into(),
        prompt: check.prompt_id.clone(),
        revision: blocked.blocked_revision,
        now: answered_at,
    });
    let resumed = f.task();
    assert_eq!(resumed.state, ExecutionState::Working);
    let after = completion::evaluation(&resumed).unwrap();
    assert_eq!(after.paused_at_ms, None);
    assert_eq!(
        after.deadline_ms,
        answered_at + goals::CHECK_BOUND_MS - 5_000,
        "the remaining check budget survives the approval wait"
    );
    f.apply(Operation::TimeoutGoal {
        task: "p".into(),
        prompt: check.prompt_id.clone(),
        now: check.deadline_ms + 1,
    });
    assert_eq!(
        f.task().state,
        ExecutionState::Working,
        "the original deadline no longer applies"
    );
    let Outcome::Settled { task: settled, .. } = f.apply(Operation::EvaluateGoal {
        task: task.task_id,
        prompt: check.prompt_id,
        result: assessment("continue").to_string(),
        now: answered_at + 1_000,
    }) else {
        panic!()
    };
    assert!(
        completion::evaluation(&settled).is_none_or(|e| e.result.is_some()
            && e.result.as_ref().unwrap()["status"] != "blocked"),
        "an approval answered after the original deadline still finishes its check"
    );
}
