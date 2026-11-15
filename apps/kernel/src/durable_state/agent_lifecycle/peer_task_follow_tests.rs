//! A coordinator follows one task of an agent in another room of the owner: the follow ends with
//! that task, so the coordinator's wait on it finishes once the outcome is handled.
use super::*;
use tests::Fixture;

const TASK: &str = "lane-task";

/// The lane's task in the other room.
fn lane_task(f: &Fixture) {
    f.apply(Operation::Begin {
        owner: "owner".into(),
        room: "peer-room".into(),
        agent: "lane".into(),
        prompt: TASK.into(),
        run: Some("lane-run".into()),
        now: 1,
    });
}

fn lane_task_ends(f: &Fixture) {
    f.apply(Operation::SourceOutcome {
        room: "peer-room".into(),
        source: TASK.into(),
        occurrence: format!("task-terminal-{TASK}"),
        success: true,
        public_answer: None,
        now: 4,
    });
}

fn follow_task(f: &Fixture) {
    f.apply(Operation::Subscribe {
        task: "p".into(),
        prompt: "p".into(),
        registration: Registration {
            id: "follow-task".into(),
            task_id: "p".into(),
            source_id: TASK.into(),
            obligation_id: None,
            source_cursor: 0,
            live: true,
            peer: true,
        },
    });
}

fn follow(f: &Fixture) -> Registration {
    f.store
        .agent_registration("follow-task")
        .unwrap()
        .expect("the follow is kept")
}

fn peer_events(f: &Fixture) -> Vec<InboxEvent> {
    f.store
        .agent_inbox("room", "parent", 0)
        .unwrap()
        .into_iter()
        .filter(|e| e.kind == "peer_event")
        .collect()
}

#[test]
fn a_followed_task_in_another_room_ends_the_follow_and_the_handled_wait() {
    let f = Fixture::new();
    lane_task(&f);
    f.begin("p");
    follow_task(&f);
    f.apply(Operation::Yield {
        task: "p".into(),
        prompt: "p".into(),
        registrations: vec!["follow-task".into()],
        cursor: 0,
        deadline: Some(60_000),
        reason: "lane result".into(),
        now: 2,
    });
    f.settle("p", true);
    assert_eq!(f.task().state, ExecutionState::Waiting);

    lane_task_ends(&f);
    let events = peer_events(&f);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].payload["event"], "turn_ended");
    assert!(
        !follow(&f).live,
        "a task ends once: its follow ends with it"
    );

    f.apply(Operation::Ack {
        room: "room".into(),
        agent: "parent".into(),
        sequence: events[0].sequence,
        handled: true,
        now: 5,
    });
    let task = f.task();
    assert_eq!(task.state, ExecutionState::Done, "{task:?}");
    assert_eq!(task.reason, RESOLVED_WAIT_REASON);
}

#[test]
fn a_task_that_ended_before_the_follow_is_reported_at_once() {
    let f = Fixture::new();
    lane_task(&f);
    lane_task_ends(&f);
    f.begin("p");
    follow_task(&f);
    assert_eq!(
        peer_events(&f).len(),
        1,
        "the outcome is in the lane's room"
    );
    assert!(!follow(&f).live);
}
