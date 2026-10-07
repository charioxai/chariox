//! MP-08 / MP-09 / MP-10 / MP-11 A02 supplementary single-writer fault tests.
use super::*;
struct Fixture {
    root: std::path::PathBuf,
    store: DurableKernelStateStore,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("chariox-am2-tests-{:032x}", rand::random::<u128>()));
        std::fs::create_dir(&root).unwrap();
        let store = DurableKernelStateStore::open_owned(root.join("state.sqlite")).unwrap();
        Self { root, store }
    }
    fn apply(&self, op: Operation) -> Outcome {
        self.store.agent_lifecycle(op).unwrap()
    }
    fn begin(&self, prompt: &str) {
        self.apply(Operation::Begin {
            owner: "owner".into(),
            room: "room".into(),
            agent: "parent".into(),
            prompt: prompt.into(),
            run: Some("run".into()),
            now: 1,
        });
    }
    fn task(&self) -> AgentTaskExecution {
        self.store
            .agent_tasks(Some("room"), Some("parent"))
            .unwrap()
            .remove(0)
    }
    fn register(&self) {
        self.apply(Operation::RegisterObligation {
            owner: "owner".into(),
            room: "room".into(),
            agent: "parent".into(),
            prompt: "p".into(),
            run: Some("run".into()),
            id: "obligation".into(),
            kind: "delegate".into(),
            resource: Some("child".into()),
            now: 1,
        });
    }
    fn subscribe(&self) {
        self.apply(Operation::Subscribe {
            task: "p".into(),
            prompt: "p".into(),
            registration: Registration {
                id: "reg".into(),
                task_id: "p".into(),
                source_id: "child".into(),
                obligation_id: Some("obligation".into()),
                source_cursor: 0,
                live: true,
            },
        });
    }
    fn yield_now(&self) {
        self.apply(Operation::Yield {
            task: "p".into(),
            prompt: "p".into(),
            registrations: vec!["reg".into()],
            cursor: 0,
            deadline: 60_000,
            reason: "child result".into(),
            now: 2,
        });
    }
    fn settle(&self, prompt: &str, answer: bool) -> Outcome {
        self.apply(Operation::Settle {
            room: "room".into(),
            agent: "parent".into(),
            prompt: prompt.into(),
            run: "run".into(),
            has_answer: answer,
            cancelled: false,
            now: 3,
        })
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
#[test]
fn a02_done_with_obligation_corrects_once_across_restart() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    let Outcome::Settled { task, correction } = f.settle("p", true) else {
        panic!()
    };
    assert!(correction);
    assert!(task.correction_used);
    assert_ne!(task.state, ExecutionState::Done);
    let id = task.pending_prompt_id.unwrap();
    f.begin(&id);
    let Outcome::Settled { task, correction } = f.settle(&id, true) else {
        panic!()
    };
    assert!(!correction);
    assert_eq!(task.state, ExecutionState::Blocked);
    assert!(task.correction_used);
    let reopened = DurableKernelStateStore::open(f.root.join("state.sqlite")).unwrap();
    assert_eq!(
        reopened.agent_tasks(Some("room"), Some("parent")).unwrap()[0],
        task
    );
}
#[test]
fn a02_yield_commits_at_native_settlement_and_recovers_racing_result() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    f.subscribe();
    f.yield_now();
    assert_eq!(f.task().state, ExecutionState::Working);
    f.apply(Operation::SourceOutcome {
        public_answer: None,
        room: "room".into(),
        source: "child".into(),
        occurrence: "result".into(),
        success: true,
        now: 2,
    });
    let Outcome::Settled { task, correction } = f.settle("p", true) else {
        panic!()
    };
    assert!(correction || task.state == ExecutionState::Waiting);
    assert_ne!(task.state, ExecutionState::Done);
    assert_eq!(f.store.agent_inbox("room", "parent", 0).unwrap().len(), 1);
}
#[test]
fn a02_deadline_occurrence_and_long_notice_coalesce() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    f.subscribe();
    f.yield_now();
    f.settle("p", true);
    assert_eq!(f.task().state, ExecutionState::Waiting);
    f.apply(Operation::Sweep {
        now: LONG_WAIT_MS + 1,
    });
    f.apply(Operation::Sweep {
        now: LONG_WAIT_MS + 2,
    });
    assert!(f.task().wait.unwrap().long_wait_notified);
    assert_eq!(f.store.agent_inbox("room", "parent", 0).unwrap().len(), 1);
}
#[test]
fn a02_empty_or_tool_only_end_never_silently_done() {
    let f = Fixture::new();
    f.begin("p");
    let Outcome::Settled { task, correction } = f.settle("p", false) else {
        panic!()
    };
    assert!(correction);
    assert_ne!(task.state, ExecutionState::Done);
}
#[test]
fn a02_yield_refuses_missing_dead_or_uncovered_sources() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    for ids in [vec![], vec!["missing".into()]] {
        assert!(f
            .store
            .agent_lifecycle(Operation::Yield {
                task: "p".into(),
                prompt: "p".into(),
                registrations: ids,
                cursor: 0,
                deadline: 10,
                reason: "waiting".into(),
                now: 1
            })
            .is_err());
    }
    f.subscribe();
    assert!(f
        .store
        .agent_lifecycle(Operation::Yield {
            task: "p".into(),
            prompt: "p".into(),
            registrations: vec!["reg".into()],
            cursor: 0,
            deadline: 1,
            reason: "waiting".into(),
            now: 1
        })
        .is_err());
}
#[test]
fn a02_uncertain_attempt_blocks_fifo_and_ack_never_repairs_it() {
    let f = Fixture::new();
    f.begin("p");
    let Outcome::Event(e) = f.apply(Operation::Occur(occurrence(
        "room",
        "parent",
        "peer",
        "one",
        "message",
        serde_json::json!({"message":"result"}),
    ))) else {
        panic!()
    };
    let duplicate = f.apply(Operation::Occur(e.clone()));
    let Outcome::Event(d) = duplicate else {
        panic!()
    };
    assert_eq!(e.sequence, d.sequence);
    f.apply(Operation::Attempt {
        room: "room".into(),
        agent: "parent".into(),
        sequence: e.sequence,
        prompt: "wake".into(),
        target: Some("p".into()),
        run: Some("run".into()),
        now: 10,
    });
    f.apply(Operation::Receipt {
        room: "room".into(),
        agent: "parent".into(),
        sequence: e.sequence,
        state: "uncertain".into(),
    });
    assert!(f
        .store
        .agent_lifecycle(Operation::Ack {
            room: "room".into(),
            agent: "parent".into(),
            sequence: e.sequence,
            handled: true,
            now: 11
        })
        .is_err());
    assert!(f
        .store
        .agent_lifecycle(Operation::Attempt {
            room: "room".into(),
            agent: "parent".into(),
            sequence: e.sequence,
            prompt: "second".into(),
            target: None,
            run: None,
            now: 12
        })
        .is_err());
    f.apply(Operation::Sweep {
        now: DELIVERY_TIMEOUT_MS + 10,
    });
    assert_eq!(f.task().state, ExecutionState::Blocked);
}
#[test]
fn a02_independent_task_never_hides_older_wait() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    f.subscribe();
    f.yield_now();
    f.settle("p", true);
    f.begin("new");
    f.settle("new", true);
    let tasks = f.store.agent_tasks(Some("room"), Some("parent")).unwrap();
    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks[0].state, ExecutionState::Waiting);
    assert_eq!(tasks[1].state, ExecutionState::Done);
}
#[test]
fn a02_stale_owner_and_foreign_ack_are_denied() {
    let f = Fixture::new();
    f.begin("p");
    f.apply(Operation::Block {
        task: "p".into(),
        prompt: "p".into(),
        reason: "need owner resource".into(),
    });
    assert!(f
        .store
        .agent_lifecycle(Operation::OwnerResponse {
            task: "p".into(),
            revision: 0,
            resume: true,
            now: 2
        })
        .is_err());
    let Outcome::Event(e) = f.apply(Operation::Occur(occurrence(
        "room",
        "parent",
        "peer",
        "one",
        "message",
        serde_json::json!({}),
    ))) else {
        panic!()
    };
    assert!(f
        .store
        .agent_lifecycle(Operation::Ack {
            room: "foreign".into(),
            agent: "parent".into(),
            sequence: e.sequence,
            handled: true,
            now: 3
        })
        .is_err());
}
#[test]
fn a02_no_progress_blocks_on_third_wake_and_durable_counter_survives_reopen() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    f.subscribe();
    f.yield_now();
    f.settle("p", true);
    for n in 1..=3 {
        let Outcome::Event(e) = f.apply(Operation::Occur(occurrence(
            "room",
            "parent",
            "timer",
            &format!("tick-{n}"),
            "deadline_reached",
            serde_json::json!({"task_id":"p"}),
        ))) else {
            panic!()
        };
        let prompt = format!("wake-{n}");
        f.apply(Operation::Attempt {
            room: "room".into(),
            agent: "parent".into(),
            sequence: e.sequence,
            prompt: prompt.clone(),
            target: None,
            run: None,
            now: 10 + n,
        });
        f.apply(Operation::Receipt {
            room: "room".into(),
            agent: "parent".into(),
            sequence: e.sequence,
            state: "accepted".into(),
        });
        f.begin(&prompt);
        f.apply(Operation::Yield {
            task: "p".into(),
            prompt: prompt.clone(),
            registrations: vec!["reg".into()],
            cursor: e.sequence,
            deadline: 60_000,
            reason: "same child wait".into(),
            now: 20 + n,
        });
        f.settle(&prompt, true);
        if n < 3 {
            assert_eq!(f.task().state, ExecutionState::Waiting);
        } else {
            assert_eq!(f.task().state, ExecutionState::Blocked);
        }
        let reopened = DurableKernelStateStore::open(f.root.join("state.sqlite")).unwrap();
        assert_eq!(
            reopened.agent_tasks(Some("room"), Some("parent")).unwrap()[0].no_progress_wakes,
            n as u32
        );
    }
}
#[test]
fn a02_corrupt_delivery_is_quarantined_without_replay() {
    let f = Fixture::new();
    f.begin("p");
    let Outcome::Event(e) = f.apply(Operation::Occur(occurrence(
        "room",
        "parent",
        "peer",
        "one",
        "message",
        serde_json::json!({}),
    ))) else {
        panic!()
    };
    let db = Connection::open(f.root.join("state.sqlite")).unwrap();
    db.execute(
        "UPDATE agent_inbox SET payload='broken' WHERE sequence=?1",
        [sql_integer(e.sequence).unwrap()],
    )
    .unwrap();
    f.apply(Operation::Sweep { now: 10 });
    let row = f
        .store
        .agent_delivery_front("room", "parent")
        .unwrap()
        .unwrap();
    assert_eq!(row.state, "blocked");
    assert!(f
        .store
        .agent_tasks(Some("room"), Some("parent"))
        .unwrap()
        .iter()
        .any(|t| t.state == ExecutionState::Blocked));
    let count: i64 = db
        .query_row("SELECT count(*) FROM agent_lifecycle_quarantine", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
}
#[test]
fn a02_source_occurrence_preceding_subscription_is_recovered() {
    let f = Fixture::new();
    f.begin("p");
    f.apply(Operation::SourceOutcome {
        public_answer: None,
        room: "room".into(),
        source: "peer".into(),
        occurrence: "answer".into(),
        success: true,
        now: 2,
    });
    f.apply(Operation::Subscribe {
        task: "p".into(),
        prompt: "p".into(),
        registration: Registration {
            id: "reg".into(),
            task_id: "p".into(),
            source_id: "peer".into(),
            obligation_id: None,
            source_cursor: 0,
            live: true,
        },
    });
    assert_eq!(
        f.store.agent_inbox("room", "parent", 0).unwrap()[0].kind,
        "source_completed"
    );
}

#[test]
fn a02_exact_late_receipt_unlocks_owner_resume_and_cancel_abandons_without_replay() {
    for resume in [false, true] {
        let f = Fixture::new();
        f.begin("p");
        let Outcome::Event(e) = f.apply(Operation::Occur(occurrence(
            "room",
            "parent",
            "peer",
            "one",
            "message",
            serde_json::json!({}),
        ))) else {
            panic!()
        };
        f.apply(Operation::Attempt {
            room: "room".into(),
            agent: "parent".into(),
            sequence: e.sequence,
            prompt: "event".into(),
            target: Some("p".into()),
            run: Some("run".into()),
            now: 1,
        });
        f.apply(Operation::Sweep {
            now: DELIVERY_TIMEOUT_MS + 1,
        });
        let blocked = f.task();
        assert_eq!(blocked.state, ExecutionState::Blocked);
        assert!(f
            .store
            .agent_lifecycle(Operation::OwnerResponse {
                task: "p".into(),
                revision: blocked.blocked_revision,
                resume: true,
                now: DELIVERY_TIMEOUT_MS + 2
            })
            .is_err());
        if resume {
            f.apply(Operation::Receipt {
                room: "room".into(),
                agent: "parent".into(),
                sequence: e.sequence,
                state: "accepted".into(),
            });
        }
        f.apply(Operation::OwnerResponse {
            task: "p".into(),
            revision: blocked.blocked_revision,
            resume,
            now: DELIVERY_TIMEOUT_MS + 3,
        });
        assert!(f
            .store
            .agent_delivery_front("room", "parent")
            .unwrap()
            .is_none());
        assert_eq!(
            f.store.agent_inbox("room", "parent", 0).unwrap()[0].state,
            if resume { "accepted" } else { "failed" }
        );
    }
}
#[test]
fn a02_named_wake_keeps_independent_waits_separate() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    f.subscribe();
    f.yield_now();
    f.settle("p", true);
    f.begin("other");
    f.apply(Operation::Subscribe {
        task: "other".into(),
        prompt: "other".into(),
        registration: Registration {
            id: "other-reg".into(),
            task_id: "other".into(),
            source_id: "peer".into(),
            obligation_id: None,
            source_cursor: 0,
            live: true,
        },
    });
    f.apply(Operation::Yield {
        task: "other".into(),
        prompt: "other".into(),
        registrations: vec!["other-reg".into()],
        cursor: 0,
        deadline: 60_000,
        reason: "peer".into(),
        now: 2,
    });
    f.settle("other", true);
    let Outcome::Event(e) = f.apply(Operation::Occur(occurrence(
        "room",
        "parent",
        "peer",
        "result",
        "source_completed",
        serde_json::json!({"task_id":"other"}),
    ))) else {
        panic!()
    };
    f.apply(Operation::Attempt {
        room: "room".into(),
        agent: "parent".into(),
        sequence: e.sequence,
        prompt: "wake".into(),
        target: None,
        run: None,
        now: 3,
    });
    let tasks = f.store.agent_tasks(Some("room"), Some("parent")).unwrap();
    assert_eq!(tasks[0].state, ExecutionState::Waiting);
    assert_eq!(tasks[0].no_progress_wakes, 0);
    assert_eq!(tasks[1].pending_prompt_id.as_deref(), Some("wake"));
    assert_eq!(tasks[1].no_progress_wakes, 1);
}
#[test]
fn a02_unsubscribe_wakes_retained_wait_and_stale_turn_is_denied() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    f.subscribe();
    f.yield_now();
    assert!(f
        .store
        .agent_lifecycle(Operation::Unsubscribe {
            task: "p".into(),
            prompt: "stale".into(),
            registration: "reg".into()
        })
        .is_err());
    f.apply(Operation::Unsubscribe {
        task: "p".into(),
        prompt: "p".into(),
        registration: "reg".into(),
    });
    assert!(!f.store.agent_registrations("p").unwrap()[0].live);
    assert_eq!(
        f.store.agent_inbox("room", "parent", 0).unwrap()[0].kind,
        "source_lost"
    );
}

