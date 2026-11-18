//! A spawn may outlive its parent's turn: the parent ends Done with the delegate still open, and
//! the child's first task is bound to it when its work message is finally delivered.
use super::*;
use tests::Fixture;

fn parent_done_with_an_accepted_delegate_and_a_queued_work_message(f: &Fixture) -> InboxEvent {
    f.begin("p");
    f.register();
    f.apply(Operation::DispatchReceipt {
        id: "obligation".into(),
        accepted: true,
        resource: Some("child".into()),
    });
    let Outcome::Event(message) = f.apply(Operation::Send {
        task: "p".into(),
        prompt: "p".into(),
        event: occurrence(
            "room",
            "child",
            "parent",
            "work-message",
            "message",
            serde_json::json!({"message":"do the delegated work"}),
        ),
    }) else {
        panic!("the work message is admitted")
    };
    let Outcome::Settled { task, correction } = f.settle("p", true) else {
        panic!("the turn settles")
    };
    assert!(!correction);
    assert_eq!(task.state, ExecutionState::Done);
    assert_eq!(task.obligations[0].status, "open");
    message
}

fn child_begins(f: &Fixture, prompt: &str) {
    f.apply(Operation::Begin {
        owner: "owner".into(),
        room: "room".into(),
        agent: "child".into(),
        prompt: prompt.into(),
        run: None,
        now: 4,
    });
}

#[test]
fn the_accepted_child_task_binds_to_a_parent_that_ended_its_turn() {
    let f = Fixture::new();
    parent_done_with_an_accepted_delegate_and_a_queued_work_message(&f);
    child_begins(&f, "child-task");
    f.apply(Operation::BindDelegate {
        parent_task: "p".into(),
        child_task: "child-task".into(),
    });
    assert_eq!(
        f.task().obligations[0].completion_task_id.as_deref(),
        Some("child-task")
    );
    // The bound child's end closes the parent's obligation.
    f.apply(Operation::SourceOutcome {
        room: "room".into(),
        source: "child-task".into(),
        occurrence: "task-terminal-child-task".into(),
        success: true,
        public_answer: None,
        now: 5,
    });
    let task = f.task();
    assert_eq!(task.state, ExecutionState::Done);
    assert!(task.obligations.iter().all(|o| !o.requires_resolution()));
}

#[test]
fn a_work_message_delivered_after_the_parents_turn_binds_its_child_task() {
    let f = Fixture::new();
    let message = parent_done_with_an_accepted_delegate_and_a_queued_work_message(&f);
    f.apply(Operation::Attempt {
        room: "room".into(),
        agent: "child".into(),
        sequence: message.sequence,
        prompt: "child-message-task".into(),
        target: None,
        run: None,
        now: 4,
        work: None,
        through: 0,
    });
    child_begins(&f, "child-message-task");
    f.apply(Operation::Receipt {
        room: "room".into(),
        agent: "child".into(),
        sequence: message.sequence,
        state: "accepted".into(),
        now: 5,
    });
    assert_eq!(
        f.task().obligations[0].completion_task_id.as_deref(),
        Some("child-message-task")
    );
}
