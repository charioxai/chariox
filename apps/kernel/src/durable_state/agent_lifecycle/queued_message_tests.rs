//! A message that is not urgent waits for its recipient's next turn: it never starts a turn, never
//! stands in front of the wakes behind it, and the person cancels it or sends it now.
use super::*;
use tests::Fixture;

const ROOM: &str = "room";
const AGENT: &str = "child";

fn message(f: &Fixture, id: &str, urgent: bool, reply: bool) -> InboxEvent {
    let mut event = occurrence(
        ROOM,
        AGENT,
        "parent",
        id,
        "message",
        serde_json::json!({"message": id, "sender_agent_id": "parent", "sender_room_id": ROOM}),
    );
    event.urgent = urgent;
    event.reply_requested = reply;
    let Outcome::Event(event) = f.apply(Operation::Send {
        task: "p".into(),
        prompt: "p".into(),
        event,
    }) else {
        panic!("event")
    };
    event
}

fn wake(f: &Fixture, id: &str) -> InboxEvent {
    let Outcome::Event(event) = f.apply(Operation::Occur(occurrence(
        ROOM,
        AGENT,
        "timer",
        id,
        "timer_fired",
        serde_json::json!({"n": id}),
    ))) else {
        panic!("event")
    };
    event
}

fn front(f: &Fixture) -> Option<u64> {
    f.store
        .agent_work_delivery_front(ROOM, AGENT, None)
        .unwrap()
        .map(|event| event.sequence)
}

fn queued(f: &Fixture) -> Vec<u64> {
    f.store
        .agent_queued_messages(ROOM, None)
        .unwrap()
        .iter()
        .map(|event| event.sequence)
        .collect()
}

#[test]
fn a_queued_message_alone_starts_no_turn_and_the_wake_behind_it_is_the_front() {
    let f = Fixture::new();
    f.begin("p");
    let queued_message = message(&f, "hello", false, false);
    assert_eq!(
        front(&f),
        None,
        "a message that is not urgent starts nothing"
    );
    let timer = wake(&f, "t1");
    assert_eq!(
        front(&f),
        Some(timer.sequence),
        "it does not stand in front of a wake"
    );
    assert_eq!(queued(&f), vec![queued_message.sequence]);
}

#[test]
fn an_urgent_message_is_delivered_at_once() {
    let f = Fixture::new();
    f.begin("p");
    let urgent = message(&f, "now", true, false);
    assert_eq!(front(&f), Some(urgent.sequence));
    assert!(queued(&f).is_empty());
}

#[test]
fn the_next_wake_carries_the_queued_messages_after_its_lead() {
    let f = Fixture::new();
    f.begin("p");
    let timer = wake(&f, "t1");
    let first = message(&f, "a", false, false);
    let second = message(&f, "b", false, false);
    let lead = f
        .store
        .agent_work_delivery_front(ROOM, AGENT, None)
        .unwrap()
        .unwrap();
    assert_eq!(lead.sequence, timer.sequence);
    let batch = f.store.agent_delivery_batch(&lead, false, None).unwrap();
    assert_eq!(
        batch.iter().map(|event| event.sequence).collect::<Vec<_>>(),
        vec![first.sequence, second.sequence]
    );
}

#[test]
fn a_message_a_read_has_shown_is_no_longer_queued() {
    let f = Fixture::new();
    f.begin("p");
    let shown = message(&f, "a", false, false);
    let waiting = message(&f, "b", false, false);
    assert_eq!(queued(&f), vec![shown.sequence, waiting.sequence]);
    f.apply(Operation::MarkShown {
        room: ROOM.into(),
        agent: AGENT.into(),
        sequences: vec![shown.sequence],
    });
    assert_eq!(queued(&f), vec![waiting.sequence]);
    let released = f.store.agent_lifecycle(Operation::ReleaseQueuedMessage {
        room: ROOM.into(),
        agent: AGENT.into(),
        sequence: shown.sequence,
    });
    assert!(
        released.is_err(),
        "a message that was read cannot be sent again"
    );
}

#[test]
fn sending_a_queued_message_now_makes_it_urgent_and_the_front() {
    let f = Fixture::new();
    f.begin("p");
    let first = message(&f, "a", false, false);
    let timer = wake(&f, "t1");
    assert_eq!(front(&f), Some(timer.sequence));
    f.apply(Operation::ReleaseQueuedMessage {
        room: ROOM.into(),
        agent: AGENT.into(),
        sequence: first.sequence,
    });
    assert_eq!(
        front(&f),
        Some(first.sequence),
        "oldest first once it is urgent"
    );
    assert!(queued(&f).is_empty());
}

#[test]
fn cancelling_a_queued_message_expires_it_and_fails_the_reply_its_sender_waits_for() {
    let f = Fixture::new();
    f.begin("p");
    let plain = message(&f, "a", false, false);
    let asked = message(&f, "b", false, true);
    f.apply(Operation::CancelQueuedMessage {
        room: ROOM.into(),
        agent: AGENT.into(),
        sequence: plain.sequence,
        now: 5,
    });
    f.apply(Operation::CancelQueuedMessage {
        room: ROOM.into(),
        agent: AGENT.into(),
        sequence: asked.sequence,
        now: 6,
    });
    assert!(queued(&f).is_empty());
    assert_eq!(front(&f), None, "a cancelled message is never delivered");
    let rows = f.store.agent_inbox(ROOM, AGENT, 0).unwrap();
    assert!(rows.iter().all(|event| event.state == "expired"));
    assert!(f
        .task()
        .obligations
        .iter()
        .all(|obligation| obligation.status == "failed"));
}

#[test]
fn a_wake_is_admitted_past_the_queued_message_before_it_and_carries_the_ones_after() {
    let f = Fixture::new();
    f.begin("p");
    let before = message(&f, "a", false, false);
    let timer = wake(&f, "t1");
    let after = message(&f, "b", false, false);
    let lead = f
        .store
        .agent_work_delivery_front(ROOM, AGENT, None)
        .unwrap()
        .unwrap();
    assert_eq!(lead.sequence, timer.sequence);
    let batch = f.store.agent_delivery_batch(&lead, false, None).unwrap();
    f.apply(Operation::Attempt {
        room: ROOM.into(),
        agent: AGENT.into(),
        sequence: lead.sequence,
        prompt: format!("agent-event-{AGENT}-{}", lead.sequence),
        target: None,
        run: None,
        now: 10,
        work: None,
        through: batch.last().map_or(lead.sequence, |last| last.sequence),
    });
    let state = |sequence: u64| {
        f.store
            .agent_inbox_event(sequence)
            .unwrap()
            .map(|(event, _)| event.state)
            .unwrap()
    };
    assert_eq!(state(timer.sequence), "submitting", "the wake is delivered");
    assert_eq!(
        state(after.sequence),
        "submitting",
        "and carries the message behind it"
    );
    assert_eq!(
        state(before.sequence),
        "pending",
        "the one before it stays queued, to be read"
    );
    assert_eq!(queued(&f), vec![before.sequence]);
}
