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

// MP-08 / MP-09 / MP-10 / MP-11: a new user turn remains a separate task,
// but may explicitly cancel this same agent's retained watcher.
#[test]
fn a03_successor_turn_cancels_waiting_watch_without_transferring_ownership() {
    let f = Fixture::new();
    f.create("process", "process", None, None);
    f.apply(Operation::ProcessStarted {
        id: "process".into(),
        pid: 42,
        now: 20,
    });
    f.wait_on(&["process"]);
    f.apply(Operation::Begin {
        owner: "owner".into(),
        room: "room".into(),
        agent: "agent".into(),
        prompt: "cancel-turn".into(),
        run: Some("run".into()),
        now: 40,
    });
    f.apply(Operation::CancelWake {
        id: "process".into(),
        task: "cancel-turn".into(),
        prompt: Some("cancel-turn".into()),
    });
    assert_eq!(f.wake("process").state, "cancelling");
    assert_eq!(f.wake("process").task_id, "p");
    assert_eq!(f.task().obligations[0].status, "open");
    f.apply(Operation::ProcessExited {
        id: "process".into(),
        exit_code: Some(143),
        tail: String::new(),
        now: 50,
    });
    assert_eq!(f.wake("process").state, "cancelled");
    assert_eq!(f.task().obligations[0].status, "cancelled");
    assert!(f
        .inbox()
        .iter()
        .any(|e| e.kind == "source_lost" && e.payload["task_id"] == "p"));
}

#[test]
fn a03_successor_timer_cancellation_wakes_original_wait() {
    let f = Fixture::new();
    f.create("timer", "timer", Some(1_000), Some(60_000));
    f.wait_on(&["timer"]);
    f.apply(Operation::Begin {
        owner: "owner".into(),
        room: "room".into(),
        agent: "agent".into(),
        prompt: "cancel-turn".into(),
        run: Some("run".into()),
        now: 40,
    });
    f.apply(Operation::CancelWake {
        id: "timer".into(),
        task: "cancel-turn".into(),
        prompt: Some("cancel-turn".into()),
    });
    assert_eq!(f.wake("timer").state, "cancelled");
    assert!(f
        .inbox()
        .iter()
        .any(|e| e.kind == "source_lost" && e.payload["task_id"] == "p"));
    f.apply(Operation::FireWakes { now: 100_000 });
    assert_eq!(f.wake("timer").fire_count, 0);
}

#[test]
fn a03_successor_cancel_rejects_foreign_room_agent_owner_and_stale_turn() {
    for (room, agent, owner, prompt) in [
        ("foreign", "agent", "owner", "cancel-turn"),
        ("room", "other", "owner", "cancel-turn"),
        ("room", "agent", "foreign", "cancel-turn"),
        ("room", "agent", "owner", "stale"),
    ] {
        let f = Fixture::new();
        f.create("timer", "timer", Some(1_000), None);
        f.wait_on(&["timer"]);
        f.apply(Operation::Begin {
            owner: owner.into(),
            room: room.into(),
            agent: agent.into(),
            prompt: "cancel-turn".into(),
            run: Some("run".into()),
            now: 40,
        });
        assert!(f
            .store
            .agent_lifecycle(Operation::CancelWake {
                id: "timer".into(),
                task: "cancel-turn".into(),
                prompt: Some(prompt.into())
            })
            .is_err());
        assert_eq!(f.wake("timer").state, "scheduled");
    }
}

