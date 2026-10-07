//! MP-08 / MP-09 / MP-10 / MP-11 A03: durable timer/process wake transactions.
use super::*;
struct Fixture {
    root: std::path::PathBuf,
    store: DurableKernelStateStore,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("chariox-am3-tests-{:032x}", rand::random::<u128>()));
        std::fs::create_dir(&root).unwrap();
        let store = DurableKernelStateStore::open_owned(root.join("state.sqlite")).unwrap();
        let f = Self { root, store };
        f.apply(Operation::Begin {
            owner: "owner".into(),
            room: "room".into(),
            agent: "agent".into(),
            prompt: "p".into(),
            run: Some("run".into()),
            now: 1,
        });
        f
    }
    fn apply(&self, op: Operation) -> Outcome {
        self.store.agent_lifecycle(op).unwrap()
    }
    fn wakes(&self, op: Operation) -> Vec<AgentWake> {
        match self.apply(op) {
            Outcome::Wakes(w) => w,
            other => panic!("unexpected {other:?}"),
        }
    }
    fn task(&self) -> AgentTaskExecution {
        self.store
            .agent_tasks(Some("room"), Some("agent"))
            .unwrap()
            .remove(0)
    }
    fn wake(&self, id: &str) -> AgentWake {
        self.store
            .agent_wakes(Some("room"), Some("agent"))
            .unwrap()
            .into_iter()
            .find(|w| w.id == id)
            .unwrap()
    }
    fn create(&self, id: &str, kind: &str, due: Option<u64>, interval: Option<u64>) -> AgentWake {
        self.wakes(Operation::CreateWake {
            task: "p".into(),
            prompt: "p".into(),
            wake: AgentWake {
                id: id.into(),
                task_id: "p".into(),
                room_id: "room".into(),
                agent_id: "agent".into(),
                registration_id: format!("completion-{id}"),
                kind: kind.into(),
                label: format!("{kind} {id}"),
                state: String::new(),
                created_at_ms: 10,
                verified_at_ms: Some(10),
                next_due_ms: due,
                interval_ms: interval,
                command: vec![],
                match_text: Some("ready".into()),
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
        })
        .remove(0)
    }
    fn wait_on(&self, ids: &[&str]) {
        self.apply(Operation::Yield {
            task: "p".into(),
            prompt: "p".into(),
            registrations: ids.iter().map(|id| format!("completion-{id}")).collect(),
            cursor: 0,
            deadline: 10_000_000,
            reason: "wake".into(),
            now: 20,
        });
        self.apply(Operation::Settle {
            room: "room".into(),
            agent: "agent".into(),
            prompt: "p".into(),
            run: "run".into(),
            has_answer: true,
            cancelled: false,
            now: 30,
        });
        assert_eq!(self.task().state, ExecutionState::Waiting);
    }
    fn inbox(&self) -> Vec<InboxEvent> {
        self.store.agent_inbox("room", "agent", 0).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn a03_interval_keeps_receipts_when_an_earlier_fire_is_acknowledged_late() {
    let f = Fixture::new();
    f.create("interval", "timer", Some(1_000), Some(60_000));
    f.wait_on(&["interval"]);
    f.apply(Operation::FireWakes { now: 1_000 });
    let first = f.inbox()[0].sequence;
    f.apply(Operation::FireWakes { now: 61_000 });
    f.apply(Operation::Ack {
        room: "room".into(),
        agent: "agent".into(),
        sequence: first,
        handled: true,
        now: 62_000,
    });
    let db = f.store.lock_connection("test.wake.receipts").unwrap();
    let receipts: Result<(i64, bool), _> = db.query_row(
        "SELECT count(*),max(CASE WHEN sequence=?1 THEN json_extract(payload,'$.acknowledged_at_ms') IS NOT NULL ELSE 0 END) FROM agent_wake_receipts WHERE wake_id='interval'",
        [first as i64], |r| Ok((r.get(0)?, r.get(1)?)));
    assert_eq!(
        receipts.unwrap(),
        (2, true),
        "each fire retains its own receipt across later fires"
    );
    drop(db);
    assert!(
        f.wake("interval").last_acknowledged_at_ms.is_none(),
        "the earlier ACK must not acknowledge the later fire"
    );
}

#[test]
fn a03_process_cancel_waits_for_physical_exit_receipt() {
    let f = Fixture::new();
    f.create("process", "process", None, None);
    f.apply(Operation::ProcessStarted {
        id: "process".into(),
        pid: 42,
        now: 20,
    });
    f.apply(Operation::CancelWake {
        id: "process".into(),
        task: "p".into(),
        prompt: Some("p".into()),
    });
    assert_eq!(f.wake("process").state, "cancelling");
    assert_ne!(
        f.task().obligations[0].status,
        "cancelled",
        "a signal request is not physical settlement"
    );
    f.apply(Operation::ProcessExited {
        id: "process".into(),
        exit_code: Some(143),
        tail: String::new(),
        now: 30,
    });
    assert_eq!(f.wake("process").state, "cancelled");
    assert_eq!(f.task().obligations[0].status, "cancelled");
}

#[test]
fn a03_timer_is_an_obligation_that_needs_scheduler_proof_of_life() {
    let f = Fixture::new();
    let wake = f.create("t1", "timer", Some(1_000), None);
    assert_eq!(wake.state, "scheduled");
    assert_eq!(
        wake.verified_at_ms, None,
        "creation alone is never assumed live"
    );
    let task = f.task();
    assert!(task
        .obligations
        .iter()
        .any(|o| o.id == "t1" && o.kind == "timer" && o.status == "open"));
    // An armed timer keeps the task from completing until fired or cancelled.
    f.apply(Operation::Settle {
        room: "room".into(),
        agent: "agent".into(),
        prompt: "p".into(),
        run: "run".into(),
        has_answer: true,
        cancelled: false,
        now: 30,
    });
    assert_ne!(f.task().state, ExecutionState::Done);
    let verified = f.wakes(Operation::VerifyWakes { now: 40 });
    assert_eq!(verified.len(), 1);
    assert_eq!(f.wake("t1").verified_at_ms, Some(40));
    assert!(f.wakes(Operation::VerifyWakes { now: 50 }).is_empty());
}

#[test]
fn a03_one_shot_timer_fires_once_wakes_the_wait_and_records_receipts() {
    let f = Fixture::new();
    f.create("t1", "timer", Some(1_000), None);
    f.wait_on(&["t1"]);
    assert!(f.wakes(Operation::FireWakes { now: 999 }).is_empty());
    let fired = f.wakes(Operation::FireWakes { now: 1_500 });
    assert_eq!(fired.len(), 1);
    assert!(
        f.wakes(Operation::FireWakes { now: 2_000 }).is_empty(),
        "no duplicate fire"
    );
    let events = f.inbox();
    assert_eq!(events.len(), 1);
    let e = &events[0];
    assert_eq!(
        (e.kind.as_str(), e.payload["task_id"].as_str()),
        ("source_completed", Some("p"))
    );
    assert_eq!(e.payload["public_answer"]["late_ms"], 500);
    let wake = f.wake("t1");
    assert_eq!(
        (
            wake.state.as_str(),
            wake.last_fired_at_ms,
            wake.last_sequence
        ),
        ("fired", Some(1_500), Some(e.sequence))
    );
    f.apply(Operation::Attempt {
        room: "room".into(),
        agent: "agent".into(),
        sequence: e.sequence,
        prompt: "wake-turn".into(),
        target: None,
        run: Some("run".into()),
        now: 1_600,
    });
    assert_eq!(f.task().state, ExecutionState::Working);
    assert_eq!(f.wake("t1").last_delivery.as_deref(), Some("submitting"));
    f.apply(Operation::Receipt {
        room: "room".into(),
        agent: "agent".into(),
        sequence: e.sequence,
        state: "accepted".into(),
    });
    let wake = f.wake("t1");
    assert!(wake.last_delivered_at_ms.is_some() && wake.last_acknowledged_at_ms.is_none());
    f.apply(Operation::Ack {
        room: "room".into(),
        agent: "agent".into(),
        sequence: e.sequence,
        handled: true,
        now: 1_700,
    });
    let wake = f.wake("t1");
    assert_eq!(wake.last_delivery.as_deref(), Some("handled"));
    assert!(wake.last_acknowledged_at_ms.is_some());
    assert!(f.task().obligations.iter().all(|o| o.status == "satisfied"));
}

#[test]
fn a03_interval_timer_coalesces_missed_fires_and_stays_armed() {
    let f = Fixture::new();
    f.create("t1", "timer", Some(60_000), Some(60_000));
    f.wait_on(&["t1"]);
    // Kernel down / host asleep for five intervals: one occurrence, explicit count.
    f.wakes(Operation::FireWakes { now: 300_010 });
    let events = f.inbox();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "timer_fired");
    assert_eq!(events[0].payload["missed_fires"], 4);
    let wake = f.wake("t1");
    assert_eq!(
        (wake.state.as_str(), wake.next_due_ms, wake.missed_fires),
        ("scheduled", Some(360_000), 4)
    );
    assert!(f.wakes(Operation::FireWakes { now: 359_999 }).is_empty());
    f.wakes(Operation::FireWakes { now: 360_000 });
    assert_eq!(f.inbox().len(), 2);
    assert!(f
        .task()
        .obligations
        .iter()
        .any(|o| o.id == "t1" && o.status == "open"));
}

#[test]
fn a03_process_exit_and_loss_rewake_the_waiting_task() {
    for (code, kind, status) in [
        (Some(0), "source_completed", "settling"),
        (Some(2), "source_lost", "failed"),
        (None, "source_lost", "failed"),
    ] {
        let f = Fixture::new();
        f.create("p1", "process", None, None);
        assert_eq!(f.task().obligations[0].dispatch_state, "intent");
        f.wakes(Operation::ProcessStarted {
            id: "p1".into(),
            pid: 4242,
            now: 15,
        });
        let wake = f.wake("p1");
        assert_eq!(
            (wake.state.as_str(), wake.pid, wake.verified_at_ms),
            ("running", Some(4242), Some(15))
        );
        f.wait_on(&["p1"]);
        f.wakes(Operation::ProcessMatched {
            id: "p1".into(),
            line: "server ready".into(),
            now: 40,
        });
        f.wakes(Operation::ProcessMatched {
            id: "p1".into(),
            line: "server ready again".into(),
            now: 41,
        });
        f.wakes(Operation::ProcessExited {
            id: "p1".into(),
            exit_code: code,
            tail: "tail".into(),
            now: 50,
        });
        f.wakes(Operation::ProcessExited {
            id: "p1".into(),
            exit_code: code,
            tail: "tail".into(),
            now: 51,
        });
        let events = f.inbox();
        assert_eq!(
            events.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(),
            vec!["process_output_matched", kind]
        );
        assert_eq!(
            events[1].payload["public_answer"]["process_lost"],
            code.is_none()
        );
        assert_eq!(f.task().obligations[0].status, status);
        assert_eq!(
            f.wake("p1").state,
            if code.is_some() { "exited" } else { "lost" }
        );
    }
}

#[test]
fn a03_owning_turn_cancels_a_wake_and_can_then_finish() {
    let f = Fixture::new();
    f.create("t1", "timer", Some(1_000), Some(60_000));
    assert!(f
        .store
        .agent_lifecycle(Operation::CancelWake {
            id: "t1".into(),
            task: "p".into(),
            prompt: Some("other".into())
        })
        .is_err());
    assert!(f
        .store
        .agent_lifecycle(Operation::CancelWake {
            id: "t1".into(),
            task: "p".into(),
            prompt: None
        })
        .is_err());
    f.wakes(Operation::CancelWake {
        id: "t1".into(),
        task: "p".into(),
        prompt: Some("p".into()),
    });
    assert_eq!(f.wake("t1").state, "cancelled");
    assert!(f.wakes(Operation::FireWakes { now: 5_000 }).is_empty());
    f.apply(Operation::Settle {
        room: "room".into(),
        agent: "agent".into(),
        prompt: "p".into(),
        run: "run".into(),
        has_answer: true,
        cancelled: false,
        now: 30,
    });
    assert_eq!(f.task().state, ExecutionState::Done);
}

#[test]
fn a03_task_cancellation_settles_due_wakes_without_firing() {
    let f = Fixture::new();
    f.create("t1", "timer", Some(1_000), None);
    f.apply(Operation::CancelTask {
        task: "p".into(),
        owner: "owner".into(),
        revision: f.task().revision,
    });
    assert!(f.wakes(Operation::FireWakes { now: 5_000 }).is_empty());
    assert_eq!(f.wake("t1").state, "cancelled");
    assert_eq!(f.task().obligations[0].status, "cancelled");
    assert!(f.inbox().iter().all(|e| e.kind != "source_completed"));
}

#[test]
fn a03_wake_creation_is_bounded_and_bound_to_the_current_turn() {
    let f = Fixture::new();
    let mut bad = f.create("t0", "timer", Some(1_000), None);
    bad.id = "t-bad".into();
    bad.registration_id = "completion-t-bad".into();
    bad.next_due_ms = Some(5);
    assert!(
        f.store
            .agent_lifecycle(Operation::CreateWake {
                task: "p".into(),
                prompt: "p".into(),
                wake: bad.clone()
            })
            .is_err(),
        "past due"
    );
    bad.next_due_ms = Some(1_000);
    assert!(
        f.store
            .agent_lifecycle(Operation::CreateWake {
                task: "p".into(),
                prompt: "stale".into(),
                wake: bad
            })
            .is_err(),
        "stale turn"
    );
    for i in 1..super::wakes::MAX_ACTIVE_WAKES {
        f.create(&format!("t{i}"), "timer", Some(1_000), None);
    }
    let mut over = f.wake("t0");
    over.id = "over".into();
    over.registration_id = "completion-over".into();
    assert!(f
        .store
        .agent_lifecycle(Operation::CreateWake {
            task: "p".into(),
            prompt: "p".into(),
            wake: over
        })
        .is_err());
}

// MP-09: real abrupt-restart drill exposed this old-progress/new-admission race.
#[test]
fn a03_overdue_wake_cold_admission_has_a_bounded_clock_without_fake_progress() {
    let f = Fixture::new();
    f.create("process", "process", None, None);
    f.wait_on(&["process"]);
    f.apply(Operation::ProcessExited {
        id: "process".into(),
        exit_code: None,
        tail: "process_lost".into(),
        now: 300_000,
    });
    let event = f.inbox().remove(0);
    let progress_before_admission = f.task().last_progress_at_ms;
    f.apply(Operation::Attempt {
        room: "room".into(),
        agent: "agent".into(),
        sequence: event.sequence,
        prompt: "fresh-wake".into(),
        target: None,
        run: None,
        now: 300_001,
    });
    let task = f.task();
    assert!(lacks_live_executor(&task, 300_002, false, false));
    assert!(f
        .store
        .agent_has_live_wake_admission(&task, || 300_002)
        .unwrap());
    assert!(!f
        .store
        .agent_has_live_wake_admission(&task, || 300_001 + DELIVERY_TIMEOUT_MS)
        .unwrap());
    assert_eq!(task.last_progress_at_ms, progress_before_admission);
    let mut wrong = task.clone();
    wrong.pending_prompt_id = Some("unrelated".into());
    assert!(!f
        .store
        .agent_has_live_wake_admission(&wrong, || 300_002)
        .unwrap());
    let mut foreign = task.clone();
    foreign.room_id = "foreign".into();
    assert!(!f
        .store
        .agent_has_live_wake_admission(&foreign, || 300_002)
        .unwrap());
    f.apply(Operation::Begin {
        owner: "owner".into(),
        room: "room".into(),
        agent: "agent".into(),
        prompt: "fresh-wake".into(),
        run: None,
        now: 300_003,
    });
    let promoted = f.task();
    assert_eq!(promoted.prompt_id, "fresh-wake");
    assert!(promoted.pending_prompt_id.is_none());
    assert_eq!(promoted.last_progress_at_ms, progress_before_admission);
    assert!(
        f.store
            .agent_has_live_wake_admission(&promoted, || 300_004)
            .unwrap(),
        "promoting the exact wake prompt is admission, not an abandoned executor"
    );
    assert!(!f
        .store
        .agent_has_live_wake_admission(&promoted, || 300_001 + DELIVERY_TIMEOUT_MS)
        .unwrap());
    let mut unrelated = promoted;
    unrelated.prompt_id = "unrelated".into();
    assert!(!f
        .store
        .agent_has_live_wake_admission(&unrelated, || 300_004)
        .unwrap());
}