#[test]
fn a02_payload_redaction_precedes_persistence_and_deduplication() {
    let f = Fixture::new();
    f.begin("p");
    let e = occurrence(
        "room",
        "parent",
        "peer",
        "redaction",
        "message",
        serde_json::json!({"password":"synthetic-sensitive-value","nested":{"access_token":"synthetic-sensitive-value"},"safe":"public result"}),
    );
    let Outcome::Event(first) = f.apply(Operation::Occur(e.clone())) else {
        panic!()
    };
    let Outcome::Event(second) = f.apply(Operation::Occur(e)) else {
        panic!()
    };
    assert_eq!(first.sequence, second.sequence);
    assert!(!encode(&first)
        .unwrap()
        .contains("synthetic-sensitive-value"));
    assert_eq!(first.payload["safe"], "public result");
    assert!(!encode(&f.store.agent_inbox("room", "parent", 0).unwrap())
        .unwrap()
        .contains("synthetic-sensitive-value"));
}
#[test]
fn a02_clock_rollback_wakes_after_a_persisted_sweep() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    f.subscribe();
    f.yield_now();
    f.settle("p", true);
    f.apply(Operation::Sweep { now: 20 });
    f.apply(Operation::Sweep { now: 10 });
    let inbox = f.store.agent_inbox("room", "parent", 0).unwrap();
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].kind, "deadline_reached");
}

