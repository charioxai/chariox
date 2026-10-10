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
fn a02_queue_dispatch_revalidates_blocked_cancelled_and_done_tasks() {
    for state in ["blocked", "cancelled", "done"] {
        let f = Fixture::new();
        f.begin("p");
        match state {
            "blocked" => {
                f.apply(Operation::Block {
                    task: "p".into(),
                    prompt: "p".into(),
                    reason: "owner must resolve missing input".into(),
                });
            }
            "cancelled" => {
                f.apply(Operation::CancelTask {
                    task: "p".into(),
                    owner: "owner".into(),
                    revision: f.task().revision,
                });
            }
            _ => {
                f.settle("p", true);
            }
        }
        let before = f.task();
        assert!(f
            .store
            .agent_lifecycle(Operation::Begin {
                owner: "owner".into(),
                room: "room".into(),
                agent: "parent".into(),
                prompt: "p".into(),
                run: Some("late-run".into()),
                now: 20,
            })
            .is_err());
        assert_eq!(f.task(), before);
        // The blocked older task cannot suppress independent authorized work.
        f.begin("new-user-prompt");
        assert!(f
            .store
            .agent_tasks(Some("room"), Some("parent"))
            .unwrap()
            .iter()
            .any(|t| t.task_id == "new-user-prompt" && t.state == ExecutionState::Working));
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
        busy_recipients: Vec::new(),
        held_work: Vec::new(),
    });
    f.apply(Operation::Sweep {
        now: LONG_WAIT_MS + 2,
        busy_recipients: Vec::new(),
        held_work: Vec::new(),
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
        work: None,
    });
    f.apply(Operation::Receipt {
        room: "room".into(),
        agent: "parent".into(),
        sequence: e.sequence,
        state: "uncertain".into(),
        now: 11,
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
            now: 12,
            work: None,
        })
        .is_err());
    f.apply(Operation::Sweep {
        now: DELIVERY_TIMEOUT_MS + 10,
        busy_recipients: Vec::new(),
        held_work: Vec::new(),
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
            work: None,
        });
        f.apply(Operation::Receipt {
            room: "room".into(),
            agent: "parent".into(),
            sequence: e.sequence,
            state: "accepted".into(),
            now: 10 + n,
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
    f.apply(Operation::Sweep {
        now: 10,
        busy_recipients: Vec::new(),
        held_work: Vec::new(),
    });
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
fn a02_cancel_independent_task_preserves_uncertain_recipient_delivery() {
    for target in [Some("p"), None] {
        let f = Fixture::new();
        f.begin("p");
        f.begin("independent");
        let Outcome::Event(e) = f.apply(Operation::Occur(occurrence(
            "room",
            "parent",
            "peer",
            "uncertain",
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
            target: target.map(str::to_owned),
            run: Some("run".into()),
            now: 1,
            work: None,
        });
        f.apply(Operation::Sweep {
            now: DELIVERY_TIMEOUT_MS + 1,
            busy_recipients: Vec::new(),
            held_work: Vec::new(),
        });
        let task = f
            .store
            .agent_tasks(Some("room"), Some("parent"))
            .unwrap()
            .into_iter()
            .find(|t| t.task_id == "independent")
            .unwrap();
        // #914 review 6: unrelated work is not blocked by another turn's receipt.
        assert_eq!(task.state, ExecutionState::Working);
        f.apply(Operation::CancelTask {
            task: task.task_id,
            owner: "owner".into(),
            revision: task.revision,
        });
        assert_eq!(
            f.store.agent_inbox("room", "parent", 0).unwrap()[0].state,
            "blocked",
            "cancelling unrelated work must not abandon another turn's receipt"
        );
        let bound_id = target
            .map(str::to_owned)
            .unwrap_or_else(|| format!("delivery-{}", e.sequence));
        let bound = f
            .store
            .agent_tasks(Some("room"), Some("parent"))
            .unwrap()
            .into_iter()
            .find(|t| t.task_id == bound_id)
            .expect("unknown delivery has its own owner interaction");
        f.apply(Operation::OwnerResponse {
            task: bound.task_id,
            revision: bound.blocked_revision,
            resume: false,
            now: DELIVERY_TIMEOUT_MS + 3,
        });
        assert_eq!(
            f.store.agent_inbox("room", "parent", 0).unwrap()[0].state,
            "failed"
        );
    }
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
            work: None,
        });
        f.apply(Operation::Sweep {
            now: DELIVERY_TIMEOUT_MS + 1,
            busy_recipients: Vec::new(),
            held_work: Vec::new(),
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
                now: DELIVERY_TIMEOUT_MS + 2,
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
        work: None,
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
    f.apply(Operation::Sweep {
        now: 20,
        busy_recipients: Vec::new(),
        held_work: Vec::new(),
    });
    f.apply(Operation::Sweep {
        now: 10,
        busy_recipients: Vec::new(),
        held_work: Vec::new(),
    });
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
    // Explicit subscriptions and the automatic completion subscription bind together.
    f.subscribe();
    for prompt in ["first-child-task", "independent-child-task"] {
        f.apply(Operation::Begin {
            owner: "owner".into(),
            room: "room".into(),
            agent: "child".into(),
            prompt: prompt.into(),
            run: Some("child-run".into()),
            now: 2,
        });
        f.apply(Operation::BindDelegate {
            parent_task: "p".into(),
            child_task: prompt.into(),
        });
    }
    assert_eq!(
        f.task().obligations[0].completion_task_id.as_deref(),
        Some("first-child-task")
    );
    let registrations = f.store.agent_registrations("p").unwrap();
    assert_eq!(registrations.len(), 2);
    assert!(registrations
        .iter()
        .all(|registration| registration.source_id == "first-child-task"));
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
        work: None,
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
        work: None,
    });
    f.apply(Operation::Receipt {
        room: "room".into(),
        agent: "parent".into(),
        sequence: event.sequence,
        state: "accepted".into(),
        now: 11,
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
        work: None,
    });
    f.apply(Operation::Receipt {
        room: "room".into(),
        agent: "parent".into(),
        sequence: urgent.sequence,
        state: "uncertain".into(),
        now: 1,
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
            work: None,
        })
        .is_err());
    f.apply(Operation::Receipt {
        room: "room".into(),
        agent: "parent".into(),
        sequence: urgent.sequence,
        state: "accepted".into(),
        now: 2,
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

#[test]
fn a02_urgent_reply_tracks_corrected_task_and_late_exact_acceptance() {
    let f = Fixture::new();
    f.begin("p");
    f.apply(Operation::Begin {
        owner: "owner".into(),
        room: "room".into(),
        agent: "child".into(),
        prompt: "q".into(),
        run: Some("child-run".into()),
        now: 1,
    });
    let mut event = occurrence(
        "room",
        "child",
        "parent",
        "request",
        "message",
        serde_json::json!({"message":"a real answer is requested"}),
    );
    event.reply_requested = true;
    event.urgent = true;
    let Outcome::Event(event) = f.apply(Operation::Send {
        task: "p".into(),
        prompt: "p".into(),
        event,
    }) else {
        panic!()
    };
    f.apply(Operation::Attempt {
        room: "room".into(),
        agent: "child".into(),
        sequence: event.sequence,
        prompt: format!("agent-event-child-{}", event.sequence),
        target: Some("q".into()),
        run: Some("child-run".into()),
        now: 2,
        work: None,
    });
    let Outcome::Settled { task, correction } = f.apply(Operation::Settle {
        room: "room".into(),
        agent: "child".into(),
        prompt: "q".into(),
        run: "child-run".into(),
        has_answer: false,
        cancelled: false,
        now: 3,
    }) else {
        panic!()
    };
    assert!(correction);
    let corrected = task.pending_prompt_id.unwrap();
    f.apply(Operation::Begin {
        owner: "owner".into(),
        room: "room".into(),
        agent: "child".into(),
        prompt: corrected.clone(),
        run: Some("child-run".into()),
        now: 4,
    });
    f.apply(Operation::Settle {
        room: "room".into(),
        agent: "child".into(),
        prompt: corrected,
        run: "child-run".into(),
        has_answer: true,
        cancelled: false,
        now: 5,
    });
    f.apply(Operation::SourceOutcome {
        room: "room".into(),
        source: "q".into(),
        occurrence: "task-terminal-q".into(),
        success: true,
        public_answer: Some(serde_json::json!({"excerpt":"verified result"})),
        now: 6,
    });
    assert!(f.store.agent_inbox("room", "parent", 0).unwrap().is_empty());
    f.begin("independent");
    f.apply(Operation::Receipt {
        room: "room".into(),
        agent: "child".into(),
        sequence: event.sequence,
        state: "accepted".into(),
        now: 6,
    });
    let reply = f.store.agent_inbox("room", "parent", 0).unwrap();
    assert_eq!(reply.len(), 1);
    assert_eq!(
        reply[0].payload["public_answer"]["excerpt"],
        "verified result"
    );
    f.apply(Operation::Ack {
        room: "room".into(),
        agent: "parent".into(),
        sequence: reply[0].sequence,
        handled: true,
        now: 7,
    });
    let tasks = f.store.agent_tasks(Some("room"), Some("parent")).unwrap();
    let sender = tasks.iter().find(|t| t.task_id == "p").unwrap();
    assert_eq!(sender.obligations[0].status, "satisfied");
    assert_eq!(sender.progress_sequence, 1);
    assert_eq!(
        tasks
            .iter()
            .find(|t| t.task_id == "independent")
            .unwrap()
            .progress_sequence,
        0
    );
    f.apply(Operation::SourceOutcome {
        room: "room".into(),
        source: "q".into(),
        occurrence: "task-terminal-q".into(),
        success: true,
        public_answer: None,
        now: 8,
    });
    assert_eq!(f.store.agent_inbox("room", "parent", 0).unwrap().len(), 1);
}

#[test]
fn a02_every_delivery_receipt_has_client_visible_text() {
    let f = Fixture::new();
    let receipt = |sequence: u64, state: &str| {
        let Outcome::Event(event) = f.apply(Operation::Receipt {
            room: "room".into(),
            agent: "parent".into(),
            sequence,
            state: state.into(),
            now: 1,
        }) else {
            panic!()
        };
        receipt_notice(&event)
    };
    let attempt = |sequence: u64, target: Option<&str>, prompt: &str| {
        f.apply(Operation::Attempt {
            room: "room".into(),
            agent: "parent".into(),
            sequence,
            prompt: prompt.into(),
            target: target.map(Into::into),
            run: Some("run".into()),
            now: 1,
            work: None,
        });
    };
    let mut steer = occurrence(
        "room",
        "parent",
        "sender",
        "steer",
        "message",
        serde_json::json!({}),
    );
    steer.urgent = true;
    let Outcome::Event(steer) = f.apply(Operation::Occur(steer)) else {
        panic!()
    };
    attempt(steer.sequence, Some("running"), "steer");
    assert_eq!(
        receipt(steer.sequence, "accepted"),
        format!(
            "Agent inbox: event {} accepted into the running turn",
            steer.sequence
        )
    );
    let Outcome::Event(queued) = f.apply(Operation::Occur(occurrence(
        "room",
        "parent",
        "sender",
        "queued",
        "message",
        serde_json::json!({}),
    ))) else {
        panic!()
    };
    attempt(queued.sequence, None, "first");
    assert!(receipt(queued.sequence, "rejected").contains("retained for the next turn"));
    attempt(queued.sequence, None, "second");
    assert!(receipt(queued.sequence, "uncertain").contains("delivery uncertain"));
    assert!(receipt(queued.sequence, "accepted").ends_with("accepted as a new turn"));
}

#[test]
fn a02_rejected_submission_withdraws_only_untouched_admission() {
    let f = Fixture::new();
    f.begin("p");
    f.apply(Operation::Withdraw { task: "p".into() });
    assert!(f
        .store
        .agent_tasks(Some("room"), Some("parent"))
        .unwrap()
        .is_empty());
    f.begin("p2");
    f.apply(Operation::RegisterObligation {
        owner: "owner".into(),
        room: "room".into(),
        agent: "parent".into(),
        prompt: "p2".into(),
        run: Some("run".into()),
        id: "obligation".into(),
        kind: "delegate".into(),
        resource: Some("child".into()),
        now: 1,
    });
    assert!(f
        .store
        .agent_lifecycle(Operation::Withdraw { task: "p2".into() })
        .is_err());
    assert_eq!(f.task().obligations.len(), 1);
}

// MP-08/MP-09/MP-10/MP-11 #914 review regressions.
fn waiting_with_deadline_wake(f: &Fixture) -> InboxEvent {
    f.begin("p");
    f.register();
    f.subscribe();
    f.yield_now();
    f.settle("p", true);
    assert_eq!(f.task().state, ExecutionState::Waiting);
    let Outcome::Event(e) = f.apply(Operation::Occur(occurrence(
        "room",
        "parent",
        "p",
        "deadline",
        "deadline_reached",
        serde_json::json!({"task_id":"p"}),
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
        now: 4,
        work: None,
    });
    assert_eq!(f.task().state, ExecutionState::Working);
    e
}

#[test]
fn a02_review_rejected_wake_returns_the_task_to_its_wait() {
    // Rejected before admission, and after admission but before provider I/O.
    for admitted in [false, true] {
        let f = Fixture::new();
        let e = waiting_with_deadline_wake(&f);
        if admitted {
            f.apply(Operation::Begin {
                owner: "owner".into(),
                room: "room".into(),
                agent: "parent".into(),
                prompt: "wake".into(),
                run: None,
                now: 5,
            });
        }
        f.apply(Operation::Receipt {
            room: "room".into(),
            agent: "parent".into(),
            sequence: e.sequence,
            state: "rejected".into(),
            now: 5,
        });
        let t = f.task();
        assert_eq!(t.state, ExecutionState::Waiting, "admitted={admitted}");
        assert!(t.wait.is_some());
        assert_eq!(t.pending_prompt_id, None);
        assert_eq!(
            t.no_progress_wakes, 0,
            "a refused wake is not a no-progress strike"
        );
    }
}

#[test]
fn a02_review_failed_turn_on_blocked_task_does_not_abort_settlement() {
    let f = Fixture::new();
    f.begin("p");
    f.apply(Operation::Block {
        task: "p".into(),
        prompt: "p".into(),
        reason: "owner must choose".into(),
    });
    let before = f.task();
    block_failed_turn(&f.store, &before, "Provider run failed".into())
        .expect("an already blocked task keeps its owner action");
    assert_eq!(f.task(), before);
}

#[test]
fn a02_review_late_rejected_dispatch_receipt_cannot_corrupt_done_task() {
    let f = Fixture::new();
    f.begin("p");
    f.apply(Operation::RegisterObligation {
        owner: "owner".into(),
        room: "room".into(),
        agent: "parent".into(),
        prompt: "p".into(),
        run: Some("run".into()),
        id: "message".into(),
        kind: "message".into(),
        resource: Some("peer".into()),
        now: 1,
    });
    let receipt = |accepted| {
        f.store.agent_lifecycle(Operation::DispatchReceipt {
            id: "message".into(),
            accepted,
            resource: Some("peer".into()),
        })
    };
    receipt(true).unwrap();
    f.settle("p", true);
    assert_eq!(f.task().state, ExecutionState::Done);
    let _ = receipt(false);
    let _ = receipt(true);
    let tasks = f.store.agent_tasks(Some("room"), Some("parent")).unwrap();
    assert_eq!(tasks.len(), 1, "no quarantine task: {tasks:?}");
    assert_eq!(tasks[0].state, ExecutionState::Done);
    assert_eq!(tasks[0].obligations[0].status, "satisfied");
}

#[test]
fn a02_review_two_parents_delegating_to_one_child_bind_distinct_child_tasks() {
    let f = Fixture::new();
    for parent in ["parent", "parent2"] {
        let prompt = format!("{parent}-turn");
        f.apply(Operation::Begin {
            owner: "owner".into(),
            room: "room".into(),
            agent: parent.into(),
            prompt: prompt.clone(),
            run: Some(format!("{parent}-run")),
            now: 1,
        });
        f.apply(Operation::RegisterObligation {
            owner: "owner".into(),
            room: "room".into(),
            agent: parent.into(),
            prompt,
            run: Some(format!("{parent}-run")),
            id: format!("{parent}-delegation"),
            kind: "delegate".into(),
            resource: Some("child".into()),
            now: 1,
        });
        f.apply(Operation::DispatchReceipt {
            id: format!("{parent}-delegation"),
            accepted: true,
            resource: Some("child".into()),
        });
    }
    for (parent, prompt) in [("parent", "child-task-1"), ("parent2", "child-task-2")] {
        f.apply(Operation::Begin {
            owner: "owner".into(),
            room: "room".into(),
            agent: "child".into(),
            prompt: prompt.into(),
            run: Some("child-run".into()),
            now: 2,
        });
        f.apply(Operation::BindDelegate {
            parent_task: format!("{parent}-turn"),
            child_task: prompt.into(),
        });
    }
    let bound = |agent: &str| {
        f.store.agent_tasks(Some("room"), Some(agent)).unwrap()[0].obligations[0]
            .completion_task_id
            .clone()
    };
    assert_eq!(bound("parent").as_deref(), Some("child-task-1"));
    assert_eq!(bound("parent2").as_deref(), Some("child-task-2"));
}

// MP-08/MP-10/MP-11 #914 round 3: task creation is not delegation order.
#[test]
fn a02_r3_reverse_parent_creation_binds_in_delegation_order() {
    let f = Fixture::new();
    for parent in ["older", "newer"] {
        f.apply(Operation::Begin {
            owner: "owner".into(),
            room: "room".into(),
            agent: parent.into(),
            prompt: format!("{parent}-turn"),
            run: Some(format!("{parent}-run")),
            now: 1,
        });
    }
    for parent in ["newer", "older"] {
        f.apply(Operation::RegisterObligation {
            owner: "owner".into(),
            room: "room".into(),
            agent: parent.into(),
            prompt: format!("{parent}-turn"),
            run: Some(format!("{parent}-run")),
            id: format!("{parent}-delegate"),
            kind: "delegate".into(),
            resource: Some("child".into()),
            now: 2,
        });
        f.apply(Operation::DispatchReceipt {
            id: format!("{parent}-delegate"),
            accepted: true,
            resource: Some("child".into()),
        });
    }
    for (parent, prompt) in [("newer", "first-child"), ("older", "second-child")] {
        // Repeat Begin just as admission and dispatch do: never consume two parents.
        for _ in 0..2 {
            f.apply(Operation::Begin {
                owner: "owner".into(),
                room: "room".into(),
                agent: "child".into(),
                prompt: prompt.into(),
                run: Some("child-run".into()),
                now: 3,
            });
            f.apply(Operation::BindDelegate {
                parent_task: format!("{parent}-turn"),
                child_task: prompt.into(),
            });
        }
    }
    for (parent, prompt) in [("newer", "first-child"), ("older", "second-child")] {
        let t = f
            .store
            .agent_tasks(Some("room"), Some(parent))
            .unwrap()
            .remove(0);
        assert_eq!(t.obligations[0].completion_task_id.as_deref(), Some(prompt));
        assert_eq!(
            f.store.agent_registrations(&t.task_id).unwrap()[0].source_id,
            prompt
        );
    }
}

// MP-08/MP-10/MP-11: rejected attempts must not renew their timeout indefinitely.
#[test]
fn a02_r3_rejected_first_message_escalates_without_task_rows() {
    let f = Fixture::new();
    let Outcome::Event(e) = f.apply(Operation::Occur(occurrence(
        "room",
        "child",
        "parent",
        "first-message",
        "message",
        serde_json::json!({"message":"work"}),
    ))) else {
        panic!()
    };
    for now in [10, 10 + SWEEP_MS, 10 + DELIVERY_TIMEOUT_MS - 1] {
        f.apply(Operation::Attempt {
            room: "room".into(),
            agent: "child".into(),
            sequence: e.sequence,
            prompt: "rejected".into(),
            target: None,
            run: None,
            now,
            work: None,
        });
        f.apply(Operation::Receipt {
            room: "room".into(),
            agent: "child".into(),
            sequence: e.sequence,
            state: "rejected".into(),
            now,
        });
        assert!(f.store.agent_tasks(None, None).unwrap().is_empty());
        let Outcome::Swept(changed) = f.apply(Operation::Sweep {
            now,
            busy_recipients: Vec::new(),
            held_work: Vec::new(),
        }) else {
            panic!()
        };
        assert!(changed.is_empty());
    }
    let Outcome::Swept(changed) = f.apply(Operation::Sweep {
        now: 10 + DELIVERY_TIMEOUT_MS,
        busy_recipients: Vec::new(),
        held_work: Vec::new(),
    }) else {
        panic!()
    };
    assert_eq!(
        changed.len(),
        1,
        "a repeatedly refused message must reach the owner"
    );
    assert_eq!(changed[0].state, ExecutionState::Blocked);
    assert_eq!(changed[0].task_id, format!("delivery-{}", e.sequence));
    assert_eq!(
        f.store
            .agent_delivery_front("room", "child")
            .unwrap()
            .unwrap()
            .state,
        "blocked"
    );
    let Outcome::Swept(changed) = f.apply(Operation::Sweep {
        now: 10 + DELIVERY_TIMEOUT_MS + SWEEP_MS,
        busy_recipients: Vec::new(),
        held_work: Vec::new(),
    }) else {
        panic!()
    };
    assert!(
        changed.is_empty(),
        "one timeout must emit one owner transition"
    );
}

#[test]
fn a02_review_timed_out_delivery_blocks_only_its_own_task() {
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
        "unrelated-message",
        "message",
        serde_json::json!({}),
    ))) else {
        panic!()
    };
    f.apply(Operation::Attempt {
        room: "room".into(),
        agent: "parent".into(),
        sequence: e.sequence,
        prompt: "message-turn".into(),
        target: None,
        run: Some("run".into()),
        now: 10,
        work: None,
    });
    f.apply(Operation::Sweep {
        now: 10 + DELIVERY_TIMEOUT_MS,
        busy_recipients: Vec::new(),
        held_work: Vec::new(),
    });
    let tasks = f.store.agent_tasks(Some("room"), Some("parent")).unwrap();
    let waiting = tasks.iter().find(|t| t.task_id == "p").unwrap();
    assert_eq!(waiting.state, ExecutionState::Waiting);
    assert!(tasks
        .iter()
        .any(|t| t.task_id == format!("delivery-{}", e.sequence)
            && t.state == ExecutionState::Blocked));
}

#[test]
fn a02_review_queued_working_task_is_not_a_missing_executor() {
    let f = Fixture::new();
    f.begin("p");
    let t = f.task();
    let late = t.last_progress_at_ms + DELIVERY_TIMEOUT_MS;
    assert!(lacks_live_executor(&t, late, false, false));
    assert!(!lacks_live_executor(&t, late, true, false));
    assert!(!lacks_live_executor(&t, late, false, true));
}

#[test]
fn a02_review_delivery_receipt_classification() {
    let busy = Err(error("target agent is busy"));
    // A non-structured adapter write is its only acceptance receipt.
    assert_eq!(delivery_receipt_state(Some(&Ok(true)), false), "accepted");
    assert_eq!(delivery_receipt_state(Some(&Ok(true)), true), "submitting");
    assert_eq!(delivery_receipt_state(Some(&Ok(false)), true), "rejected");
    assert_eq!(delivery_receipt_state(Some(&busy), true), "uncertain");
    // Nothing reached the provider: retain for a later wake, never uncertain.
    assert_eq!(delivery_receipt_state(None, true), "rejected");
}

// MP-08/MP-10/MP-11: a refusal cannot shorten a later admitted attempt.
#[test]
fn a02_r4_submitting_after_refusal_gets_full_delivery_timeout() {
    a02_r4_admission_after_refusal_gets_full_delivery_timeout("submitting");
}
#[test]
fn a02_r4_uncertain_after_refusal_gets_full_delivery_timeout() {
    a02_r4_admission_after_refusal_gets_full_delivery_timeout("uncertain");
}
fn a02_r4_admission_after_refusal_gets_full_delivery_timeout(receipt: &str) {
    let f = Fixture::new();
    let Outcome::Event(e) = f.apply(Operation::Occur(occurrence(
        "room",
        "child",
        "parent",
        "retry",
        "message",
        serde_json::json!({}),
    ))) else {
        panic!()
    };
    for now in [10, 90_010] {
        f.apply(Operation::Attempt {
            room: "room".into(),
            agent: "child".into(),
            sequence: e.sequence,
            prompt: "delivery".into(),
            target: None,
            run: None,
            now,
            work: None,
        });
        if now == 10 || receipt == "uncertain" {
            f.apply(Operation::Receipt {
                room: "room".into(),
                agent: "child".into(),
                sequence: e.sequence,
                state: if now == 10 { "rejected" } else { receipt }.into(),
                now,
            });
        }
    }
    f.apply(Operation::Sweep {
        now: 120_010,
        busy_recipients: Vec::new(),
        held_work: Vec::new(),
    });
    let event = f
        .store
        .agent_delivery_front("room", "child")
        .unwrap()
        .unwrap();
    assert_eq!(
        event.state, receipt,
        "a refusal must not consume in-flight time"
    );
    assert_eq!(event.attempted_at_ms, Some(90_010));
    f.apply(Operation::Sweep {
        now: 90_010 + DELIVERY_TIMEOUT_MS - 1,
        busy_recipients: Vec::new(),
        held_work: Vec::new(),
    });
    assert!(f.store.agent_tasks(None, None).unwrap().is_empty());
    f.apply(Operation::Sweep {
        now: 90_010 + DELIVERY_TIMEOUT_MS,
        busy_recipients: Vec::new(),
        held_work: Vec::new(),
    });
    assert_eq!(
        f.store
            .agent_delivery_front("room", "child")
            .unwrap()
            .unwrap()
            .state,
        "blocked"
    );
}

// MP-08/MP-10: late busy refusals must leave an idle delivery opportunity.
#[test]
fn a02_r5_late_steer_rejection_before_timeout_retries_idle() {
    late_steer_rejection_retries_idle(false);
}

#[test]
fn a02_r5_late_steer_rejection_after_timeout_retries_idle() {
    late_steer_rejection_retries_idle(true);
}

fn late_steer_rejection_retries_idle(timed_out: bool) {
    let f = Fixture::new();
    let attempted_at = crate::session::unix_epoch_ms() - if timed_out { 130_000 } else { 100_000 };
    let idle_at = attempted_at
        + if timed_out {
            150_000
        } else {
            DELIVERY_TIMEOUT_MS
        };
    let mut e = occurrence(
        "room",
        "child",
        "parent",
        "late-steer",
        "message",
        serde_json::json!({}),
    );
    e.urgent = true;
    let Outcome::Event(e) = f.apply(Operation::Occur(e)) else {
        panic!()
    };
    f.apply(Operation::Attempt {
        room: "room".into(),
        agent: "child".into(),
        sequence: e.sequence,
        prompt: "steer".into(),
        target: Some("ending-turn".into()),
        run: Some("run".into()),
        now: attempted_at,
        work: None,
    });
    f.apply(Operation::BindSubmission {
        room: "room".into(),
        agent: "child".into(),
        sequence: e.sequence,
        prompt: "steer".into(),
        target: Some("ending-turn".into()),
        run: "run".into(),
        submit_epoch: 7,
        now: attempted_at,
    });
    if timed_out {
        f.apply(Operation::Sweep {
            now: attempted_at + DELIVERY_TIMEOUT_MS,
            busy_recipients: Vec::new(),
            held_work: Vec::new(),
        });
        assert_eq!(
            f.store
                .agent_delivery_front("room", "child")
                .unwrap()
                .unwrap()
                .state,
            "blocked"
        );
    }
    // Exercise the real structured-submit reconciliation seam, including its
    // exact run/epoch binding and the already-blocked receipt path.
    let finished = crate::provider::FinishedProviderPromptSubmitJob {
        session_id: "room".into(),
        agent_id: "child".into(),
        prompt_id: "steer".into(),
        provider_run_id: "run".into(),
        result: Err(crate::error::DaemonError::ProviderPromptSteerRejected {
            source: Box::new(error("target turn ended")),
        }),
        settlement_retry_attempt: 0,
    };
    let receipt = finish_provider_event_submit(&f.store, 7, &finished)
        .unwrap()
        .unwrap();
    assert!(
        f.store
            .agent_tasks(None, None)
            .unwrap()
            .iter()
            .all(|t| t.state != ExecutionState::Blocked),
        "an exact late receipt must close the synthetic delivery task"
    );
    assert!(receipt.steered);
    assert!(receipt
        .notice
        .unwrap()
        .contains("retained for the next turn"));
    let Outcome::Swept(changed) = f.apply(Operation::Sweep {
        now: idle_at,
        busy_recipients: Vec::new(),
        held_work: Vec::new(),
    }) else {
        panic!()
    };
    assert!(
        changed.is_empty(),
        "a busy refusal cannot consume the idle retry"
    );
    assert_eq!(
        f.store
            .agent_delivery_front("room", "child")
            .unwrap()
            .unwrap()
            .state,
        "pending"
    );
    f.apply(Operation::Attempt {
        room: "room".into(),
        agent: "child".into(),
        sequence: e.sequence,
        prompt: "idle-retry".into(),
        target: None,
        run: Some("next-run".into()),
        now: idle_at,
        work: None,
    });
    f.apply(Operation::Receipt {
        room: "room".into(),
        agent: "child".into(),
        sequence: e.sequence,
        state: "accepted".into(),
        now: idle_at,
    });
    assert_eq!(
        f.store.agent_inbox("room", "child", 0).unwrap()[0].state,
        "accepted"
    );
}

#[test]
fn a02_r5_idle_refusal_clock_starts_at_receipt() {
    let f = Fixture::new();
    let Outcome::Event(e) = f.apply(Operation::Occur(occurrence(
        "room",
        "child",
        "parent",
        "idle-refusal",
        "message",
        serde_json::json!({}),
    ))) else {
        panic!()
    };
    let now = crate::session::unix_epoch_ms();
    f.apply(Operation::Attempt {
        room: "room".into(),
        agent: "child".into(),
        sequence: e.sequence,
        prompt: "idle".into(),
        target: None,
        run: None,
        now: now - 100_000,
        work: None,
    });
    f.apply(Operation::Receipt {
        room: "room".into(),
        agent: "child".into(),
        sequence: e.sequence,
        state: "rejected".into(),
        now: crate::session::unix_epoch_ms(),
    });
    // Only the refusal budget is under test, rather than the attempt window.
    let db = Connection::open(f.root.join("state.sqlite")).unwrap();
    let at: i64 = db
        .query_row(
            "SELECT first_refused_at_ms FROM agent_inbox_refusals WHERE sequence=?1",
            [sql_integer(e.sequence).unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        at as u64 >= now,
        "receipt time must start the idle-refusal clock"
    );
    f.apply(Operation::Sweep {
        now: now + 30_000,
        busy_recipients: Vec::new(),
        held_work: Vec::new(),
    });
    assert_eq!(
        f.store
            .agent_delivery_front("room", "child")
            .unwrap()
            .unwrap()
            .state,
        "pending"
    );
    f.apply(Operation::Sweep {
        now: at as u64 + DELIVERY_TIMEOUT_MS,
        busy_recipients: Vec::new(),
        held_work: Vec::new(),
    });
    assert_eq!(
        f.store
            .agent_delivery_front("room", "child")
            .unwrap()
            .unwrap()
            .state,
        "blocked"
    );
}

// MP-08/MP-10: isolate damaged clocks while supervising other Rooms normally.
#[test]
fn a02_r5_text_refusal_clock_is_quarantined_without_aborting_sweep() {
    damaged_refusal_clock_is_quarantined(rusqlite::types::Value::Text("damaged".into()));
}

#[test]
fn a02_r5_negative_refusal_clock_is_quarantined_without_aborting_sweep() {
    damaged_refusal_clock_is_quarantined(rusqlite::types::Value::Integer(-1));
}

fn damaged_refusal_clock_is_quarantined(clock: rusqlite::types::Value) {
    let f = Fixture::new();
    let mut sequences = Vec::new();
    for room in ["damaged-room", "healthy-room"] {
        let Outcome::Event(e) = f.apply(Operation::Occur(occurrence(
            room,
            "child",
            "parent",
            "refused",
            "message",
            serde_json::json!({}),
        ))) else {
            panic!()
        };
        f.apply(Operation::Attempt {
            room: room.into(),
            agent: "child".into(),
            sequence: e.sequence,
            prompt: room.into(),
            target: None,
            run: None,
            now: 10,
            work: None,
        });
        f.apply(Operation::Receipt {
            room: room.into(),
            agent: "child".into(),
            sequence: e.sequence,
            state: "rejected".into(),
            now: 10,
        });
        sequences.push(e.sequence);
    }
    let db = Connection::open(f.root.join("state.sqlite")).unwrap();
    db.execute(
        "UPDATE agent_inbox_refusals SET first_refused_at_ms=?2 WHERE sequence=?1",
        params![sql_integer(sequences[0]).unwrap(), clock],
    )
    .unwrap();
    let expected_raw: String = db.query_row(
        "SELECT typeof(first_refused_at_ms) || ':' || hex(CAST(first_refused_at_ms AS BLOB)) FROM agent_inbox_refusals WHERE sequence=?1",
        [sql_integer(sequences[0]).unwrap()], |r| r.get(0),
    ).unwrap();
    let Outcome::Swept(changed) = f.apply(Operation::Sweep {
        now: 120_010,
        busy_recipients: Vec::new(),
        held_work: Vec::new(),
    }) else {
        panic!()
    };
    assert_eq!(
        changed.len(),
        2,
        "one damaged row must not stop another Room's owner transition"
    );
    let damaged = changed
        .iter()
        .find(|t| t.room_id == "damaged-room")
        .unwrap();
    assert_eq!(damaged.state, ExecutionState::Blocked);
    assert!(
        damaged.reason.contains("quarantined"),
        "owner must see the damaged clock"
    );
    assert_eq!(
        f.store
            .agent_delivery_front("healthy-room", "child")
            .unwrap()
            .unwrap()
            .state,
        "blocked"
    );
    let retained: String = db
        .query_row(
            "SELECT payload FROM agent_lifecycle_quarantine WHERE kind='refusal-clock' AND id=?1",
            [sequences[0].to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        retained, expected_raw,
        "retain the damaged clock for owner reconciliation"
    );
    let remaining: i64 = db
        .query_row("SELECT count(*) FROM agent_inbox_refusals", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(remaining, 0);
    let Outcome::Swept(changed) = f.apply(Operation::Sweep {
        now: 150_010,
        busy_recipients: Vec::new(),
        held_work: Vec::new(),
    }) else {
        panic!()
    };
    assert!(
        changed.is_empty(),
        "quarantine must project one owner transition"
    );
}

// MP-08/MP-10/MP-11: pre-work can receive a refusal after Sweep captured now.
#[test]
fn a02_r6_delivery_clocks_tolerate_sweep_receipt_race() {
    for receipt in ["rejected", "uncertain", "submitting"] {
        for ahead in [1, DELIVERY_TIMEOUT_MS, DELIVERY_TIMEOUT_MS + 1] {
            let f = Fixture::new();
            let now = 10;
            let at = now + ahead;
            let Outcome::Event(e) = f.apply(Operation::Occur(occurrence(
                "room",
                "child",
                "parent",
                "sweep-race",
                "message",
                serde_json::json!({}),
            ))) else {
                panic!()
            };
            f.apply(Operation::Attempt {
                room: "room".into(),
                agent: "child".into(),
                sequence: e.sequence,
                prompt: "delivery".into(),
                target: None,
                run: None,
                now: if receipt == "rejected" { now } else { at },
                work: None,
            });
            if receipt != "submitting" {
                f.apply(Operation::Receipt {
                    room: "room".into(),
                    agent: "child".into(),
                    sequence: e.sequence,
                    state: receipt.into(),
                    now: at,
                });
            }
            f.apply(Operation::Sweep {
                now,
                busy_recipients: Vec::new(),
                held_work: Vec::new(),
            });
            let expected = if ahead > DELIVERY_TIMEOUT_MS {
                "blocked"
            } else if receipt == "rejected" {
                "pending"
            } else {
                receipt
            };
            assert_eq!(
                f.store
                    .agent_delivery_front("room", "child")
                    .unwrap()
                    .unwrap()
                    .state,
                expected,
                "receipt={receipt}, ahead={ahead}"
            );
            if ahead <= DELIVERY_TIMEOUT_MS {
                assert!(f.store.agent_tasks(None, None).unwrap().is_empty());
                f.apply(Operation::Sweep {
                    now: at + DELIVERY_TIMEOUT_MS - 1,
                    busy_recipients: Vec::new(),
                    held_work: Vec::new(),
                });
                assert!(f.store.agent_tasks(None, None).unwrap().is_empty());
                f.apply(Operation::Sweep {
                    now: at + DELIVERY_TIMEOUT_MS,
                    busy_recipients: Vec::new(),
                    held_work: Vec::new(),
                });
                assert_eq!(
                    f.store
                        .agent_delivery_front("room", "child")
                        .unwrap()
                        .unwrap()
                        .state,
                    "blocked"
                );
            }
        }
    }
}

// MP-08/MP-10/MP-11: every exact receipt retracts the synthetic timeout decision.
#[test]
fn a02_r6_late_receipts_close_only_synthetic_delivery_tasks() {
    for receipt in ["accepted", "uncertain", "rejected"] {
        let f = Fixture::new();
        f.begin("real-task");
        f.apply(Operation::Block {
            task: "real-task".into(),
            prompt: "real-task".into(),
            reason: "unrelated owner action".into(),
        });
        let Outcome::Event(e) = f.apply(Operation::Occur(occurrence(
            "room",
            "parent",
            "peer",
            "late-receipt",
            "message",
            serde_json::json!({}),
        ))) else {
            panic!()
        };
        f.apply(Operation::Attempt {
            room: "room".into(),
            agent: "parent".into(),
            sequence: e.sequence,
            prompt: "delivery".into(),
            target: None,
            run: None,
            now: 10,
            work: None,
        });
        f.apply(Operation::Sweep {
            now: 10 + DELIVERY_TIMEOUT_MS,
            busy_recipients: Vec::new(),
            held_work: Vec::new(),
        });
        let synthetic = format!("delivery-{}", e.sequence);
        assert_eq!(
            f.store
                .agent_tasks(None, None)
                .unwrap()
                .iter()
                .find(|t| t.task_id == synthetic)
                .unwrap()
                .state,
            ExecutionState::Blocked
        );
        f.apply(Operation::Receipt {
            room: "room".into(),
            agent: "parent".into(),
            sequence: e.sequence,
            state: receipt.into(),
            now: 10 + DELIVERY_TIMEOUT_MS,
        });
        let tasks = f.store.agent_tasks(None, None).unwrap();
        assert_eq!(
            tasks.iter().find(|t| t.task_id == synthetic).unwrap().state,
            ExecutionState::Done
        );
        assert_eq!(
            tasks
                .iter()
                .find(|t| t.task_id == "real-task")
                .unwrap()
                .state,
            ExecutionState::Blocked
        );
    }
}

// MP-08/MP-10/MP-11 security F1: admission pressure is recipient-local.
fn security_fill_inbox(f: &Fixture, agent: &str, state: &str) {
    let mut db = Connection::open(f.root.join("state.sqlite")).unwrap();
    let tx = db.transaction().unwrap();
    for n in 0..1024 {
        let mut e = occurrence(
            "room",
            agent,
            "sender",
            &format!("fill-{n}"),
            "message",
            serde_json::json!({}),
        );
        tx.execute("INSERT INTO agent_inbox(room_id,agent_id,source_id,occurrence_id,payload) VALUES(?1,?2,?3,?4,'{}')", params![e.room_id,e.agent_id,e.source_id,e.occurrence_id]).unwrap();
        e.sequence = tx.last_insert_rowid() as u64;
        e.state = state.into();
        tx.execute(
            "UPDATE agent_inbox SET payload=?2 WHERE sequence=?1",
            params![sql_integer(e.sequence).unwrap(), encode(&e).unwrap()],
        )
        .unwrap();
    }
    tx.commit().unwrap();
}
#[test]
fn a02_security_f1_full_recipient_cannot_abort_source_outcome() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    f.subscribe();
    f.apply(Operation::Begin {
        owner: "owner".into(),
        room: "room".into(),
        agent: "healthy".into(),
        prompt: "healthy-task".into(),
        run: None,
        now: 1,
    });
    f.apply(Operation::Subscribe {
        task: "healthy-task".into(),
        prompt: "healthy-task".into(),
        registration: Registration {
            id: "healthy-reg".into(),
            task_id: "healthy-task".into(),
            source_id: "child".into(),
            obligation_id: None,
            source_cursor: 0,
            live: true,
        },
    });
    security_fill_inbox(&f, "parent", "pending");
    f.apply(Operation::SourceOutcome {
        room: "room".into(),
        source: "child".into(),
        occurrence: "completion".into(),
        success: true,
        public_answer: None,
        now: 2,
    });
    assert_eq!(f.store.agent_inbox("room", "healthy", 0).unwrap().len(), 1);
    assert!(f
        .store
        .agent_inbox("room", "parent", 1024)
        .unwrap()
        .iter()
        .any(|e| e.kind == "source_completed"));
    assert!(!f.store.agent_registrations("p").unwrap()[0].live);
}
#[test]
fn a02_security_f1_full_recipient_cannot_abort_deadline_sweep() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    f.subscribe();
    f.yield_now();
    f.settle("p", true);
    security_fill_inbox(&f, "parent", "pending");
    f.apply(Operation::Sweep {
        now: LONG_WAIT_MS + 1,
        busy_recipients: Vec::new(),
        held_work: Vec::new(),
    });
    assert!(f
        .store
        .agent_inbox("room", "parent", 1024)
        .unwrap()
        .iter()
        .any(|e| e.kind == "deadline_reached"));
}
#[test]
fn a02_security_f1_accepted_history_does_not_exhaust_admission() {
    let f = Fixture::new();
    security_fill_inbox(&f, "parent", "accepted");
    f.apply(Operation::Occur(occurrence(
        "room",
        "parent",
        "sender",
        "fresh",
        "message",
        serde_json::json!({}),
    )));
    let db = Connection::open(f.root.join("state.sqlite")).unwrap();
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM agent_inbox WHERE json_extract(payload,'$.state')='accepted'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(count <= 256, "accepted history retained {count} rows");
}
#[test]
fn a02_security_f1_done_task_closes_registrations() {
    let f = Fixture::new();
    f.begin("p");
    f.apply(Operation::Subscribe {
        task: "p".into(),
        prompt: "p".into(),
        registration: Registration {
            id: "passive".into(),
            task_id: "p".into(),
            source_id: "peer".into(),
            obligation_id: None,
            source_cursor: 0,
            live: true,
        },
    });
    f.settle("p", true);
    assert_eq!(f.task().state, ExecutionState::Done);
    assert!(!f.store.agent_registrations("p").unwrap()[0].live);
}
#[test]
fn a02_security_f2_reused_child_id_cannot_recover_old_answer() {
    let f = Fixture::new();
    f.apply(Operation::SourceOutcome {
        room: "room".into(),
        source: "child".into(),
        occurrence: "deleted-child-result".into(),
        success: true,
        public_answer: Some(serde_json::json!("old answer")),
        now: 1,
    });
    f.begin("p");
    f.register();
    f.apply(Operation::DispatchReceipt {
        id: "obligation".into(),
        accepted: true,
        resource: Some("child".into()),
    });
    assert_eq!(f.task().obligations[0].status, "open");
    assert!(f.store.agent_inbox("room", "parent", 0).unwrap().is_empty());
}
#[test]
fn a02_security_f4_unattributed_child_task_cannot_bind_delegation() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    f.apply(Operation::DispatchReceipt {
        id: "obligation".into(),
        accepted: true,
        resource: Some("child".into()),
    });
    f.apply(Operation::Begin {
        owner: "owner".into(),
        room: "room".into(),
        agent: "child".into(),
        prompt: "peer-request".into(),
        run: Some("child-run".into()),
        now: 2,
    });
    assert!(f.task().obligations[0].completion_task_id.is_none());
    f.apply(Operation::SourceOutcome {
        room: "room".into(),
        source: "peer-request".into(),
        occurrence: "peer-answer".into(),
        success: true,
        public_answer: Some(serde_json::json!("peer-controlled")),
        now: 3,
    });
    assert_eq!(f.task().obligations[0].status, "open");
    assert!(f.store.agent_inbox("room", "parent", 0).unwrap().is_empty());
}

// MP-08/MP-10/MP-11 F4: exact accepted task recovery covers a fast child.
#[test]
fn a02_security_f4_parent_task_binds_after_peer_task_and_fast_completion() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    f.apply(Operation::DispatchReceipt {
        id: "obligation".into(),
        accepted: true,
        resource: Some("child".into()),
    });
    for prompt in ["peer-first", "parent-request"] {
        f.apply(Operation::Begin {
            owner: "owner".into(),
            room: "room".into(),
            agent: "child".into(),
            prompt: prompt.into(),
            run: None,
            now: 2,
        });
        f.apply(Operation::SourceOutcome {
            room: "room".into(),
            source: prompt.into(),
            occurrence: format!("{prompt}-result"),
            success: true,
            public_answer: Some(serde_json::json!(prompt)),
            now: 3,
        });
        f.apply(Operation::SourceOutcome {
            room: "room".into(),
            source: "child".into(),
            occurrence: format!("{prompt}-agent-result"),
            success: true,
            public_answer: Some(serde_json::json!(prompt)),
            now: 3,
        });
    }
    assert_eq!(f.task().obligations[0].status, "open");
    f.apply(Operation::BindDelegate {
        parent_task: "p".into(),
        child_task: "parent-request".into(),
    });
    assert_eq!(
        f.task().obligations[0].completion_task_id.as_deref(),
        Some("parent-request")
    );
    let inbox = f.store.agent_inbox("room", "parent", 0).unwrap();
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].payload["public_answer"], "parent-request");
}
#[test]
fn a02_security_f4_accepted_parent_message_binds_exact_task() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    f.apply(Operation::DispatchReceipt {
        id: "obligation".into(),
        accepted: true,
        resource: Some("child".into()),
    });
    let Outcome::Event(e) = f.apply(Operation::Send {
        task: "p".into(),
        prompt: "p".into(),
        event: occurrence(
            "room",
            "child",
            "parent",
            "work-message",
            "message",
            serde_json::json!({"message":"do the delegated work"}),
        ),
    }) else {
        panic!()
    };
    f.apply(Operation::Attempt {
        room: "room".into(),
        agent: "child".into(),
        sequence: e.sequence,
        prompt: "child-message-task".into(),
        target: None,
        run: None,
        now: 2,
        work: None,
    });
    f.apply(Operation::Begin {
        owner: "owner".into(),
        room: "room".into(),
        agent: "child".into(),
        prompt: "child-message-task".into(),
        run: None,
        now: 2,
    });
    assert!(f.task().obligations[0].completion_task_id.is_none());
    f.apply(Operation::Receipt {
        room: "room".into(),
        agent: "child".into(),
        sequence: e.sequence,
        state: "accepted".into(),
        now: 3,
    });
    assert_eq!(
        f.task().obligations[0].completion_task_id.as_deref(),
        Some("child-message-task")
    );
}

