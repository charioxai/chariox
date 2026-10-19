use super::*;
pub(super) fn test_wake(id: &str, task: &str, agent: &str) -> AgentWake {
    AgentWake {
        id: id.into(),
        task_id: task.into(),
        room_id: "room".into(),
        agent_id: agent.into(),
        registration_id: format!("completion-{id}"),
        kind: "test".into(),
        label: "test source".into(),
        state: "starting".into(),
        created_at_ms: 1,
        verified_at_ms: None,
        next_due_ms: None,
        interval_ms: None,
        summary: "test source".into(),
        detail: serde_json::Value::Null,
        fire_count: 0,
        missed_fires: 0,
        last_fired_at_ms: None,
        last_sequence: None,
        last_delivery: None,
        last_delivered_at_ms: None,
        last_acknowledged_at_ms: None,
        alerted_sequence: None,
    }
}
pub(super) struct Fixture {
    pub(super) root: std::path::PathBuf,
    pub(super) store: DurableKernelStateStore,
}
impl Fixture {
    pub(super) fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("chariox-am2-tests-{:032x}", rand::random::<u128>()));
        std::fs::create_dir(&root).unwrap();
        let store = DurableKernelStateStore::open_owned(root.join("state.sqlite")).unwrap();
        Self { root, store }
    }
    pub(super) fn apply(&self, op: Operation) -> Outcome {
        self.store.agent_lifecycle(op).unwrap()
    }
    pub(super) fn begin(&self, prompt: &str) {
        self.apply(Operation::Begin {
            owner: "owner".into(),
            room: "room".into(),
            agent: "parent".into(),
            prompt: prompt.into(),
            run: Some("run".into()),
            now: 1,
        });
    }
    pub(super) fn task(&self) -> AgentTaskExecution {
        self.store
            .agent_tasks(Some("room"), Some("parent"))
            .unwrap()
            .remove(0)
    }
    /// Arms a driver-confirmed source `child` (obligation `child`,
    /// registration `completion-child`) and confirms it live.
    pub(super) fn register(&self) {
        self.arm("p", "child");
    }
    pub(super) fn arm(&self, task: &str, id: &str) {
        self.apply(Operation::ArmWake {
            task: task.into(),
            prompt: task.into(),
            wake: test_wake(id, task, "parent"),
        });
        self.apply(Operation::WakeVerified {
            id: id.into(),
            now: 1,
        });
    }
    fn yield_now(&self) {
        self.apply(Operation::Yield {
            task: "p".into(),
            prompt: "p".into(),
            registrations: vec!["completion-child".into()],
            cursor: 0,
            deadline: 60_000,
            reason: "child result".into(),
            now: 2,
        });
    }
    pub(super) fn settle(&self, prompt: &str) -> Outcome {
        self.apply(Operation::Settle {
            room: "room".into(),
            agent: "parent".into(),
            prompt: prompt.into(),
            run: "run".into(),
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
                f.settle("p");
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
    let Outcome::Settled { task, correction } = f.settle("p") else {
        panic!()
    };
    assert!(correction);
    assert!(task.correction_used);
    assert_ne!(task.state, ExecutionState::Done);
    let id = task.pending_prompt_id.unwrap();
    f.begin(&id);
    let Outcome::Settled { task, correction } = f.settle(&id) else {
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
    f.yield_now();
    assert_eq!(f.task().state, ExecutionState::Working);
    f.apply(Operation::SourceOutcome {
        now: 6,
        public_answer: None,
        room: "room".into(),
        source: "child".into(),
        occurrence: "result".into(),
        success: true,
    });
    let Outcome::Settled { task, correction } = f.settle("p") else {
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
    f.yield_now();
    f.settle("p");
    assert_eq!(f.task().state, ExecutionState::Waiting);
    f.apply(Operation::Sweep {
        now: LONG_WAIT_MS + 1,
        busy_recipients: Vec::new(),
    });
    f.apply(Operation::Sweep {
        now: LONG_WAIT_MS + 2,
        busy_recipients: Vec::new(),
    });
    assert!(f.task().wait.unwrap().long_wait_notified);
    assert_eq!(f.store.agent_inbox("room", "parent", 0).unwrap().len(), 1);
}
#[test]
fn a02_plain_turn_without_obligation_or_wait_ends_done_without_correction() {
    let f = Fixture::new();
    f.begin("p");
    let Outcome::Settled { task, correction } = f.settle("p") else {
        panic!()
    };
    assert!(!correction, "a plain turn is never corrected");
    assert_eq!(task.state, ExecutionState::Done);
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
    assert!(f
        .store
        .agent_lifecycle(Operation::Yield {
            task: "p".into(),
            prompt: "p".into(),
            registrations: vec!["completion-child".into()],
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
        })
        .is_err());
    f.apply(Operation::Sweep {
        now: DELIVERY_TIMEOUT_MS + 10,
        busy_recipients: Vec::new(),
    });
    assert_eq!(f.task().state, ExecutionState::Blocked);
}
#[test]
fn a02_independent_task_never_hides_older_wait() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    f.yield_now();
    f.settle("p");
    f.begin("new");
    f.settle("new");
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
    f.yield_now();
    f.settle("p");
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
            now: 10 + n,
        });
        f.begin(&prompt);
        f.apply(Operation::Yield {
            task: "p".into(),
            prompt: prompt.clone(),
            registrations: vec!["completion-child".into()],
            cursor: e.sequence,
            deadline: 60_000,
            reason: "same child wait".into(),
            now: 20 + n,
        });
        f.settle(&prompt);
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
        });
        f.apply(Operation::Sweep {
            now: DELIVERY_TIMEOUT_MS + 1,
            busy_recipients: Vec::new(),
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
        });
        f.apply(Operation::Sweep {
            now: DELIVERY_TIMEOUT_MS + 1,
            busy_recipients: Vec::new(),
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
    f.yield_now();
    f.settle("p");
    f.apply(Operation::Sweep {
        now: 20,
        busy_recipients: Vec::new(),
    });
    f.apply(Operation::Sweep {
        now: 10,
        busy_recipients: Vec::new(),
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
    let Outcome::Settled { task, .. } = f.settle("p") else {
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
fn a02_owner_cancel_closes_sources_without_faking_physical_completion() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
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
        now: 6,
        public_answer: None,
        room: "room".into(),
        source: "child".into(),
        occurrence: "physical-cancellation".into(),
        success: false,
    });
    assert_eq!(f.task().obligations[0].status, "cancelled");
    assert_eq!(f.task().state, ExecutionState::Cancelled);
}
#[test]
fn a02_artifact_receipt_resets_guard_once_and_survives_restart() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    f.yield_now();
    f.settle("p");
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
    f.yield_now();
    f.settle("p");
    f.begin("independent");
    f.arm("independent", "child2");
    let db = Connection::open(f.root.join("state.sqlite")).unwrap();
    db.execute(
        "UPDATE agent_registrations SET payload='broken' WHERE id='completion-child'",
        [],
    )
    .unwrap();
    f.apply(Operation::SourceOutcome {
        now: 6,
        public_answer: None,
        room: "room".into(),
        source: "child".into(),
        occurrence: "terminal".into(),
        success: true,
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
        .all(|e| e.payload["task_id"] != "independent"));
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
        });
    };
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