#[test]
fn a02_superseded_native_settlement_cannot_replace_a_corrected_task() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    let Outcome::Settled { task, .. } = f.settle("p", true) else {
        panic!()
    };
    let correction = task.pending_prompt_id.unwrap();
    f.begin(&correction);
    let before = f.task();
    assert!(f
        .store
        .agent_lifecycle(Operation::Settle {
            room: "room".into(),
            agent: "parent".into(),
            prompt: "p".into(),
            run: "run".into(),
            has_answer: true,
            cancelled: false,
            now: 4
        })
        .is_err());
    assert!(f
        .store
        .agent_lifecycle(Operation::Begin {
            owner: "foreign".into(),
            room: "foreign".into(),
            agent: "peer".into(),
            prompt: "p".into(),
            run: None,
            now: 4
        })
        .is_err());
    assert_eq!(f.task(), before);
}

#[test]
fn a02_first_delegate_task_binding_is_not_replaced_by_independent_work() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    f.apply(Operation::DispatchReceipt {
        id: "obligation".into(),
        accepted: true,
        resource: Some("child".into()),
    });
    for prompt in ["first-child-task", "independent-child-task"] {
        f.apply(Operation::Begin {
            owner: "owner".into(),
            room: "room".into(),
            agent: "child".into(),
            prompt: prompt.into(),
            run: Some("child-run".into()),
            now: 2,
        });
    }
    assert_eq!(
        f.task().obligations[0].completion_task_id.as_deref(),
        Some("first-child-task")
    );
    assert_eq!(
        f.store.agent_registrations("p").unwrap()[0].source_id,
        "first-child-task"
    );
    f.apply(Operation::SourceOutcome {
        public_answer: None,
        room: "room".into(),
        source: "independent-child-task".into(),
        occurrence: "other-result".into(),
        success: true,
        now: 3,
    });
    assert_eq!(f.task().obligations[0].status, "open");
    f.apply(Operation::SourceOutcome {
        public_answer: None,
        room: "room".into(),
        source: "first-child-task".into(),
        occurrence: "result".into(),
        success: true,
        now: 3,
    });
    assert_eq!(f.task().obligations[0].status, "settling");
}
#[test]
fn a02_owner_cancel_closes_sources_without_faking_physical_completion() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    f.subscribe();
    f.apply(Operation::CancelTask {
        task: "p".into(),
        owner: "owner".into(),
        revision: f.task().revision,
    });
    assert_eq!(f.task().state, ExecutionState::Cancelled);
    assert_eq!(f.task().obligations[0].status, "open");
    assert_eq!(f.task().obligations[0].dispatch_state, "cancel_requested");
    assert!(!f.store.agent_registrations("p").unwrap()[0].live);
    f.apply(Operation::SourceOutcome {
        public_answer: None,
        room: "room".into(),
        source: "child".into(),
        occurrence: "physical-cancellation".into(),
        success: false,
        now: 3,
    });
    assert_eq!(f.task().obligations[0].status, "cancelled");
    assert_eq!(f.task().state, ExecutionState::Cancelled);
}
#[test]
fn a02_default_message_does_not_replace_an_independent_wait() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    f.subscribe();
    f.yield_now();
    f.settle("p", true);
    let Outcome::Event(e) = f.apply(Operation::Occur(occurrence(
        "room",
        "parent",
        "peer",
        "new-task",
        "message",
        serde_json::json!({}),
    ))) else {
        panic!()
    };
    f.apply(Operation::Attempt {
        room: "room".into(),
        agent: "parent".into(),
        sequence: e.sequence,
        prompt: "message-task".into(),
        target: None,
        run: None,
        now: 4,
    });
    assert_eq!(f.task().state, ExecutionState::Waiting);
    assert_eq!(f.task().no_progress_wakes, 0);
}

