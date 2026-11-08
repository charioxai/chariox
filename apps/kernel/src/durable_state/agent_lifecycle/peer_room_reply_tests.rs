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
