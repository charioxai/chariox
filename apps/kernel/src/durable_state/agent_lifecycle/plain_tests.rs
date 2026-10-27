//! A plain turn (no goal, obligation or wait) only gets a task row: it is never
//! corrected, held to a wait or blocked for the owner.
use super::tests::Fixture;
use super::*;

#[test]
fn plain_turn_end_is_done_without_a_correction() {
    for has_answer in [true, false] {
        let f = Fixture::new();
        f.begin("p");
        let Outcome::Settled { task, correction } = f.settle("p", has_answer) else {
            panic!()
        };
        assert!(!correction, "answer={has_answer}");
        assert_eq!(task.state, ExecutionState::Done, "answer={has_answer}");
        assert!(task.pending_prompt_id.is_none() && !task.correction_used);
        let again = f.settle("p", has_answer);
        assert!(matches!(
            again,
            Outcome::Settled {
                correction: false,
                ..
            }
        ));
        assert_eq!(f.task().state, ExecutionState::Done);
    }
}

#[test]
fn plain_failed_turn_is_cancelled_not_blocked() {
    let f = Fixture::new();
    f.begin("p");
    end_failed_turn(&f.store, &f.task(), "Provider run failed".into()).unwrap();
    let task = f.task();
    assert_eq!(task.state, ExecutionState::Cancelled, "{task:?}");
    assert!(task.reason.contains("Provider run failed"));
}

#[test]
fn failed_turn_with_ledger_work_still_blocks_for_the_owner() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    end_failed_turn(&f.store, &f.task(), "Provider run failed".into()).unwrap();
    assert_eq!(f.task().state, ExecutionState::Blocked);
}

#[test]
fn turn_with_a_registered_obligation_keeps_the_ledger_correction() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    let Outcome::Settled { task, correction } = f.settle("p", true) else {
        panic!()
    };
    assert!(correction);
    assert_ne!(task.state, ExecutionState::Done);
}

#[test]
fn agent_archive_cancels_parent_participation_without_cancelling_child_work() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    let Outcome::Task(child) = f.apply(Operation::Begin {
        owner: "owner".into(),
        room: "room".into(),
        agent: "child".into(),
        prompt: "child-turn".into(),
        run: Some("child-run".into()),
        now: 2,
    }) else {
        panic!("child task")
    };
    let parent = f.task();
    let Outcome::Task(archived) = f.apply(Operation::ArchiveTask {
        task: parent.task_id,
        owner: parent.owner_user_id,
        revision: parent.revision,
    }) else {
        panic!("archived task")
    };
    assert_eq!(archived.state, ExecutionState::Cancelled);
    assert!(archived.reason.contains("Agent archived"));
    assert!(archived.stopped_by_owner());
    assert!(archived
        .obligations
        .iter()
        .all(|o| o.kind != "delegate" || o.dispatch_state != "cancel_requested"));
    let retained = f.store.agent_tasks(Some("room"), Some("child")).unwrap();
    assert_eq!(retained[0], child);
}

#[test]
fn agent_archive_cancels_handoff_and_late_completion_does_not_reopen_task() {
    let f = Fixture::new();
    f.begin("p");
    f.apply(Operation::RegisterObligation {
        owner: "owner".into(),
        room: "room".into(),
        agent: "parent".into(),
        prompt: "p".into(),
        run: Some("run".into()),
        id: "handoff".into(),
        kind: "hand_off".into(),
        resource: Some("handoff-source".into()),
        now: 1,
    });
    f.apply(Operation::DispatchReceipt {
        id: "handoff".into(),
        accepted: true,
        resource: Some("handoff-source".into()),
    });
    let task = f.task();
    f.apply(Operation::ArchiveTask {
        task: task.task_id,
        owner: task.owner_user_id,
        revision: task.revision,
    });
    f.apply(Operation::SourceOutcome {
        public_answer: None,
        room: "room".into(),
        source: "handoff-source".into(),
        occurrence: "late".into(),
        success: true,
        now: 3,
    });
    let task = f.task();
    assert!(task.stopped_by_owner());
    assert!(task.obligations.iter().all(|o| o.status == "cancelled"));
    assert!(f
        .store
        .agent_registrations("p")
        .unwrap()
        .iter()
        .all(|r| !r.live));
}