#[test]
fn a03_owner_resume_delivery_cannot_create_a_second_task_before_continuation() {
    let f = Fixture::new();
    f.create("timer", "timer", Some(1_000), Some(60_000));
    f.apply(Operation::FireWakes { now: 1_000 });
    f.apply(Operation::Block {
        task: "p".into(),
        prompt: "p".into(),
        reason: "owner action".into(),
    });
    let revision = f.task().blocked_revision;
    let sequence = f.inbox()[0].sequence;
    f.apply(Operation::OwnerResponse {
        task: "p".into(),
        revision,
        resume: true,
        now: 1_001,
    });
    let resumed = f.task();
    let continuation = resumed.pending_prompt_id.clone().unwrap();
    let attempt = || Operation::Attempt {
        room: "room".into(),
        agent: "agent".into(),
        sequence,
        prompt: "event-turn".into(),
        target: None,
        run: None,
        now: 1_002,
    };
    assert!(
        f.store.agent_lifecycle(attempt()).is_err(),
        "an event must not overtake the owner continuation"
    );
    assert_eq!(f.inbox()[0].state, "pending");
    assert_eq!(f.task(), resumed);
    f.apply(Operation::Begin {
        owner: "owner".into(),
        room: "room".into(),
        agent: "agent".into(),
        prompt: continuation.clone(),
        run: Some("run".into()),
        now: 1_003,
    });
    f.apply(Operation::Yield {
        task: "p".into(),
        prompt: continuation.clone(),
        registrations: vec!["completion-timer".into()],
        cursor: 0,
        deadline: 100_000,
        reason: "resume waiting".into(),
        now: 1_004,
    });
    f.apply(Operation::Settle {
        room: "room".into(),
        agent: "agent".into(),
        prompt: continuation,
        run: "run".into(),
        has_answer: true,
        cancelled: false,
        now: 1_005,
    });
    f.apply(attempt());
    f.apply(Operation::Begin {
        owner: "owner".into(),
        room: "room".into(),
        agent: "agent".into(),
        prompt: "event-turn".into(),
        run: Some("run".into()),
        now: 1_006,
    });
    assert_eq!(f.task().task_id, "p");
    assert_eq!(f.task().prompt_id, "event-turn");
    assert_eq!(
        f.store
            .agent_tasks(Some("room"), Some("agent"))
            .unwrap()
            .len(),
        1
    );
    f.apply(Operation::CancelWake {
        id: "timer".into(),
        task: "p".into(),
        prompt: Some("event-turn".into()),
    });
    assert_eq!(f.wake("timer").state, "cancelled");
}

