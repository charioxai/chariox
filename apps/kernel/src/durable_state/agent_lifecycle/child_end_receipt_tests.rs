//! A child's end is told once, by its `child_finished`; the receipt of the same end that follows
//! arrives already handled. When the parent handled the `child_finished` before that receipt, the
//! receipt still settles the delegate obligation it reports, so the parent's task can end.
use super::*;
use tests::Fixture;

#[test]
fn a_receipt_after_the_handled_child_finished_settles_the_delegate() {
    let f = Fixture::new();
    f.begin("p");
    f.apply(Operation::RegisterObligation {
        owner: "owner".into(),
        room: "room".into(),
        agent: "parent".into(),
        prompt: "p".into(),
        run: Some("run".into()),
        id: "delegate".into(),
        kind: "delegate".into(),
        resource: Some("child".into()),
        now: 1,
    });
    f.apply(Operation::DispatchReceipt {
        id: "delegate".into(),
        accepted: true,
        resource: Some("child".into()),
    });
    f.apply(Operation::Begin {
        owner: "owner".into(),
        room: "room".into(),
        agent: "child".into(),
        prompt: "ct".into(),
        run: None,
        now: 2,
    });
    f.apply(Operation::BindDelegate {
        parent_task: "p".into(),
        child_task: "ct".into(),
    });
    // A deadline stops the child: the parent hears child_finished first and handles it at once.
    let Outcome::Event(finished) = f.apply(Operation::Occur(occurrence(
        "room",
        "parent",
        "child",
        "child-finished-ct",
        "child_finished",
        serde_json::json!({"child_task_id": "ct", "success": false, "deadline_reached": true}),
    ))) else {
        panic!("child_finished recorded")
    };
    f.apply(Operation::Ack {
        room: "room".into(),
        agent: "parent".into(),
        sequence: finished.sequence,
        handled: true,
        now: 3,
    });
    // Then the receipt of the child's task end lands, folded into that child_finished.
    f.apply(Operation::SourceOutcome {
        room: "room".into(),
        source: "ct".into(),
        occurrence: "task-terminal-ct".into(),
        success: false,
        public_answer: None,
        now: 4,
    });
    let delegate = f
        .task()
        .obligations
        .into_iter()
        .find(|o| o.id == "delegate")
        .unwrap();
    assert_eq!(delegate.status, "satisfied", "{delegate:?}");
    let Outcome::Settled { task, correction } = f.settle("p", true) else {
        panic!("the turn settles")
    };
    assert!(!correction, "nothing is left for the parent to resolve");
    assert_eq!(task.state, ExecutionState::Done, "{task:?}");
}
