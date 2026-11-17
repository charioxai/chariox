//! A delivery refused before any provider I/O: who is told, and how the owner's Resume sends it again.
use super::*;
use tests::Fixture;

/// A message from "parent" in room "home" to "child" in room "work", refused by the recipient's
/// worker, then swept after the delivery window. Returns the failed event and the prompt id the
/// worker rejected.
fn refused_and_swept(f: &Fixture) -> (InboxEvent, String) {
    f.apply(Operation::Begin {
        owner: "owner".into(),
        room: "home".into(),
        agent: "parent".into(),
        prompt: "p".into(),
        run: Some("run".into()),
        now: 1,
    });
    let mut message = occurrence(
        "work",
        "child",
        "parent",
        "cross-room",
        "message",
        serde_json::json!({"message": "hello", "sender_room_id": "home", "no_reply": true}),
    );
    message.urgent = true;
    let Outcome::Event(sent) = f.apply(Operation::Send {
        task: "p".into(),
        prompt: "p".into(),
        event: message,
    }) else {
        panic!("event")
    };
    let prompt = sent.transport_prompt_id(true);
    let now = crate::session::unix_epoch_ms();
    f.apply(Operation::Attempt {
        room: "work".into(),
        agent: "child".into(),
        sequence: sent.sequence,
        prompt: prompt.clone(),
        target: None,
        run: None,
        now,
        work: None,
        through: sent.sequence,
    });
    f.apply(Operation::Receipt {
        room: "work".into(),
        agent: "child".into(),
        sequence: sent.sequence,
        state: "rejected".into(),
        now,
    });
    f.apply(Operation::Sweep {
        now: now + DELIVERY_TIMEOUT_MS + 1_000,
        busy_recipients: vec![],
        vault_wait_recipients: vec![],
        held_work: vec![],
    });
    let failed = f.store.agent_inbox("work", "child", 0).unwrap().remove(0);
    assert_eq!(failed.state, "failed");
    (failed, prompt)
}

#[test]
fn a_cross_room_message_that_could_not_start_tells_its_sender_in_the_senders_room() {
    let f = Fixture::new();
    let (failed, _) = refused_and_swept(&f);
    let told = f.store.agent_inbox("home", "parent", 0).unwrap();
    assert!(
        told.iter().any(|e| e.kind == "source_lost"
            && e.source_id == format!("agent-event-child-{}", failed.sequence)),
        "the sender is told in its own room: {told:?}"
    );
    assert!(
        f.store.agent_inbox("work", "parent", 0).unwrap().is_empty(),
        "nothing is left in the recipient's room for the sender"
    );
}

#[test]
fn a_leased_delivery_the_owner_resumes_goes_out_under_a_new_prompt_id() {
    let f = Fixture::new();
    let (_, rejected) = refused_and_swept(&f);
    let card = f
        .store
        .agent_tasks(Some("work"), Some("child"))
        .unwrap()
        .into_iter()
        .find(|t| t.task_id.starts_with("delivery-"))
        .expect("the owner is asked about the undelivered message");
    f.apply(Operation::OwnerResponse {
        task: card.task_id.clone(),
        revision: card.blocked_revision,
        resume: true,
        now: crate::session::unix_epoch_ms(),
    });
    let resumed = f.store.agent_inbox("work", "child", 0).unwrap().remove(0);
    assert_eq!(resumed.state, "pending");
    assert_ne!(
        resumed.transport_prompt_id(true),
        rejected,
        "the worker keeps the rejection receipt of the first prompt id"
    );
    assert_eq!(
        resumed.transport_prompt_id(false),
        format!("agent-event-child-{}", resumed.sequence),
        "a local delivery has no worker fence and keeps the plain id"
    );
}