#[test]
fn a03_interval_keeps_receipts_when_an_earlier_fire_is_acknowledged_late() {
    let f = Fixture::new();
    f.create("interval", "timer", Some(1_000), Some(60_000));
    f.wait_on(&["interval"]);
    f.apply(Operation::FireWakes { now: 1_000 });
    let first = f.inbox()[0].sequence;
    f.apply(Operation::Ack {
        room: "room".into(),
        agent: "agent".into(),
        sequence: first,
        handled: true,
        now: 60_000,
    });
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
        now: 1_600,
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
    f.apply(Operation::Ack {
        room: "room".into(),
        agent: "agent".into(),
        sequence: events[0].sequence,
        handled: true,
        now: 359_999,
    });
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

// MP-09/MP-11: a long-lived inbox must retain the exact new admission witness.
#[test]
fn a03_wake_admission_survives_more_than_one_page_of_handled_history() {
    let f = Fixture::new();
    f.create("process", "process", None, None);
    f.wait_on(&["process"]);
    for index in 0..128 {
        let Outcome::Event(event) = f.apply(Operation::Occur(InboxEvent {
            sequence: 0,
            room_id: "room".into(),
            agent_id: "agent".into(),
            source_id: "history".into(),
            occurrence_id: format!("handled-{index}"),
            kind: "message".into(),
            payload: serde_json::json!({}),
            urgent: false,
            reply_requested: false,
            state: "pending".into(),
            prompt_id: None,
            target_prompt_id: None,
            provider_run_id: None,
            attempted_at_ms: None,
            submit_epoch: None,
        })) else {
            panic!("expected historical event")
        };
        f.apply(Operation::Ack {
            room: "room".into(),
            agent: "agent".into(),
            sequence: event.sequence,
            handled: true,
            now: 100,
        });
    }
    f.apply(Operation::ProcessExited {
        id: "process".into(),
        exit_code: None,
        tail: "process_lost".into(),
        now: 300_000,
    });
    let event = f.store.agent_inbox("room", "agent", 128).unwrap().remove(0);
    assert!(event.sequence > 128);
    f.apply(Operation::Attempt {
        room: "room".into(),
        agent: "agent".into(),
        sequence: event.sequence,
        prompt: "fresh-wake".into(),
        target: None,
        run: None,
        now: 300_001,
    });
    assert!(
        f.store
            .agent_has_live_wake_admission(&f.task(), || 300_002)
            .unwrap(),
        "handled inbox history must not hide the exact new wake admission"
    );
}

#[test]
fn a03_recurring_timer_check_ins_require_owner_after_three_without_progress() {
    let f = Fixture::new();
    f.create("t1", "timer", Some(60_000), Some(60_000));
    f.wait_on(&["t1"]);
    // Wake -> inspect -> nothing changed -> ack -> wait; owner gate at three.
    for n in 1..=3u64 {
        let now = n * 60_000;
        assert_eq!(f.wakes(Operation::FireWakes { now }).len(), 1);
        let e = f.inbox().pop().unwrap();
        assert_eq!(e.kind, "timer_fired");
        let prompt = format!("check-in-{n}");
        f.apply(Operation::Attempt {
            room: "room".into(),
            agent: "agent".into(),
            sequence: e.sequence,
            prompt: prompt.clone(),
            target: None,
            run: Some("run".into()),
            now: now + 1,
        });
        f.apply(Operation::Receipt {
            room: "room".into(),
            agent: "agent".into(),
            sequence: e.sequence,
            state: "accepted".into(),
            now: now + 1,
        });
        f.apply(Operation::Begin {
            owner: "owner".into(),
            room: "room".into(),
            agent: "agent".into(),
            prompt: prompt.clone(),
            run: Some("run".into()),
            now: now + 2,
        });
        f.apply(Operation::Ack {
            room: "room".into(),
            agent: "agent".into(),
            sequence: e.sequence,
            handled: true,
            now: now + 3,
        });
        f.apply(Operation::Yield {
            task: "p".into(),
            prompt: prompt.clone(),
            registrations: vec!["completion-t1".into()],
            cursor: e.sequence,
            deadline: 10_000_000,
            reason: "next check-in".into(),
            now: now + 4,
        });
        f.apply(Operation::Settle {
            room: "room".into(),
            agent: "agent".into(),
            prompt,
            run: "run".into(),
            has_answer: true,
            cancelled: false,
            now: now + 5,
        });
        assert_eq!(
            f.task().state,
            if n < 3 {
                ExecutionState::Waiting
            } else {
                ExecutionState::Blocked
            },
            "a check-in with no useful progress consumes its bounded wake budget"
        );
    }
    assert!(f.wakes(Operation::FireWakes { now: 240_000 }).is_empty());
    assert_eq!(
        f.wake("t1").fire_count,
        3,
        "blocked timers stay inert until owner action"
    );
}

#[test]
fn a03_cancellation_is_not_recorded_as_a_fire() {
    let f = Fixture::new();
    f.create("t1", "timer", Some(1_000), None);
    f.apply(Operation::CancelTask {
        task: "p".into(),
        owner: "owner".into(),
        revision: f.task().revision,
    });
    f.apply(Operation::FireWakes { now: 5_000 });
    let wake = f.wake("t1");
    assert_eq!(wake.state, "cancelled");
    assert_eq!(
        (wake.fire_count, wake.last_fired_at_ms, wake.last_sequence),
        (0, None, None),
        "a timer that never fired shows no fire"
    );
    assert!(f.store.agent_wake_receipts(None, None).unwrap().is_empty());
}

#[test]
fn a03_teardown_retires_wakes_without_owner_authority() {
    let f = Fixture::new();
    f.create("t1", "timer", Some(1_000), Some(60_000));
    f.create("process", "process", None, None);
    f.apply(Operation::ProcessStarted {
        id: "process".into(),
        pid: 42,
        now: 20,
    });
    f.wait_on(&["t1", "process"]);
    // A Sweep/Settle-admitted task has no owner; owner cancellation refuses it.
    f.apply(Operation::Begin {
        owner: String::new(),
        room: "room".into(),
        agent: "agent".into(),
        prompt: "unowned".into(),
        run: None,
        now: 25,
    });
    let retired = f.wakes(Operation::RetireWakes {
        room: "room".into(),
        agent: Some("agent".into()),
        now: 30,
    });
    assert_eq!(retired.len(), 2);
    assert_eq!(f.wake("t1").state, "cancelled");
    assert_eq!(
        f.wake("process").state,
        "cancelling",
        "a process settles only on its physical exit"
    );
    assert!(f.wakes(Operation::FireWakes { now: 120_000 }).is_empty());
    f.apply(Operation::ProcessExited {
        id: "process".into(),
        exit_code: Some(143),
        tail: String::new(),
        now: 40,
    });
    assert_eq!(f.wake("process").state, "cancelled");
    // The removed recipient keeps no work for the delivery sweep to retry.
    let tasks = f.store.agent_tasks(Some("room"), Some("agent")).unwrap();
    assert!(tasks.iter().all(|t| t.state == ExecutionState::Cancelled));
    assert!(f.inbox().iter().all(|e| e.state != "pending"));
    assert!(f.store.agent_pending_inbox_recipients().unwrap().is_empty());
}

#[test]
fn a03_teardown_settles_inflight_submitting_delivery_before_timeout() {
    teardown_settles_inflight_deliveries("submitting");
}

#[test]
fn a03_teardown_settles_inflight_uncertain_delivery_before_timeout() {
    teardown_settles_inflight_deliveries("uncertain");
}

#[test]
fn a03_teardown_settles_inflight_blocked_delivery() {
    teardown_settles_inflight_deliveries("blocked");
}

fn teardown_settles_inflight_deliveries(state: &str) {
    for agent in [Some("agent"), None] {
        let f = Fixture::new();
        f.create("t1", "timer", Some(1_000), Some(60_000));
        f.wait_on(&["t1"]);
        f.apply(Operation::FireWakes { now: 1_000 });
        let sequence = f.inbox()[0].sequence;
        f.apply(Operation::Attempt {
            room: "room".into(),
            agent: "agent".into(),
            sequence,
            prompt: "wake-prompt".into(),
            target: None,
            run: Some("wake-run".into()),
            now: 1_001,
        });
        if state == "uncertain" {
            f.apply(Operation::Receipt {
                room: "room".into(),
                agent: "agent".into(),
                sequence,
                state: state.into(),
                now: 1_002,
            });
        } else if state == "blocked" {
            f.apply(Operation::Sweep {
                now: 1_001 + DELIVERY_TIMEOUT_MS,
                busy_recipients: vec![],
            });
        }
        assert_eq!(f.inbox()[0].state, state);
        // Agent removal leaves peers alone; Room teardown leaves other Rooms alone.
        for (room, recipient) in [("room", "peer"), ("other-room", "agent")] {
            f.apply(Operation::Occur(occurrence(
                room,
                recipient,
                "source",
                "control",
                "message",
                serde_json::json!({}),
            )));
        }
        for _ in 0..2 {
            f.apply(Operation::RetireWakes {
                room: "room".into(),
                agent: agent.map(str::to_owned),
                now: 2_000 + DELIVERY_TIMEOUT_MS,
            });
        }
        f.apply(Operation::Sweep {
            now: 3_000 + 2 * DELIVERY_TIMEOUT_MS,
            busy_recipients: vec![],
        });
        assert!(
            f.store
                .agent_tasks(Some("room"), Some("agent"))
                .unwrap()
                .iter()
                .all(|t| t.state == ExecutionState::Cancelled),
            "teardown of {state} must not produce a new Blocked delivery task"
        );
        assert_eq!(f.inbox()[0].state, "expired");
        assert_eq!(f.wake("t1").last_delivery.as_deref(), Some("expired"));
        let receipts = f.store.agent_wake_receipts(None, None).unwrap();
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0].delivery, "expired");
        assert!(receipts[0].delivered_at_ms.is_none());
        assert!(receipts[0].acknowledged_at_ms.is_none());
        for state in ["accepted", "rejected", "uncertain"] {
            assert!(f
                .store
                .agent_lifecycle(Operation::Receipt {
                    room: "room".into(),
                    agent: "agent".into(),
                    sequence,
                    state: state.into(),
                    now: 4_000 + 2 * DELIVERY_TIMEOUT_MS,
                })
                .is_err());
        }
        let reopened = DurableKernelStateStore::open(f.root.join("state.sqlite")).unwrap();
        assert_eq!(
            reopened.agent_inbox("room", "agent", 0).unwrap()[0].state,
            "expired"
        );
        assert_eq!(
            reopened.agent_inbox("room", "peer", 0).unwrap()[0].state,
            if agent.is_some() {
                "pending"
            } else {
                "expired"
            }
        );
        assert_eq!(
            reopened.agent_inbox("other-room", "agent", 0).unwrap()[0].state,
            "pending"
        );
    }
}