// MP-08/MP-10/MP-11 F4: cancellation after admission cannot reject an accepted prompt.
#[test]
fn a02_security_f4_cancelled_parent_does_not_bind_or_reject_accepted_child() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    f.apply(Operation::DispatchReceipt {
        id: "obligation".into(),
        accepted: true,
        resource: Some("child".into()),
    });
    f.apply(Operation::Begin {
        owner: "owner".into(),
        room: "room".into(),
        agent: "child".into(),
        prompt: "child-task".into(),
        run: None,
        now: 2,
    });
    f.apply(Operation::CancelTask {
        task: "p".into(),
        owner: "owner".into(),
        revision: f.task().revision,
    });
    f.apply(Operation::BindDelegate {
        parent_task: "p".into(),
        child_task: "child-task".into(),
    });
    assert_eq!(f.task().state, ExecutionState::Cancelled);
    assert!(f.task().obligations[0].completion_task_id.is_none());
}

#[test]
fn a02_security_g11_workflow_run_receipt_reconciles_fast_and_late_completion() {
    for early in [false, true] {
        let f = Fixture::new();
        f.begin("p");
        f.apply(Operation::RegisterObligation {
            owner: "owner".into(),
            room: "room".into(),
            agent: "parent".into(),
            prompt: "p".into(),
            run: Some("run".into()),
            id: "workflow-obligation".into(),
            kind: "workflow_run".into(),
            resource: Some("workflow-ref".into()),
            now: 1,
        });
        let completed = || Operation::SourceOutcome {
            public_answer: None,
            room: "room".into(),
            source: "completed-run".into(),
            occurrence: "terminal-run".into(),
            success: true,
            now: 2,
        };
        if early {
            f.apply(completed());
        }
        f.apply(Operation::DispatchReceipt {
            id: "workflow-obligation".into(),
            accepted: true,
            resource: Some("completed-run".into()),
        });
        let regs = f.store.agent_registrations("p").unwrap();
        assert_eq!(
            regs.len(),
            1,
            "MP-08 / MP-10 / MP-11 G11: actual workflow_run kind needs completion registration"
        );
        assert_eq!(regs[0].source_id, "completed-run");
        if !early {
            f.apply(completed());
        }
        assert_eq!(f.task().obligations[0].status, "settling");
        let event = f
            .store
            .agent_inbox("room", "parent", 0)
            .unwrap()
            .pop()
            .unwrap();
        f.apply(Operation::Ack {
            room: "room".into(),
            agent: "parent".into(),
            sequence: event.sequence,
            handled: true,
            now: 3,
        });
        f.settle("p", true);
        assert_eq!(f.task().state, ExecutionState::Done);
        assert!(f.task().obligations.iter().all(|o| o.status == "satisfied"));
    }
}

