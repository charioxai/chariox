//! MP-08 / MP-09 / MP-10 / MP-11: a current worker can refresh its owner action.
use super::*;
use tests::Fixture;

#[test]
fn blocked_intent_refresh_after_handled_orphan_preserves_owner_control() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    f.subscribe();
    let Outcome::Task(blocked) = f.apply(Operation::Block {
        task: "p".into(),
        prompt: "p".into(),
        reason: "disk is full".into(),
    }) else {
        panic!("task")
    };
    f.apply(Operation::SourceOutcome {
        room: "room".into(),
        source: "child".into(),
        occurrence: "gone".into(),
        success: false,
        public_answer: Some(
            serde_json::json!({"kind":"delegate_gone","delegate_id":"child","reason":"deleted"}),
        ),
        now: 5,
    });
    let event = f.store.agent_inbox("room", "parent", 0).unwrap().remove(0);
    f.apply(Operation::Ack {
        room: "room".into(),
        agent: "parent".into(),
        sequence: event.sequence,
        handled: true,
        now: 6,
    });
    let result = f.store.agent_lifecycle(Operation::Block {
        task: "p".into(),
        prompt: "p".into(),
        reason: "disk recovered; coordinator must resume the task".into(),
    });
    assert!(
        result.is_ok(),
        "the current blocked worker must be able to update its reason: {result:?}"
    );
    let next = f.task();
    assert_eq!(next.state, ExecutionState::Blocked);
    assert_eq!(next.blocked_revision, blocked.blocked_revision);
    assert_eq!(
        next.obligations[0].status, "failed",
        "handling cannot override owner Resume"
    );
    assert!(next.reason.starts_with("disk recovered"));
    f.apply(Operation::Block {
        task: "p".into(),
        prompt: "p".into(),
        reason: next.reason.clone(),
    });
    assert_eq!(f.task(), next, "an unchanged owner action is a no-op");
    let reopened = DurableKernelStateStore::open(f.root.join("state.sqlite")).unwrap();
    assert_eq!(
        reopened.agent_tasks(Some("room"), Some("parent")).unwrap()[0],
        next
    );
    f.apply(Operation::OwnerResponse {
        task: "p".into(),
        revision: blocked.blocked_revision,
        resume: true,
        now: 7,
    });
    f.apply(Operation::Ack {
        room: "room".into(),
        agent: "parent".into(),
        sequence: event.sequence,
        handled: true,
        now: 8,
    });
    assert_eq!(f.task().obligations[0].status, "satisfied");
}

#[test]
fn blocked_intent_refresh_never_revives_a_closed_or_foreign_turn() {
    let f = Fixture::new();
    f.begin("p");
    f.apply(Operation::Block {
        task: "p".into(),
        prompt: "p".into(),
        reason: "owner action".into(),
    });
    let before = f.task();
    assert!(f
        .store
        .agent_lifecycle(Operation::Block {
            task: "p".into(),
            prompt: "other".into(),
            reason: "changed".into()
        })
        .is_err());
    assert_eq!(f.task(), before);
    f.apply(Operation::CancelTask {
        task: "p".into(),
        owner: "owner".into(),
        revision: before.revision,
    });
    let before = f.task();
    let error = f
        .store
        .agent_lifecycle(Operation::Block {
            task: "p".into(),
            prompt: "p".into(),
            reason: "changed".into(),
        })
        .unwrap_err();
    assert!(
        error.to_string().contains("new prompt"),
        "state refusal must give a recovery action: {error}"
    );
    assert_eq!(f.task(), before);
}
