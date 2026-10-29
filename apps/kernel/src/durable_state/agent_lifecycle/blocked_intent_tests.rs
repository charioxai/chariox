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

#[test]
fn restart_failure_retirement_keeps_owned_work_and_requires_current_revision() {
    for retained_work in ["none", "obligation", "goal", "registration", "stale"] {
        let f = Fixture::new();
        f.begin("p");
        match retained_work {
            "obligation" => f.register(),
            "goal" => {
                f.apply(Operation::StartGoal {
                    task: "p".into(),
                    owner: "owner".into(),
                    objective: "Finish work".into(),
                    minutes: 5,
                    now: 2,
                });
            }
            "registration" => {
                f.apply(Operation::Subscribe {
                    task: "p".into(),
                    prompt: "p".into(),
                    registration: Registration {
                        id: "reg".into(),
                        task_id: "p".into(),
                        source_id: "source".into(),
                        obligation_id: None,
                        source_cursor: 0,
                        live: true,
                    },
                });
            }
            _ => {}
        }
        f.apply(Operation::Block {
            task: "p".into(),
            prompt: "p".into(),
            reason: "restart recovery failed".into(),
        });
        let before = f.task();
        f.apply(Operation::RetireRestartFailure {
            task: "p".into(),
            revision: before.blocked_revision + u64::from(retained_work == "stale"),
            now: 3,
        });
        let after = f.task();
        if retained_work == "none" {
            assert_eq!(after.state, ExecutionState::Cancelled);
        } else {
            assert_eq!(after, before, "{retained_work} must remain supervised");
        }
        if retained_work == "registration" {
            assert!(f.store.agent_registrations("p").unwrap()[0].live);
        }
    }
}

#[test]
fn restart_failure_retirement_keeps_pending_deliveries() {
    let f = Fixture::new();
    f.begin("p");
    f.apply(Operation::Occur(InboxEvent {
        sequence: 0,
        room_id: "room".into(),
        agent_id: "parent".into(),
        source_id: "source".into(),
        occurrence_id: "pending".into(),
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
    }));
    f.apply(Operation::Block {
        task: "p".into(),
        prompt: "p".into(),
        reason: "restart recovery failed".into(),
    });
    let before = f.task();
    let inbox = f.store.agent_inbox("room", "parent", 0).unwrap();
    f.apply(Operation::RetireRestartFailure {
        task: "p".into(),
        revision: before.blocked_revision,
        now: 3,
    });
    assert_eq!(f.task(), before);
    assert_eq!(f.store.agent_inbox("room", "parent", 0).unwrap(), inbox);
}