#[test]
fn a03_wake_history_and_receipts_are_bounded() {
    let f = Fixture::new();
    f.create("interval", "timer", Some(60_000), Some(60_000));
    for n in 1..=100u64 {
        f.apply(Operation::FireWakes { now: n * 60_000 });
        let e = f.inbox().pop().unwrap();
        f.apply(Operation::Ack {
            room: "room".into(),
            agent: "agent".into(),
            sequence: e.sequence,
            handled: true,
            now: n * 60_000 + 1,
        });
    }
    let receipts = f.store.agent_wake_receipts(None, None).unwrap();
    assert!(receipts.len() <= 64, "{} receipts retained", receipts.len());
    assert_eq!(
        receipts.last().map(|r| r.sequence),
        f.wake("interval").last_sequence,
        "the newest receipt is kept"
    );
    for n in 0..100 {
        let id = format!("finished-{n}");
        f.create(&id, "process", None, None);
        f.apply(Operation::ProcessExited {
            id: id.clone(),
            exit_code: None,
            tail: String::new(),
            now: 50,
        });
        // History retention is independent of the outstanding source budget.
        f.apply(Operation::Ack {
            room: "room".into(),
            agent: "agent".into(),
            sequence: f.wake(&id).last_sequence.unwrap(),
            handled: true,
            now: 51,
        });
    }
    let wakes = f.store.agent_wakes(None, None).unwrap();
    assert!(
        wakes.len() <= 66,
        "finished wakes are bounded per agent, {} retained",
        wakes.len()
    );
    assert!(
        wakes.iter().any(|w| w.id == "interval"),
        "armed wakes are kept"
    );
    assert!(wakes.iter().any(|w| w.id == "finished-99"));
}

