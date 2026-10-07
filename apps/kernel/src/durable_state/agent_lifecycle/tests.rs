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
        [e.sequence],
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
