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
        known_block: None,
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
    for is_goal in [false, true] {
        let f = Fixture::new();
        if is_goal {
            goal(&f, 1);
        } else {
            plain(&f);
        }
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
            known_block: None,
            task: task.task_id.clone(),
            prompt: check.prompt_id.clone(),
            now: check.deadline_ms,
        });
        assert_eq!(
            f.task().state,
            ExecutionState::Done,
            "a timed-out check is no objection to a Goal or a plain request"
        );
        assert_eq!(
            if is_goal {
                f.task().goal.unwrap().check_count
            } else {
                f.task().completion_check.unwrap().check_count
            },
            1
        );
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
fn a12_failed_eof_blocks_and_unusable_checks_settle_once() {
    let f = Fixture::new();
    goal(&f, 10);
    f.settle("p", false);
    assert_eq!(f.task().state, ExecutionState::Blocked);
    assert_eq!(f.task().goal.unwrap().check_count, 0);
    for (result, now) in [
        ("invalid".into(), 4),
        ("x".repeat(8193), 4),
        (
            assessment("continue").to_string(),
            goals::CHECK_BOUND_MS + 4,
        ),
    ] {
        let f = Fixture::new();
        goal(&f, 10);
        f.settle("p", true);
        let id = f.task().pending_prompt_id.unwrap();
        f.begin(&id);
        f.apply(Operation::EvaluateGoal {
            known_block: None,
            task: "p".into(),
            prompt: id.clone(),
            result: result.clone(),
            now,
        });
        let before = f.task();
        assert_eq!(before.state, ExecutionState::Done);
        f.apply(Operation::EvaluateGoal {
            known_block: None,
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
        known_block: None,
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
            known_block: None,
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
fn a12_timeout_once_fences_its_late_timer_and_result() {
    let f = Fixture::new();
    goal(&f, 10);
    f.settle("p", true);
    let id = f.task().pending_prompt_id.clone().unwrap();
    f.begin(&id);
    f.apply(Operation::TimeoutGoal {
        known_block: None,
        task: "p".into(),
        prompt: id.clone(),
        now: goals::CHECK_BOUND_MS + 4,
    });
    let done = f.task();
    assert_eq!(done.state, ExecutionState::Done);
    assert!(matches!(
        f.apply(Operation::TimeoutGoal {
            known_block: None,
            task: "p".into(),
            prompt: id.clone(),
            now: goals::CHECK_BOUND_MS + 5
        }),
        Outcome::Saved
    ));
    f.apply(Operation::EvaluateGoal {
        known_block: None,
        task: "p".into(),
        prompt: id,
        result: assessment("continue").to_string(),
        now: goals::CHECK_BOUND_MS + 6,
    });
    assert_eq!(f.task(), done);
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
        known_block: None,
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

// MP-08 / MP-09 / MP-10 / MP-11 A12: plain confirmation never grants goal mode.
#[test]
fn a12_plain_done_candidate_must_be_confirmed_without_a_goal_loop() {
    let f = Fixture::new();
    f.begin("p");
    f.apply(Operation::EnableCompletionCheck { task: "p".into() });
    f.settle("p", true);
    assert_eq!(
        f.task().state,
        ExecutionState::Working,
        "plain done candidate needs a native check"
    );
}

// MP-08 / MP-09 / MP-10 / MP-11 A12: supplementary plain/delegate parity guards.
fn plain(f: &Fixture) {
    f.begin("p");
    f.apply(Operation::EnableCompletionCheck { task: "p".into() });
}
#[test]
fn a12_plain_check_has_one_correction_and_no_continuation_mode() {
    let f = Fixture::new();
    plain(&f);
    f.settle("p", true);
    let first = f.task();
    f.settle("p", true);
    assert_eq!(first, f.task());
    let t = check(&f, assessment("continue"));
    assert!(t.goal.is_none());
    assert!(t.correction_used);
    assert_eq!(t.completion_check.as_ref().unwrap().check_count, 1);
    assert_eq!(t.pending_prompt_id.as_deref(), Some("task-correction-p"));
    let id = t.pending_prompt_id.unwrap();
    f.begin(&id);
    f.settle(&id, true);
    let t = check(&f, assessment("continue"));
    assert_eq!(
        t.state,
        ExecutionState::Done,
        "a repeated verdict with no open obligations is no objection to a plain request"
    );
    assert!(t.pending_prompt_id.is_none());
    assert_eq!(t.completion_check.unwrap().check_count, 2);
    let f = Fixture::new();
    plain(&f);
    f.register();
    f.settle("p", true);
    let t = check(&f, assessment("continue"));
    let id = t.pending_prompt_id.unwrap();
    f.begin(&id);
    f.settle(&id, true);
    assert_eq!(
        check(&f, assessment("continue")).state,
        ExecutionState::Blocked,
        "an open obligation still needs the owner"
    );
    // A cleanly exited source whose receipt nobody acknowledged keeps the task waiting for it:
    // Done with a settling obligation is not a valid saved state.
    let f = Fixture::new();
    plain(&f);
    f.register();
    f.subscribe();
    f.apply(Operation::SourceOutcome {
        room: "room".into(),
        source: "child".into(),
        occurrence: "source-done".into(),
        success: true,
        public_answer: None,
        now: 2,
    });
    assert!(f.task().obligations.iter().any(|o| o.status == "settling"));
    f.settle("p", true);
    let t = check(&f, assessment("continue"));
    let id = t.pending_prompt_id.unwrap();
    f.begin(&id);
    f.settle(&id, true);
    check(&f, assessment("continue"));
    // Reload the persisted task: a quarantined row would project Blocked here.
    let t = f.task();
    assert_eq!(t.state, ExecutionState::Waiting);
    assert_eq!(t.wait.unwrap().registration_ids, vec!["reg"]);
    assert!(t.obligations.iter().any(|o| o.status == "settling"));
}
#[test]
fn a12_plain_achieved_needs_evidence_and_fences_checks_from_work() {
    let f = Fixture::new();
    plain(&f);
    f.settle("p", true);
    let id = f.task().pending_prompt_id.clone().unwrap();
    f.begin(&id);
    assert!(f
        .store
        .agent_lifecycle(Operation::Progress {
            task: "p".into(),
            prompt: id.clone(),
            receipt: "forbidden".into(),
            now: 4
        })
        .is_err());
    let Outcome::Settled{task:t,..}=f.apply(Operation::EvaluateGoal { known_block: None, task:"p".into(),prompt:id.clone(),result:serde_json::json!({"status":"achieved","final_answer":"Verified original request","evidence":["public:artifact"]}).to_string(),now:4 }) else {panic!()};
    assert_eq!(t.state, ExecutionState::Done);
    assert!(t.goal.is_none());
    f.settle(&id, true);
    assert_eq!(f.task(), t);
}
#[test]
fn a12_plain_wait_and_failure_are_not_done_candidates() {
    let f = Fixture::new();
    plain(&f);
    f.register();
    f.subscribe();
    f.apply(Operation::Yield {
        task: "p".into(),
        prompt: "p".into(),
        registrations: vec!["reg".into()],
        cursor: 0,
        deadline: 60000,
        reason: "depends on peer".into(),
        now: 2,
    });
    f.settle("p", true);
    assert_eq!(f.task().state, ExecutionState::Waiting);
    assert_eq!(f.task().completion_check.unwrap().check_count, 0);
    let f = Fixture::new();
    plain(&f);
    f.settle("p", false);
    let t = f.task();
    assert!(t.correction_used);
    assert_eq!(t.completion_check.unwrap().check_count, 0);
}
#[test]
fn a12_plain_timeout_settles_done_idempotently_and_fences_a_late_check() {
    let f = Fixture::new();
    plain(&f);
    f.settle("p", true);
    let id = f.task().pending_prompt_id.unwrap();
    f.begin(&id);
    f.apply(Operation::TimeoutGoal {
        known_block: None,
        task: "p".into(),
        prompt: id.clone(),
        now: goals::CHECK_BOUND_MS + 3,
    });
    let t = f.task();
    assert_eq!(
        t.state,
        ExecutionState::Done,
        "a late check must not block a plain request"
    );
    assert!(matches!(
        f.apply(Operation::TimeoutGoal {
            known_block: None,
            task: "p".into(),
            prompt: id.clone(),
            now: goals::CHECK_BOUND_MS + 1000
        }),
        Outcome::Saved
    ));
    f.apply(Operation::EvaluateGoal {
        known_block: None,
        task: "p".into(),
        prompt: id,
        result: serde_json::json!({"status":"achieved","final_answer":"late","evidence":["late"]})
            .to_string(),
        now: goals::CHECK_BOUND_MS + 1001,
    });
    assert_eq!(f.task(), t);
}

// An empty or malformed verdict is no objection to a plain request or a Goal.
#[test]
fn a12_unusable_verdict_settles_plain_and_goal_done() {
    for (is_goal, raw) in [
        (false, ""),
        (false, "not json"),
        (true, ""),
        (true, "not json"),
    ] {
        let f = Fixture::new();
        if is_goal {
            goal(&f, 1);
        } else {
            plain(&f);
        }
        f.settle("p", true);
        let id = f.task().pending_prompt_id.unwrap();
        f.begin(&id);
        f.apply(Operation::EvaluateGoal {
            known_block: None,
            task: "p".into(),
            prompt: id,
            result: raw.into(),
            now: 4,
        });
        assert_eq!(
            f.task().state,
            ExecutionState::Done,
            "goal={is_goal} raw={raw:?}"
        );
    }
}

// An unusable check (malformed, or past its bound) must not drop open work of a plain task.
fn unusable_plain_check(f: &Fixture, timeout: bool) {
    f.settle("p", true);
    let id = f.task().pending_prompt_id.unwrap();
    f.begin(&id);
    if timeout {
        f.apply(Operation::TimeoutGoal {
            known_block: None,
            task: "p".into(),
            prompt: id,
            now: goals::CHECK_BOUND_MS + 3,
        });
    } else {
        f.apply(Operation::EvaluateGoal {
            known_block: None,
            task: "p".into(),
            prompt: id,
            result: "not json".into(),
            now: 4,
        });
    }
}
// A kernel-observed denial or timeout survives an unusable check, with no obligation involved.
#[test]
fn a12_unusable_check_keeps_a_known_protected_denial() {
    const CLASS: &str = "[protected task: approval denied]";
    for (is_goal, timeout) in [(false, false), (false, true), (true, false), (true, true)] {
        let f = Fixture::new();
        if is_goal {
            goal(&f, 1);
        } else {
            plain(&f);
        }
        f.settle("p", true);
        let id = f.task().pending_prompt_id.unwrap();
        f.begin(&id);
        if timeout {
            f.apply(Operation::TimeoutGoal {
                task: "p".into(),
                prompt: id,
                known_block: Some(CLASS.into()),
                now: goals::CHECK_BOUND_MS + 3,
            });
        } else {
            f.apply(Operation::EvaluateGoal {
                task: "p".into(),
                prompt: id,
                result: "not json".into(),
                known_block: Some(CLASS.into()),
                now: 4,
            });
        }
        let t = f.task();
        assert_eq!(
            t.state,
            ExecutionState::Blocked,
            "goal={is_goal} timeout={timeout}"
        );
        assert_eq!(t.reason, CLASS, "goal={is_goal} timeout={timeout}");
    }
}
fn source_outcome(f: &Fixture, success: bool, now: u64) {
    f.apply(Operation::SourceOutcome {
        room: "room".into(),
        source: "child".into(),
        occurrence: "source-outcome".into(),
        success,
        public_answer: None,
        now,
    });
}
#[test]
fn a12_unusable_check_keeps_a_plain_task_waiting_on_open_work() {
    for timeout in [false, true] {
        let f = Fixture::new();
        plain(&f);
        f.register();
        f.subscribe();
        unusable_plain_check(&f, timeout);
        let t = f.task();
        assert_eq!(t.state, ExecutionState::Waiting, "timeout={timeout}");
        assert_eq!(t.wait.as_ref().unwrap().registration_ids, vec!["reg"]);
        assert!(t.obligations.iter().all(|o| o.status == "open"));
        source_outcome(&f, true, goals::CHECK_BOUND_MS + 10);
        assert!(
            f.task().obligations.iter().all(|o| o.status == "settling"),
            "a later outcome must still reach the task (timeout={timeout})"
        );
    }
}
#[test]
fn a12_unusable_check_keeps_a_failed_obligation_failed() {
    for timeout in [false, true] {
        let f = Fixture::new();
        plain(&f);
        f.register();
        f.subscribe();
        source_outcome(&f, false, 2);
        unusable_plain_check(&f, timeout);
        let t = f.task();
        assert_eq!(t.state, ExecutionState::Blocked, "timeout={timeout}");
        assert!(t.obligations.iter().any(|o| o.status == "failed"));
    }
}
#[test]
fn a12_unusable_check_waits_for_an_unacknowledged_clean_exit_receipt() {
    for timeout in [false, true] {
        let f = Fixture::new();
        plain(&f);
        f.register();
        f.subscribe();
        source_outcome(&f, true, 2);
        assert!(f.task().obligations.iter().any(|o| o.status == "settling"));
        unusable_plain_check(&f, timeout);
        let t = f.task();
        assert_eq!(t.state, ExecutionState::Waiting, "timeout={timeout}");
        assert_eq!(t.wait.unwrap().registration_ids, vec!["reg"]);
    }
}

// An unusable check on a Goal keeps open work waiting and a failed obligation blocked too.
#[test]
fn a12_unusable_check_keeps_a_goal_waiting_on_open_work() {
    for timeout in [false, true] {
        let f = Fixture::new();
        goal(&f, 1);
        f.register();
        f.subscribe();
        unusable_plain_check(&f, timeout);
        let t = f.task();
        assert_eq!(t.state, ExecutionState::Waiting, "timeout={timeout}");
        assert_eq!(t.wait.as_ref().unwrap().registration_ids, vec!["reg"]);
        assert_eq!(
            t.goal.unwrap().next_check_in_ms,
            Some(t.wait.unwrap().deadline_ms)
        );
        let f = Fixture::new();
        goal(&f, 1);
        f.register();
        f.subscribe();
        source_outcome(&f, false, 2);
        unusable_plain_check(&f, timeout);
        assert_eq!(f.task().state, ExecutionState::Blocked, "timeout={timeout}");
    }
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
    assert_eq!(t.state, ExecutionState::Done);
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
// MP-08 / MP-09 / MP-10 / MP-11 A12 (accam r3): a workflow node's completion
// check is a continuation of its bound task. It keeps that lineage and can
// neither rebind it nor block the B->C edge until the 60 s bound.
#[test]
fn a12_workflow_bound_check_keeps_its_lineage_and_settles() {
    let f = Fixture::new();
    plain(&f);
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
    let Outcome::Settled { task, .. } = f.apply(Operation::EvaluateGoal { known_block: None,
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