#[test]
fn a02_one_source_occurrence_reaches_multiple_tasks_without_conflict() {
    let f = Fixture::new();
    for (prompt, id) in [("p", "first"), ("second", "second")] {
        f.begin(prompt);
        f.apply(Operation::Subscribe {
            task: prompt.into(),
            prompt: prompt.into(),
            registration: Registration {
                id: id.into(),
                task_id: prompt.into(),
                source_id: "peer".into(),
                obligation_id: None,
                source_cursor: 0,
                live: true,
            },
        });
    }
    f.apply(Operation::SourceOutcome {
        public_answer: None,
        room: "room".into(),
        source: "peer".into(),
        occurrence: "one-result".into(),
        success: true,
        now: 2,
    });
    let inbox = f.store.agent_inbox("room", "parent", 0).unwrap();
    assert_eq!(inbox.len(), 1);
    assert_eq!(
        inbox[0].payload["task_ids"],
        serde_json::json!(["p", "second"])
    );
    f.apply(Operation::SourceOutcome {
        public_answer: None,
        room: "room".into(),
        source: "peer".into(),
        occurrence: "one-result".into(),
        success: true,
        now: 3,
    });
    assert_eq!(f.store.agent_inbox("room", "parent", 0).unwrap().len(), 1);
}

#[test]
fn a02_artifact_receipt_resets_guard_once_and_survives_restart() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    f.subscribe();
    f.yield_now();
    f.settle("p", true);
    let Outcome::Event(event) = f.apply(Operation::Occur(occurrence(
        "room",
        "parent",
        "timer",
        "tick",
        "deadline_reached",
        serde_json::json!({"task_id":"p"}),
    ))) else {
        panic!()
    };
    f.apply(Operation::Attempt {
        room: "room".into(),
        agent: "parent".into(),
        sequence: event.sequence,
        prompt: "wake".into(),
        target: None,
        run: None,
        now: 10,
    });
    f.apply(Operation::Receipt {
        room: "room".into(),
        agent: "parent".into(),
        sequence: event.sequence,
        state: "accepted".into(),
    });
    f.begin("wake");
    assert_eq!(f.task().no_progress_wakes, 1);
    assert!(f
        .store
        .agent_lifecycle(Operation::Progress {
            task: "p".into(),
            prompt: "p".into(),
            receipt: "artifact-1".into(),
            now: 11,
        })
        .is_err());
    f.apply(Operation::Progress {
        task: "p".into(),
        prompt: "wake".into(),
        receipt: "artifact-1".into(),
        now: 12,
    });
    let recorded = f.task();
    assert_eq!(recorded.no_progress_wakes, 0);
    assert_eq!(recorded.last_progress_at_ms, 12);
    let reopened = DurableKernelStateStore::open(f.root.join("state.sqlite")).unwrap();
    reopened
        .agent_lifecycle(Operation::Progress {
            task: "p".into(),
            prompt: "wake".into(),
            receipt: "artifact-1".into(),
            now: 999,
        })
        .unwrap();
    assert_eq!(
        reopened.agent_tasks(Some("room"), Some("parent")).unwrap()[0],
        recorded
    );
}

