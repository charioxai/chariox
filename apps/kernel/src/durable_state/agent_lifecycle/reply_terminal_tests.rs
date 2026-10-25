//! MP-08 / MP-09 / MP-10 / MP-11: terminal replies survive a writer restart.
use super::*;
use tests::Fixture;

fn send(f: &Fixture, key: &str) -> InboxEvent {
    let Outcome::Event(event) = f.apply(Operation::Send {
        task: "p".into(),
        prompt: "p".into(),
        event: InboxEvent {
            sequence: 0,
            room_id: "room".into(),
            agent_id: "child".into(),
            source_id: "parent".into(),
            occurrence_id: key.into(),
            kind: "message".into(),
            payload: serde_json::json!({"message":"return findings"}),
            urgent: false,
            reply_requested: true,
            state: "pending".into(),
            prompt_id: None,
            target_prompt_id: None,
            provider_run_id: None,
            attempted_at_ms: None,
            submit_epoch: None,
        },
    }) else {
        panic!("event")
    };
    event
}

#[test]
fn source_lost_reply_survives_writer_restart_before_delete_once() {
    let mut f = Fixture::new();
    f.begin("p");
    send(&f, "first");
    send(&f, "second");
    f.apply(Operation::SourceOutcome {
        room: "room".into(),
        source: "child".into(),
        occurrence: "lost".into(),
        success: false,
        public_answer: None,
        now: 2,
    });
    let before = f.store.agent_inbox("room", "parent", 0).unwrap();
    assert_eq!(before.len(), 2);
    assert!(before
        .iter()
        .all(|e| e.payload["public_answer"] == DelegateGoneReason::SourceLost.answer("child")));
    // Drop the owner lease and stop the old writer before opening the same database.
    let temporary = DurableKernelStateStore::open_owned(f.root.join("temporary.sqlite")).unwrap();
    drop(std::mem::replace(&mut f.store, temporary));
    f.store = DurableKernelStateStore::open_owned(f.root.join("state.sqlite")).unwrap();
    for _ in 0..2 {
        f.apply(Operation::RetireWakes {
            room: "room".into(),
            agent: Some("child".into()),
            now: 3,
        });
    }
    assert_eq!(f.store.agent_inbox("room", "parent", 0).unwrap(), before);
    assert!(f
        .store
        .agent_registrations("p")
        .unwrap()
        .iter()
        .all(|r| !r.live));
    assert!(f.task().obligations.iter().all(|o| o.status == "failed"));
}

#[test]
fn deleting_recipient_preserves_successful_reply_and_settles_the_other_request() {
    let f = Fixture::new();
    f.begin("p");
    let first = send(&f, "first");
    send(&f, "second");
    f.apply(Operation::SourceOutcome {
        room: "room".into(),
        source: format!("agent-event-child-{}", first.sequence),
        occurrence: "successful-reply".into(),
        success: true,
        public_answer: Some(serde_json::json!({"answer":"PROBE"})),
        now: 2,
    });
    for _ in 0..2 {
        f.apply(Operation::RetireWakes {
            room: "room".into(),
            agent: Some("child".into()),
            now: 3,
        });
    }
    let events = f.store.agent_inbox("room", "parent", 0).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].kind, "source_completed");
    assert_eq!(events[0].payload["public_answer"]["answer"], "PROBE");
    assert_eq!(
        events[1].payload["public_answer"],
        DelegateGoneReason::Deleted.answer("child")
    );
}

#[test]
fn failed_reply_start_does_not_complete_the_owned_delegate() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    f.subscribe();
    send(&f, "requested-reply");
    f.apply(Operation::DelegateUnavailable {
        room: "room".into(),
        agent: "child".into(),
        reason: DelegateGoneReason::FailedStart,
        now: 2,
    });
    let task = f.task();
    assert_eq!(
        task.obligations
            .iter()
            .find(|o| o.kind == "delegate")
            .unwrap()
            .status,
        "open"
    );
    assert_eq!(
        task.obligations
            .iter()
            .find(|o| o.kind == "reply")
            .unwrap()
            .status,
        "failed"
    );
    assert!(f
        .store
        .agent_registrations("p")
        .unwrap()
        .iter()
        .any(|r| r.source_id == "child" && r.live));
    let events = f.store.agent_inbox("room", "parent", 0).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].payload["public_answer"],
        DelegateGoneReason::FailedStart.answer("child")
    );
}
