//! An agent may always end its turn: an open watched process or delegate never costs a correction
//! or a blocked card. The task ends Done and the open source stays supervised: its end reaches
//! the agent as a new event.
use super::*;

fn settle(f: &Fixture) -> AgentTaskExecution {
    let Outcome::Settled { task, correction } = f.apply(Operation::Settle {
        room: "room".into(),
        agent: "agent".into(),
        prompt: "p".into(),
        run: "run".into(),
        has_answer: true,
        cancelled: false,
        now: 30,
    }) else {
        panic!("the turn did not settle")
    };
    assert!(!correction, "ending the turn is never corrected");
    task
}

fn running_process(f: &Fixture) {
    f.create("build", "process", None, None);
    f.apply(Operation::ProcessStarted {
        id: "build".into(),
        pid: 42,
        now: 20,
    });
}

fn exit_process(f: &Fixture) {
    f.apply(Operation::ProcessExited {
        id: "build".into(),
        exit_code: Some(0),
        tail: "built".into(),
        now: 50,
    });
}

#[test]
fn a_turn_that_ends_with_an_open_process_is_done_and_the_process_stays_running() {
    let f = Fixture::new();
    running_process(&f);
    let task = settle(&f);
    assert_eq!(task.state, ExecutionState::Done);
    assert_eq!(f.wake("build").state, "running");
    assert!(
        f.inbox()
            .iter()
            .all(|e| !crate::durable_state::agent_lifecycle::is_open(e)),
        "ending the turn tells the agent nothing"
    );
}

#[test]
fn the_end_of_a_process_left_open_reaches_the_agent_as_an_event_to_deliver() {
    let f = Fixture::new();
    running_process(&f);
    settle(&f);
    exit_process(&f);
    let events = f
        .inbox()
        .into_iter()
        .filter(crate::durable_state::agent_lifecycle::is_open)
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].kind, "source_completed");
    assert_eq!(events[0].payload["success"], true);
    assert!(
        events[0].payload["after_turn"] == true,
        "the event names that the turn that started the process had ended: {:?}",
        events[0].payload
    );
    assert_eq!(f.task().state, ExecutionState::Done);
    assert!(f
        .task()
        .obligations
        .iter()
        .all(|o| !o.requires_resolution()));

    // The woken turn reads the event and acknowledges it like any other.
    f.apply(Operation::Ack {
        room: "room".into(),
        agent: "agent".into(),
        sequence: events[0].sequence,
        handled: true,
        now: 60,
    });
    assert!(f
        .inbox()
        .iter()
        .all(|e| !crate::durable_state::agent_lifecycle::is_open(e)));
}

fn open_delegate(f: &Fixture) {
    f.apply(Operation::RegisterObligation {
        owner: "owner".into(),
        room: "room".into(),
        agent: "agent".into(),
        prompt: "p".into(),
        run: Some("run".into()),
        id: "helper-work".into(),
        kind: "delegate".into(),
        resource: Some("helper".into()),
        now: 5,
    });
    f.apply(Operation::Subscribe {
        task: "p".into(),
        prompt: "p".into(),
        registration: Registration {
            id: "helper-reg".into(),
            task_id: "p".into(),
            source_id: "helper".into(),
            obligation_id: Some("helper-work".into()),
            source_cursor: 0,
            live: true,
            peer: false,
        },
    });
}

#[test]
fn a_turn_that_ends_with_an_open_delegate_is_done_and_its_end_closes_the_obligation() {
    let f = Fixture::new();
    open_delegate(&f);
    assert_eq!(settle(&f).state, ExecutionState::Done);
    assert_eq!(f.task().obligations[0].status, "open");

    // The helper's end reaches the agent as its child_finished; the ledger only closes the work.
    f.apply(Operation::SourceOutcome {
        room: "room".into(),
        source: "helper".into(),
        occurrence: "terminal".into(),
        success: true,
        public_answer: None,
        now: 50,
    });
    let task = f.task();
    assert_eq!(task.state, ExecutionState::Done);
    assert!(task.obligations.iter().all(|o| !o.requires_resolution()));
}

#[test]
fn an_open_process_and_an_open_delegate_do_not_hold_a_turn_that_also_waits() {
    let f = Fixture::new();
    running_process(&f);
    open_delegate(&f);
    f.create("waited", "timer", Some(1_000), None);
    f.wait_on(&["waited"]);
    assert_eq!(f.task().state, ExecutionState::Waiting);
    assert_eq!(f.wake("build").state, "running");
}

#[test]
fn a_failed_process_left_open_reaches_the_agent_as_a_lost_source() {
    let f = Fixture::new();
    running_process(&f);
    settle(&f);
    f.apply(Operation::ProcessExited {
        id: "build".into(),
        exit_code: None,
        tail: String::new(),
        now: 50,
    });
    let events = f
        .inbox()
        .into_iter()
        .filter(crate::durable_state::agent_lifecycle::is_open)
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].kind, "source_lost");
    assert_eq!(events[0].payload["after_turn"], true);
}

#[test]
fn a_process_output_match_after_the_turn_ended_reaches_the_agent() {
    let f = Fixture::new();
    running_process(&f);
    settle(&f);
    f.apply(Operation::ProcessMatched {
        id: "build".into(),
        line: "ready".into(),
        now: 40,
    });
    let events = f
        .inbox()
        .into_iter()
        .filter(crate::durable_state::agent_lifecycle::is_open)
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].kind, "process_output_matched");
    assert_eq!(events[0].payload["after_turn"], true);
}
