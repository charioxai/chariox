//! A message to an agent of another room of the same owner lives in the recipient's room; its
//! requested reply settles the sender's task in the sender's room.
use super::*;
use tests::Fixture;

fn send_to_peer_room(f: &Fixture, key: &str, sender_room: &str) -> Result<Outcome, DaemonError> {
    f.store.agent_lifecycle(Operation::Send {
        task: "p".into(),
        prompt: "p".into(),
        event: InboxEvent {
            sequence: 0,
            room_id: "peer-room".into(),
            agent_id: "peer".into(),
            source_id: "parent".into(),
            occurrence_id: key.into(),
            kind: "message".into(),
            payload: serde_json::json!({"message":"compare notes","sender_room_id":sender_room}),
            urgent: false,
            reply_requested: true,
            state: "pending".into(),
            prompt_id: None,
            target_prompt_id: None,
            provider_run_id: None,
            attempted_at_ms: None,
            submit_epoch: None,
        },
    })
}

#[test]
fn reply_from_another_room_settles_the_senders_task() {
    let f = Fixture::new();
    f.begin("p");
    let Outcome::Event(event) = send_to_peer_room(&f, "first", "room").unwrap() else {
        panic!("event")
    };
    assert_eq!(event.room_id, "peer-room");
    assert_eq!(
        f.store.agent_inbox("peer-room", "peer", 0).unwrap().len(),
        1
    );
    f.apply(Operation::SourceOutcome {
        room: "peer-room".into(),
        source: format!("agent-event-peer-{}", event.sequence),
        occurrence: "done".into(),
        success: true,
        public_answer: Some(serde_json::json!({"text":"notes compared"})),
        now: 2,
    });
    let replies = f.store.agent_inbox("room", "parent", 0).unwrap();
    assert_eq!(replies.len(), 1, "the reply reaches the sender's own room");
    assert_eq!(replies[0].kind, "source_completed");
    assert_eq!(
        replies[0].payload["public_answer"]["text"],
        "notes compared"
    );
    assert!(f.task().obligations.iter().all(|o| o.status != "open"));
}

#[test]
fn lost_peer_in_another_room_settles_the_senders_requested_reply() {
    let f = Fixture::new();
    f.begin("p");
    send_to_peer_room(&f, "first", "room").unwrap();
    f.apply(Operation::SourceOutcome {
        room: "peer-room".into(),
        source: "peer".into(),
        occurrence: "lost".into(),
        success: false,
        public_answer: None,
        now: 2,
    });
    let replies = f.store.agent_inbox("room", "parent", 0).unwrap();
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0].kind, "source_lost");
    assert!(f.task().obligations.iter().all(|o| o.status == "failed"));
}

#[test]
fn a_message_cannot_claim_a_room_its_sender_is_not_in() {
    let f = Fixture::new();
    f.begin("p");
    assert!(send_to_peer_room(&f, "first", "peer-room").is_err());
    assert!(f
        .store
        .agent_inbox("peer-room", "peer", 0)
        .unwrap()
        .is_empty());
}

fn follow(f: &Fixture, id: &str, peer: bool) {
    f.apply(Operation::Subscribe {
        task: "p".into(),
        prompt: "p".into(),
        registration: Registration {
            id: id.into(),
            task_id: "p".into(),
            source_id: "peer".into(),
            obligation_id: None,
            source_cursor: 0,
            live: true,
            peer,
        },
    });
}

#[test]
fn a_peer_followed_from_another_room_ends_as_one_peer_event() {
    let f = Fixture::new();
    f.begin("p");
    follow(&f, "follow-peer", true);
    f.apply(Operation::SourceOutcome {
        room: "peer-room".into(),
        source: "peer".into(),
        occurrence: "turn-1".into(),
        success: true,
        public_answer: Some(serde_json::json!({"text":"private notes"})),
        now: 2,
    });
    let events = f.store.agent_inbox("room", "parent", 0).unwrap();
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].kind, "peer_event");
    assert_eq!(events[0].payload["event"], "turn_ended");
    assert!(events[0].payload.get("public_answer").is_none());
}

#[test]
fn only_a_peer_registration_reaches_across_rooms() {
    let f = Fixture::new();
    f.begin("p");
    follow(&f, "same-room-only", false);
    f.apply(Operation::SourceOutcome {
        room: "peer-room".into(),
        source: "peer".into(),
        occurrence: "turn-1".into(),
        success: true,
        public_answer: None,
        now: 2,
    });
    assert!(f.store.agent_inbox("room", "parent", 0).unwrap().is_empty());
}

#[test]
fn a_sender_still_working_in_another_room_does_not_hold_the_recipients_receipt() {
    let f = Fixture::new();
    f.begin("p");
    let Outcome::Event(event) = send_to_peer_room(&f, "first", "room").unwrap() else {
        panic!("event")
    };
    let prompt = format!("agent-event-peer-{}", event.sequence);
    f.apply(Operation::Begin {
        owner: "owner".into(),
        room: "peer-room".into(),
        agent: "peer".into(),
        prompt: prompt.clone(),
        run: Some("peer-run".into()),
        now: 2,
    });
    f.apply(Operation::Attempt {
        room: "peer-room".into(),
        agent: "peer".into(),
        sequence: event.sequence,
        prompt,
        target: None,
        run: None,
        now: 2,
        work: None,
        through: 0,
    });
    let receipt = f.store.agent_lifecycle(Operation::Receipt {
        room: "peer-room".into(),
        agent: "peer".into(),
        sequence: event.sequence,
        state: "accepted".into(),
        now: 3,
    });
    assert!(receipt.is_ok(), "{receipt:?}");
    assert_eq!(
        f.store.agent_inbox("peer-room", "peer", 0).unwrap()[0].state,
        "accepted"
    );
}