// MP-08/MP-09/MP-10/MP-11 #914 review regressions.
fn waiting_with_deadline_wake(f: &Fixture) -> InboxEvent {
    f.begin("p");
    f.register();
    f.yield_now();
    f.settle("p");
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
fn a02_review_timed_out_delivery_blocks_only_its_own_task() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    f.yield_now();
    f.settle("p");
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
    });
    f.apply(Operation::Sweep {
        now: 10 + DELIVERY_TIMEOUT_MS,
        busy_recipients: Vec::new(),
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
    });
    assert!(f.store.agent_tasks(None, None).unwrap().is_empty());
    f.apply(Operation::Sweep {
        now: 90_010 + DELIVERY_TIMEOUT_MS,
        busy_recipients: Vec::new(),
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
                });
                assert!(f.store.agent_tasks(None, None).unwrap().is_empty());
                f.apply(Operation::Sweep {
                    now: at + DELIVERY_TIMEOUT_MS,
                    busy_recipients: Vec::new(),
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
        });
        f.apply(Operation::Sweep {
            now: 10 + DELIVERY_TIMEOUT_MS,
            busy_recipients: Vec::new(),
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
    security_fill_inbox(&f, "parent", "pending");
    f.apply(Operation::SourceOutcome {
        now: 6,
        room: "room".into(),
        source: "child".into(),
        occurrence: "completion".into(),
        success: true,
        public_answer: None,
    });
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
    f.yield_now();
    f.settle("p");
    security_fill_inbox(&f, "parent", "pending");
    f.apply(Operation::Sweep {
        now: LONG_WAIT_MS + 1,
        busy_recipients: Vec::new(),
    });
    assert!(f
        .store
        .agent_inbox("room", "parent", 1024)
        .unwrap()
        .iter()
        .any(|e| e.kind == "deadline_reached"));
}
#[test]
fn a02_g02_pre_io_failure_does_not_fabricate_uncertain_delivery() {
    let f = Fixture::new();
    let e = waiting_with_deadline_wake(&f);
    let failure = Err(error("provider run ended before enqueue"));
    let state = delivery_attempt_receipt_state(false, &failure, true);
    f.apply(Operation::Receipt {
        room: "room".into(),
        agent: "parent".into(),
        sequence: e.sequence,
        state: state.into(),
        now: 6,
    });
    assert_eq!(
        f.store.agent_inbox("room", "parent", 0).unwrap()[0].state,
        "pending"
    );
    assert_eq!(f.task().state, ExecutionState::Waiting);
    assert_eq!(f.task().no_progress_wakes, 0);
    assert_eq!(
        delivery_attempt_receipt_state(true, &failure, true),
        "uncertain",
        "an admitted I/O boundary still forbids replay"
    );
}

// MP-08 / MP-09 / MP-10 / MP-11 A12-S02: retiring an automatic wake must not discard its resource outcome.
#[test]
fn yield_without_open_obligation_points_to_the_final_answer() {
    let f = Fixture::new();
    f.begin("p");
    let error = f
        .store
        .agent_lifecycle(Operation::Yield {
            task: "p".into(),
            prompt: "p".into(),
            registrations: vec![],
            cursor: 0,
            deadline: 60_000,
            reason: "nothing".into(),
            now: 2,
        })
        .unwrap_err();
    assert!(error.to_string().contains("final answer"), "{error}");
}

// MP-08/MP-09/MP-10/MP-11 F03: an accepted finite wait cannot disappear
// because native turn settlement arrives after its deadline.
#[test]
fn a02_late_native_settlement_delivers_the_original_deadline_once() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    f.yield_now();
    let Outcome::Settled { task, correction } = f.apply(Operation::Settle {
        room: "room".into(),
        agent: "parent".into(),
        prompt: "p".into(),
        run: "run".into(),
        cancelled: false,
        now: 60_001,
    }) else {
        panic!()
    };
    assert_eq!(task.state, ExecutionState::Waiting);
    assert!(!correction);
    for now in [60_002, 60_003] {
        f.apply(Operation::Sweep {
            now,
            busy_recipients: vec![],
        });
    }
    let inbox = f.store.agent_inbox("room", "parent", 0).unwrap();
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].kind, "deadline_reached");
    assert_eq!(f.task().wait.unwrap().deadline_ms, 60_000);
}

// MP-08/MP-09/MP-10/MP-11 F03: continued native polling cannot replace a
// previously accepted deadline with a new one before native settlement.
#[test]
fn a02_accepted_wait_rejects_repeated_yield_without_extending_deadline() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    f.yield_now();
    assert!(f
        .store
        .agent_lifecycle(Operation::Yield {
            task: "p".into(),
            prompt: "p".into(),
            registrations: vec!["completion-child".into()],
            cursor: 0,
            deadline: 120_000,
            reason: "polling".into(),
            now: 3,
        })
        .is_err());
    assert_eq!(f.task().wait.unwrap().deadline_ms, 60_000);
}

// MP-08/MP-10/MP-11 F04: a delivered message remains satisfied while the
// parent observes THIS child task, including completion-before-subscribe.
