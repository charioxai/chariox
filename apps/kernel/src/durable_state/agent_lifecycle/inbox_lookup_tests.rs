//! MP-08 / MP-09 / MP-10 / MP-11: a room inbox keeps its closed rows, so the lookups a delivery
//! receipt and a provider settlement make per prompt must not read the whole retained history.
use super::*;
use tests::Fixture;

/// Steps the per-prompt lookups take for one steered receipt and one settlement, with `closed`
/// handled rows already in the recipient's inbox.
fn lookup_steps(closed: usize) -> i32 {
    let f = Fixture::new();
    f.begin("p");
    let mut db = Connection::open(f.root.join("state.sqlite")).unwrap();
    let tx = db.transaction().unwrap();
    for index in 0..closed {
        let payload = serde_json::json!({
            "prompt_id": format!("old-{index}"),
            "target_prompt_id": format!("old-target-{index}"),
            "state": "handled",
        });
        tx.execute(
            "INSERT INTO agent_inbox(room_id,agent_id,source_id,occurrence_id,payload) VALUES('room','parent','child',?1,?2)",
            params![format!("old-{index}"), payload.to_string()],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    let mut event = occurrence(
        "room",
        "parent",
        "child",
        "fresh",
        "child_finished",
        serde_json::json!({"n": "fresh"}),
    );
    event.urgent = true;
    let Outcome::Event(event) = f.apply(Operation::Occur(event)) else {
        panic!("event")
    };
    // The writer thread keeps its own counters, so the operations run on this thread.
    let mut run = |op: Operation| {
        let tx = db.transaction().unwrap();
        apply(&tx, op).unwrap();
        tx.commit().unwrap();
    };
    INBOX_LOOKUP_STEPS.with(|n| n.set(0));
    for op in [
        Operation::Attempt {
            room: "room".into(),
            agent: "parent".into(),
            sequence: event.sequence,
            prompt: "steer".into(),
            target: Some("p".into()),
            run: Some("run".into()),
            now: 10,
            work: None,
            through: 0,
        },
        Operation::Receipt {
            room: "room".into(),
            agent: "parent".into(),
            sequence: event.sequence,
            state: "accepted".into(),
            now: 11,
        },
    ] {
        run(op);
    }
    let receipt = INBOX_LOOKUP_STEPS.with(|n| n.get());
    run(Operation::Settle {
        room: "room".into(),
        agent: "parent".into(),
        prompt: "p".into(),
        run: "run".into(),
        has_answer: true,
        cancelled: false,
        now: 12,
    });
    let settled = INBOX_LOOKUP_STEPS.with(|n| n.get()) - receipt;
    assert!(
        receipt > 0 && settled > 0,
        "the lookups were not measured: {receipt} and {settled} steps"
    );
    receipt + settled
}

#[test]
fn per_prompt_inbox_lookups_do_not_walk_the_closed_history() {
    let (few, many) = (lookup_steps(10), lookup_steps(2000));
    assert!(
        many <= few + 256,
        "the lookups took {few} steps with 10 closed rows and {many} with 2000"
    );
}

#[test]
fn inbox_arrival_survives_idempotent_admission_and_reopen() {
    let root = crate::test_temp_dir::TestTempDir::new("inbox-arrival");
    let path = root.path().join("state.sqlite");
    let accepted = {
        let store = DurableKernelStateStore::open_owned(path.clone()).unwrap();
        let occurrence = occurrence(
            "room",
            "agent",
            "peer",
            "one",
            "peer_event",
            serde_json::json!({}),
        );
        let Outcome::Event(first) = store
            .agent_lifecycle(Operation::Occur(occurrence.clone()))
            .unwrap()
        else {
            panic!("first occurrence must have an inbox row");
        };
        let Outcome::Event(replay) = store.agent_lifecycle(Operation::Occur(occurrence)).unwrap()
        else {
            panic!("retried occurrence must return its inbox row");
        };
        assert!(first.arrived_at_ms.is_some());
        assert_eq!(replay.sequence, first.sequence);
        assert_eq!(replay.arrived_at_ms, first.arrived_at_ms);
        replay
    };
    let reopened = DurableKernelStateStore::open(path).unwrap();
    assert_eq!(
        reopened.agent_inbox("room", "agent", 0).unwrap(),
        vec![accepted]
    );
}