// MP-08/MP-10/MP-11 R947-1: transport attempt IDs must not strand a reply.
#[test]
fn a02_requested_reply_survives_deferred_and_rejected_idle_delivery() {
    for rejected in [false, true] {
        let f = Fixture::new();
        f.begin("p");
        let mut request = occurrence(
            "room",
            "child",
            "parent",
            "request",
            "message",
            serde_json::json!({"message": "return the verified result"}),
        );
        request.reply_requested = true;
        let Outcome::Event(event) = f.apply(Operation::Send {
            task: "p".into(),
            prompt: "p".into(),
            event: request,
        }) else {
            panic!()
        };
        let logical = format!("agent-event-child-{}", event.sequence);
        if rejected {
            f.apply(Operation::Attempt {
                room: "room".into(),
                agent: "child".into(),
                sequence: event.sequence,
                prompt: logical.clone(),
                target: None,
                run: None,
                now: 2,
                work: None,
            });
            f.apply(Operation::Receipt {
                room: "room".into(),
                agent: "child".into(),
                sequence: event.sequence,
                state: "rejected".into(),
                now: 3,
            });
        } else {
            // Sudo defers an unrelated requested reply before its first admission.
            f.apply(Operation::Defer {
                room: "room".into(),
                agent: "child".into(),
                sequence: event.sequence,
                now: 2,
            });
        }
        let attempt = format!("{logical}-2");
        f.apply(Operation::Attempt {
            room: "room".into(),
            agent: "child".into(),
            sequence: event.sequence,
            prompt: attempt.clone(),
            target: None,
            run: None,
            now: 4,
            work: None,
        });
        f.apply(Operation::Begin {
            owner: "owner".into(),
            room: "room".into(),
            agent: "child".into(),
            prompt: attempt.clone(),
            run: Some("child-run".into()),
            now: 5,
        });
        f.apply(Operation::Settle {
            room: "room".into(),
            agent: "child".into(),
            prompt: attempt.clone(),
            run: "child-run".into(),
            has_answer: true,
            cancelled: false,
            now: 6,
        });
        f.apply(Operation::SourceOutcome {
            room: "room".into(),
            source: attempt.clone(),
            occurrence: format!("task-terminal-{attempt}"),
            success: true,
            public_answer: Some(serde_json::json!({"excerpt": "verified retry result"})),
            now: 7,
        });
        // Even an exact terminal outcome must wait for provider acceptance.
        assert!(f.store.agent_inbox("room", "parent", 0).unwrap().is_empty());
        f.apply(Operation::Receipt {
            room: "room".into(),
            agent: "child".into(),
            sequence: event.sequence,
            state: "accepted".into(),
            now: 8,
        });
        let replies = f.store.agent_inbox("room", "parent", 0).unwrap();
        assert_eq!(
            replies.len(),
            1,
            "requested reply lost after rejected={rejected}"
        );
        assert_eq!(replies[0].source_id, logical);
        assert_eq!(
            replies[0].payload["public_answer"]["excerpt"],
            "verified retry result"
        );
        f.apply(Operation::Ack {
            room: "room".into(),
            agent: "parent".into(),
            sequence: replies[0].sequence,
            handled: true,
            now: 9,
        });
        assert_eq!(f.task().obligations[0].status, "satisfied");
        f.apply(Operation::SourceOutcome {
            room: "room".into(),
            source: attempt.clone(),
            occurrence: format!("task-terminal-{attempt}"),
            success: true,
            public_answer: None,
            now: 10,
        });
        assert_eq!(f.store.agent_inbox("room", "parent", 0).unwrap().len(), 1);
    }
}
