//! A message that starts a turn on a waiting agent starts it beside the retained task. The wait's
//! outcome handled inside that turn must not end the retained task, and so the delegate its parent
//! holds, while the agent is still working.
use super::*;
use tests::Fixture;

fn task_of(f: &Fixture, id: &str) -> AgentTaskExecution {
    f.store
        .agent_tasks(Some("room"), Some("parent"))
        .unwrap()
        .into_iter()
        .find(|t| t.task_id == id)
        .unwrap()
}

fn sweep(f: &Fixture, now: u64) {
    f.apply(Operation::Sweep {
        now,
        vault_wait_recipients: Vec::new(),
        busy_recipients: Vec::new(),
        held_work: Vec::new(),
    });
}

#[test]
fn a_wait_handled_in_a_steer_started_turn_ends_with_that_turn() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    f.subscribe();
    f.apply(Operation::Yield {
        task: "p".into(),
        prompt: "p".into(),
        registrations: vec!["reg".into()],
        cursor: 0,
        deadline: Some(60_000),
        reason: "child result".into(),
        now: 2,
    });
    f.settle("p", true);
    assert_eq!(task_of(&f, "p").state, ExecutionState::Waiting);

    // The parent's urgent message starts a turn of its own while "p" waits.
    f.apply(Operation::Begin {
        owner: "owner".into(),
        room: "room".into(),
        agent: "parent".into(),
        prompt: "steer".into(),
        run: Some("run".into()),
        now: 4,
    });
    f.apply(Operation::SourceOutcome {
        room: "room".into(),
        source: "child".into(),
        occurrence: "completed".into(),
        success: true,
        public_answer: None,
        now: 5,
    });
    let events = f.store.agent_inbox("room", "parent", 0).unwrap();
    for event in events.iter().filter(|e| e.kind == "source_completed") {
        f.apply(Operation::Ack {
            room: "room".into(),
            agent: "parent".into(),
            sequence: event.sequence,
            handled: true,
            now: 6,
        });
    }
    sweep(&f, 7);
    let waiting = task_of(&f, "p");
    assert_eq!(
        waiting.state,
        ExecutionState::Waiting,
        "the agent still works in its steer-started turn: {waiting:?}"
    );

    f.apply(Operation::Settle {
        room: "room".into(),
        agent: "parent".into(),
        prompt: "steer".into(),
        run: "run".into(),
        has_answer: true,
        cancelled: false,
        now: 8,
    });
    let finished = task_of(&f, "p");
    assert_eq!(finished.state, ExecutionState::Done, "{finished:?}");
    assert_eq!(finished.reason, RESOLVED_WAIT_REASON);
}