// MP-09/MP-11 secrev-d F1: a timer cannot fill its own inbox without ACKs.
#[test]
fn security_f1_one_shot_admission_bounds_unhandled_fires() {
    let f = Fixture::new();
    for n in 0..32 {
        f.create(&format!("flood-{n}"), "timer", Some(1000), None);
        f.apply(Operation::FireWakes { now: 1000 });
    }
    let mut wake = f.wake("flood-31");
    wake.id = "overflow".into();
    wake.registration_id = "completion-overflow".into();
    wake.next_due_ms = Some(2000);
    let result = f.store.agent_lifecycle(Operation::CreateWake {
        task: "p".into(),
        prompt: "p".into(),
        wake,
    });
    assert!(
        result.is_err(),
        "unhandled fires must reserve wake capacity"
    );
}

#[test]
fn security_f1_recurring_fire_coalesces_until_handled() {
    let f = Fixture::new();
    f.create("recurring", "timer", Some(1000), Some(60000));
    f.apply(Operation::FireWakes { now: 1000 });
    f.apply(Operation::FireWakes { now: 181000 });
    assert_eq!(
        f.inbox().len(),
        1,
        "one unhandled occurrence per recurring source"
    );
    let seq = f.inbox()[0].sequence;
    f.apply(Operation::Ack {
        room: "room".into(),
        agent: "agent".into(),
        sequence: seq,
        handled: true,
        now: 181001,
    });
    f.apply(Operation::FireWakes { now: 181002 });
    assert_eq!(f.wake("recurring").fire_count, 2);
    assert_eq!(f.wake("recurring").missed_fires, 2);
}