#[test]
fn a02_corrupt_source_is_quarantined_without_poisoning_other_tasks() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    f.subscribe();
    f.yield_now();
    f.settle("p", true);
    f.begin("independent");
    f.apply(Operation::Subscribe {
        task: "independent".into(),
        prompt: "independent".into(),
        registration: Registration {
            id: "other-reg".into(),
            task_id: "independent".into(),
            source_id: "child".into(),
            obligation_id: None,
            source_cursor: 0,
            live: true,
        },
    });
    let db = Connection::open(f.root.join("state.sqlite")).unwrap();
    db.execute(
        "UPDATE agent_registrations SET payload='broken' WHERE id='reg'",
        [],
    )
    .unwrap();
    f.apply(Operation::SourceOutcome {
        public_answer: None,
        room: "room".into(),
        source: "child".into(),
        occurrence: "terminal".into(),
        success: true,
        now: 10,
    });
    let tasks = f.store.agent_tasks(Some("room"), Some("parent")).unwrap();
    let blocked = tasks.iter().find(|t| t.task_id == "p").unwrap();
    assert_eq!(blocked.state, ExecutionState::Blocked);
    assert_eq!(
        tasks
            .iter()
            .find(|t| t.task_id == "independent")
            .unwrap()
            .state,
        ExecutionState::Working
    );
    assert!(f
        .store
        .agent_inbox("room", "parent", 0)
        .unwrap()
        .iter()
        .any(|e| e.kind == "source_completed" && e.payload["task_id"] == "independent"));
    let retained: i64 = db
        .query_row(
            "SELECT count(*) FROM agent_lifecycle_quarantine WHERE kind='registration'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(retained, 1);
    assert!(f
        .store
        .agent_lifecycle(Operation::OwnerResponse {
            task: "p".into(),
            revision: blocked.blocked_revision,
            resume: true,
            now: 20,
        })
        .is_err());
    f.apply(Operation::OwnerResponse {
        task: "p".into(),
        revision: blocked.blocked_revision,
        resume: false,
        now: 20,
    });
    assert_eq!(
        f.store
            .agent_tasks(Some("room"), Some("parent"))
            .unwrap()
            .iter()
            .find(|t| t.task_id == "p")
            .unwrap()
            .state,
        ExecutionState::Cancelled
    );
}

