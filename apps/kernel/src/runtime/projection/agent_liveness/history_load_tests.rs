use super::*;
use crate::durable_state::agent_lifecycle::{Operation, Outcome};
use crate::durable_state::DurableKernelStateStore;
use crate::test_temp_dir::TestTempDir;

fn waiting_store(root: &std::path::Path) -> DurableKernelStateStore {
    let store = DurableKernelStateStore::open_owned(root.join("state.sqlite")).unwrap();
    let apply = |operation| store.agent_lifecycle(operation).unwrap();
    apply(Operation::Begin {
        owner: "owner".into(),
        room: "room".into(),
        agent: "agent".into(),
        prompt: "turn".into(),
        run: Some("run".into()),
        now: 1,
    });
    let wake: AgentWake = serde_json::from_value(serde_json::json!({
        "id": "live", "task_id": "turn", "room_id": "room", "agent_id": "agent",
        "registration_id": "completion-live", "kind": "timer", "label": "check build",
        "state": "scheduled", "created_at_ms": 2, "verified_at_ms": 2,
        "next_due_ms": 100000, "command": [], "fire_count": 0, "missed_fires": 0
    }))
    .unwrap();
    let Outcome::Wakes(_) = apply(Operation::CreateWake {
        task: "turn".into(),
        prompt: "turn".into(),
        wake,
    }) else {
        panic!("wake must be created")
    };
    apply(Operation::Yield {
        task: "turn".into(),
        prompt: "turn".into(),
        registrations: vec!["completion-live".into()],
        cursor: 0,
        deadline: Some(200000),
        reason: "wait for build".into(),
        now: 3,
    });
    apply(Operation::Settle {
        room: "room".into(),
        agent: "agent".into(),
        prompt: "turn".into(),
        run: "run".into(),
        has_answer: true,
        cancelled: false,
        now: 4,
    });
    store
}

fn seed_finished_wakes(root: &std::path::Path, malformed: bool) {
    let mut db = rusqlite::Connection::open(root.join("state.sqlite")).unwrap();
    let tx = db.transaction().unwrap();
    let mut payload = serde_json::json!({
        "id": "retired", "task_id": "old-turn", "room_id": "room", "agent_id": "retired-agent",
        "registration_id": "old-registration", "kind": "process", "label": "old build",
        "state": "exited", "created_at_ms": 1, "command": vec!["x".repeat(2048); 8],
        "fire_count": 0, "missed_fires": 0
    });
    if malformed {
        payload["command"] = serde_json::json!(42);
    }
    for index in 0..2048 {
        let id = format!("old-{index}");
        let agent = format!("retired-agent-{}", index / 64);
        payload["id"] = serde_json::json!(id);
        payload["agent_id"] = serde_json::json!(agent);
        tx.execute("INSERT INTO agent_wakes(id,task_id,room_id,agent_id,payload) VALUES(?1,'old-turn','room',?2,?3)",
            rusqlite::params![id, agent, payload.to_string()]).unwrap();
    }
    tx.commit().unwrap();
}

#[test]
fn liveness_reads_only_the_wakes_of_waiting_tasks_under_retained_history_load() {
    let root = TestTempDir::new("liveness-retained-wakes");
    let store = waiting_store(&root);
    seed_finished_wakes(&root, false);
    let yielded = YieldedSources::of_room(&store, "room");
    let steps = DurableKernelStateStore::last_waiting_wake_read_steps_for_test();
    assert!(
        steps > 0 && steps < 256,
        "liveness scanned the retained history: {steps} SQLite steps"
    );
    assert_eq!(yielded.tasks.len(), 1);
    assert_eq!(
        yielded.wakes.len(),
        1,
        "every client update must read only the live source, not deserialize all retained commands"
    );
    assert_eq!(yielded.wakes[0].id, "live");
    assert_eq!(
        yielded_wait("agent", &yielded).unwrap().detail,
        "check build"
    );
}

#[test]
fn a_finished_wakes_old_payload_cannot_hide_the_current_wait() {
    let root = TestTempDir::new("liveness-old-wake-payload");
    let store = waiting_store(&root);
    seed_finished_wakes(&root, true);
    let yielded = YieldedSources::of_room(&store, "room");
    assert_eq!(
        yielded.wakes.len(),
        1,
        "an unrelated finished wake must not discard the live source"
    );
    assert_eq!(
        yielded_wait("agent", &yielded).unwrap().kind,
        AgentWaitKind::Timer
    );
}

#[test]
fn room_wake_reads_do_not_sort_retained_command_payloads() {
    let root = TestTempDir::new("room-wake-read-order");
    let store = waiting_store(&root);
    seed_finished_wakes(&root, false);
    let wakes = store.agent_wakes(Some("room"), None).unwrap();
    assert_eq!(wakes.len(), 2049);
    assert_eq!(
        DurableKernelStateStore::last_wake_read_sorts_for_test(),
        0,
        "sorting retained command payloads spills to disk while holding the shared reader"
    );
    assert_eq!(wakes[0].id, "live");
    assert_eq!(wakes.last().unwrap().id, "old-2047");
}