#[test]
fn security_f1_full_recipient_does_not_stop_peer_timers() {
    let f = Fixture::new();
    let mut peer = f.create("healthy", "timer", Some(1000), None);
    f.apply(Operation::Begin {
        owner: "owner".into(),
        room: "other-room".into(),
        agent: "peer".into(),
        prompt: "peer-turn".into(),
        run: Some("peer-run".into()),
        now: 1,
    });
    peer.id = "peer-wake".into();
    peer.task_id = "peer-turn".into();
    peer.room_id = "other-room".into();
    peer.agent_id = "peer".into();
    peer.registration_id = "completion-peer-wake".into();
    f.apply(Operation::CreateWake {
        task: "peer-turn".into(),
        prompt: "peer-turn".into(),
        wake: peer,
    });
    for n in 0..1024 {
        f.apply(Operation::Occur(occurrence(
            "room",
            "agent",
            "sender",
            &n.to_string(),
            "message",
            serde_json::json!({}),
        )));
    }
    let result = f.store.agent_lifecycle(Operation::FireWakes { now: 1000 });
    assert!(
        result.is_ok(),
        "one recipient must not roll back all timers: {result:?}"
    );
    let peer = f
        .store
        .agent_wakes(Some("other-room"), Some("peer"))
        .unwrap();
    assert_eq!(peer[0].state, "fired");
    // #914 may bypass the message cap for derived events. Until its shared
    // policy lands, #925 retains the blocked occurrence for retry. Both paths
    // must keep the occurrence, never silently lose it or stop peer timers.
    let own = f.wake("healthy");
    assert!(
        (own.state == "scheduled" && own.fire_count == 0 && own.next_due_ms == Some(1000))
            || (own.state == "fired" && own.fire_count == 1 && own.last_sequence.is_some()),
        "full recipient must retain a retryable or committed occurrence: {own:?}"
    );
    let oldest = f.inbox()[0].sequence;
    f.apply(Operation::Ack {
        room: "room".into(),
        agent: "agent".into(),
        sequence: oldest,
        handled: true,
        now: 1001,
    });
    f.apply(Operation::FireWakes { now: 1002 });
    assert_eq!(f.wake("healthy").state, "fired");
    assert_eq!(
        f.wake("healthy").fire_count,
        1,
        "retry cannot duplicate the occurrence"
    );
}

// MP-09/MP-11 F12: acknowledgement alone is not useful progress.
#[test]
fn security_f12_recurring_ack_does_not_reset_no_progress_budget() {
    let f = Fixture::new();
    f.create("check-in", "timer", Some(1000), Some(60000));
    f.wait_on(&["check-in"]);
    let before = f.task();
    f.apply(Operation::FireWakes { now: 1000 });
    let seq = f.inbox()[0].sequence;
    f.apply(Operation::Attempt {
        room: "room".into(),
        agent: "agent".into(),
        sequence: seq,
        prompt: "check-in-turn".into(),
        target: None,
        run: Some("run".into()),
        now: 1001,
    });
    f.apply(Operation::Receipt {
        room: "room".into(),
        agent: "agent".into(),
        sequence: seq,
        state: "accepted".into(),
        now: 1002,
    });
    f.apply(Operation::Ack {
        room: "room".into(),
        agent: "agent".into(),
        sequence: seq,
        handled: true,
        now: 1003,
    });
    let after = f.task();
    assert_eq!(
        after.no_progress_wakes, 1,
        "a handled timer cannot fund another endless wake turn"
    );
    assert_eq!(after.progress_sequence, before.progress_sequence);
    assert_eq!(after.last_progress_at_ms, before.last_progress_at_ms);
}

// MP-08 / MP-09 / MP-10 / MP-11: canonical room workflow admission
// and legacy persisted admission both get a completion registration.
#[test]
fn a03_workflow_run_receipt_admits_a_live_completion_registration() {
    for kind in ["workflow", "workflow_run"] {
        let f = Fixture::new();
        f.apply(Operation::RegisterObligation {
            owner: "owner".into(),
            room: "room".into(),
            agent: "agent".into(),
            prompt: "p".into(),
            run: Some("run".into()),
            id: "workflow-obligation".into(),
            kind: kind.into(),
            resource: Some("workflow-run".into()),
            now: 10,
        });
        f.apply(Operation::DispatchReceipt {
            id: "workflow-obligation".into(),
            accepted: true,
            resource: Some("workflow-run".into()),
        });
        let regs = f.store.agent_registrations("p").unwrap();
        assert!(
            regs.iter().any(|r| r.source_id == "workflow-run" && r.live),
            "{kind} must admit its completion source"
        );
    }
}