#[test]
fn a02_urgent_steers_past_nonurgent_queue_but_never_uncertain_receipts() {
    let f = Fixture::new();
    let Outcome::Event(normal) = f.apply(Operation::Occur(occurrence(
        "room",
        "parent",
        "sender",
        "normal",
        "message",
        serde_json::json!({}),
    ))) else {
        panic!()
    };
    let mut urgent = occurrence(
        "room",
        "parent",
        "sender",
        "urgent",
        "message",
        serde_json::json!({}),
    );
    urgent.urgent = true;
    let Outcome::Event(urgent) = f.apply(Operation::Occur(urgent)) else {
        panic!()
    };
    assert_eq!(
        f.store
            .agent_delivery_front("room", "parent")
            .unwrap()
            .unwrap()
            .sequence,
        normal.sequence
    );
    assert_eq!(
        f.store
            .agent_urgent_delivery_front("room", "parent")
            .unwrap()
            .unwrap()
            .sequence,
        urgent.sequence
    );
    f.apply(Operation::Attempt {
        room: "room".into(),
        agent: "parent".into(),
        sequence: urgent.sequence,
        prompt: "steer".into(),
        target: Some("running".into()),
        run: Some("run".into()),
        now: 1,
    });
    f.apply(Operation::Receipt {
        room: "room".into(),
        agent: "parent".into(),
        sequence: urgent.sequence,
        state: "uncertain".into(),
    });
    let mut later = occurrence(
        "room",
        "parent",
        "sender",
        "later",
        "message",
        serde_json::json!({}),
    );
    later.urgent = true;
    let Outcome::Event(later) = f.apply(Operation::Occur(later)) else {
        panic!()
    };
    assert_eq!(
        f.store
            .agent_urgent_delivery_front("room", "parent")
            .unwrap()
            .unwrap()
            .sequence,
        urgent.sequence
    );
    assert!(f
        .store
        .agent_lifecycle(Operation::Attempt {
            room: "room".into(),
            agent: "parent".into(),
            sequence: later.sequence,
            prompt: "later".into(),
            target: Some("running".into()),
            run: Some("run".into()),
            now: 2,
        })
        .is_err());
    f.apply(Operation::Receipt {
        room: "room".into(),
        agent: "parent".into(),
        sequence: urgent.sequence,
        state: "accepted".into(),
    });
    assert_eq!(
        f.store
            .agent_delivery_front("room", "parent")
            .unwrap()
            .unwrap()
            .sequence,
        normal.sequence
    );
    assert_eq!(
        f.store
            .agent_urgent_delivery_front("room", "parent")
            .unwrap()
            .unwrap()
            .sequence,
        later.sequence
    );
}
