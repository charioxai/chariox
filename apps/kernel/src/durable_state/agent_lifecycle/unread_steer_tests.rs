//! MP-08 / MP-09 / MP-10 / MP-11: a steered row the turn never read returns to pending when the turn completes.
use super::*;
use tests::Fixture;

const ROOM: &str = "room";
const AGENT: &str = "parent";

fn urgent(f: &Fixture, id: &str) -> InboxEvent {
    let mut event = occurrence(
        ROOM,
        AGENT,
        "child",
        id,
        "child_finished",
        serde_json::json!({"n": id}),
    );
    event.urgent = true;
    let Outcome::Event(event) = f.apply(Operation::Occur(event)) else {
        panic!("event")
    };
    event
}

/// The kernel steers the row into the running `target` turn and the provider accepts it.
fn steer(store: &DurableKernelStateStore, row: &InboxEvent, target: &str) {
    for op in [
        Operation::Attempt {
            room: ROOM.into(),
            agent: AGENT.into(),
            sequence: row.sequence,
            prompt: format!("steer-{target}-{}", row.sequence),
            target: Some(target.into()),
            run: Some("run".into()),
            now: 10,
            work: None,
            through: 0,
        },
        Operation::Receipt {
            room: ROOM.into(),
            agent: AGENT.into(),
            sequence: row.sequence,
            state: "accepted".into(),
            now: 11,
        },
    ] {
        store.agent_lifecycle(op).unwrap();
    }
}

fn settle(store: &DurableKernelStateStore, prompt: &str) {
    store
        .agent_lifecycle(Operation::Settle {
            room: ROOM.into(),
            agent: AGENT.into(),
            prompt: prompt.into(),
            run: "run".into(),
            has_answer: true,
            cancelled: false,
            now: 12,
        })
        .unwrap();
}

fn row(store: &DurableKernelStateStore, sequence: u64) -> InboxEvent {
    store
        .agent_inbox(ROOM, AGENT, sequence - 1)
        .unwrap()
        .remove(0)
}

fn read(store: &DurableKernelStateStore, sequence: u64) {
    store
        .agent_lifecycle(Operation::MarkShown {
            room: ROOM.into(),
            agent: AGENT.into(),
            sequences: vec![sequence],
        })
        .unwrap();
}

#[test]
fn a_steered_row_never_read_returns_to_pending_when_the_prompt_completes() {
    let f = Fixture::new();
    f.begin("p");
    let r = urgent(&f, "a");
    steer(&f.store, &r, "p");
    assert_eq!(row(&f.store, r.sequence).state, "accepted");
    settle(&f.store, "p");
    let after = row(&f.store, r.sequence);
    assert_eq!(after.state, "pending");
    assert!(after.prompt_id.is_none() && after.target_prompt_id.is_none());
    assert!(
        after.attempted_at_ms.is_some(),
        "a reverted row is never steered again"
    );
    // Completing the same prompt again raises nothing further.
    settle(&f.store, "p");
    assert_eq!(row(&f.store, r.sequence).state, "pending");
}

#[test]
fn a_steered_row_the_turn_read_is_not_reverted() {
    let f = Fixture::new();
    f.begin("p");
    let r = urgent(&f, "a");
    steer(&f.store, &r, "p");
    read(&f.store, r.sequence);
    settle(&f.store, "p");
    assert_eq!(row(&f.store, r.sequence).state, "accepted");
}

#[test]
fn a_settled_row_is_not_reverted() {
    let f = Fixture::new();
    f.begin("p");
    let r = urgent(&f, "a");
    steer(&f.store, &r, "p");
    read(&f.store, r.sequence);
    f.apply(Operation::Ack {
        room: ROOM.into(),
        agent: AGENT.into(),
        sequence: r.sequence,
        handled: false,
        now: 11,
    });
    settle(&f.store, "p");
    assert_eq!(row(&f.store, r.sequence).state, "acknowledged");
}

#[test]
fn a_row_steered_into_another_prompt_is_not_reverted() {
    let f = Fixture::new();
    f.begin("p");
    let r = urgent(&f, "a");
    steer(&f.store, &r, "other");
    settle(&f.store, "p");
    assert_eq!(row(&f.store, r.sequence).state, "accepted");
}

#[test]
fn only_the_unread_rows_of_the_prompt_are_reverted() {
    let f = Fixture::new();
    f.begin("p");
    let (a, b) = (urgent(&f, "a"), urgent(&f, "b"));
    steer(&f.store, &a, "p");
    steer(&f.store, &b, "p");
    read(&f.store, a.sequence);
    settle(&f.store, "p");
    assert_eq!(row(&f.store, a.sequence).state, "accepted");
    assert_eq!(row(&f.store, b.sequence).state, "pending");
}

/// One more turn that steers the row and ends without reading it.
fn unread_round(store: &DurableKernelStateStore, row: &InboxEvent, prompt: &str) {
    store
        .agent_lifecycle(Operation::Begin {
            owner: "owner".into(),
            room: ROOM.into(),
            agent: AGENT.into(),
            prompt: prompt.into(),
            run: Some("run".into()),
            now: 1,
        })
        .unwrap();
    steer(store, row, prompt);
    settle(store, prompt);
}

#[test]
fn a_row_nobody_reads_is_reverted_a_bounded_number_of_times() {
    let f = Fixture::new();
    let r = urgent(&f, "a");
    let mut states = vec![];
    for round in 0..3 {
        unread_round(&f.store, &r, &format!("p{round}"));
        states.push(row(&f.store, r.sequence).state);
    }
    assert_eq!(states, ["pending", "pending", "accepted"]);
}

#[test]
fn the_revert_count_survives_a_restart() {
    let f = Fixture::new();
    let r = urgent(&f, "a");
    unread_round(&f.store, &r, "p0");
    unread_round(&f.store, &r, "p1");
    let reopened = DurableKernelStateStore::open(f.root.join("state.sqlite")).unwrap();
    unread_round(&reopened, &r, "p2");
    assert_eq!(row(&reopened, r.sequence).state, "accepted");
}

#[test]
fn a_reverted_row_still_needs_a_read_before_it_can_be_acked() {
    let f = Fixture::new();
    f.begin("p");
    let r = urgent(&f, "a");
    steer(&f.store, &r, "p");
    settle(&f.store, "p");
    assert!(f
        .store
        .agent_unread_inbox(ROOM, AGENT, 0, u64::MAX >> 1, None)
        .unwrap()
        .iter()
        .any(|e| e.sequence == r.sequence));
}

#[test]
fn a_steer_accepted_after_its_prompt_settled_returns_to_pending_at_once() {
    let f = Fixture::new();
    f.begin("p");
    let r = urgent(&f, "a");
    f.store
        .agent_lifecycle(Operation::Attempt {
            room: ROOM.into(),
            agent: AGENT.into(),
            sequence: r.sequence,
            prompt: "steer".into(),
            target: Some("p".into()),
            run: Some("run".into()),
            now: 10,
            work: None,
            through: 0,
        })
        .unwrap();
    settle(&f.store, "p");
    f.apply(Operation::Receipt {
        room: ROOM.into(),
        agent: AGENT.into(),
        sequence: r.sequence,
        state: "accepted".into(),
        now: 13,
    });
    assert_eq!(row(&f.store, r.sequence).state, "pending");
}
