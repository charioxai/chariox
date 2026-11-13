//! The owner ends a wait from the agent's pane: the sources stop, nobody is woken, and the task
//! is no longer registered for the wait.
use super::*;

fn cancel(f: &Fixture, owner: &str) -> Result<Vec<AgentWake>, DaemonError> {
    let started_at_ms = f.task().wait.expect("the task waits").started_at_ms;
    f.store
        .agent_lifecycle(Operation::CancelOwnerWait {
            task: "p".into(),
            room: "room".into(),
            owner: owner.into(),
            started_at_ms,
        })
        .map(|outcome| match outcome {
            Outcome::Wakes(wakes) => wakes,
            other => panic!("unexpected {other:?}"),
        })
}

fn live_registrations(f: &Fixture) -> usize {
    f.store
        .agent_registrations("p")
        .unwrap()
        .iter()
        .filter(|registration| registration.live)
        .count()
}

#[test]
fn cancelling_a_timer_wait_stops_the_timer_without_waking_the_agent() {
    let f = Fixture::new();
    f.create("t1", "timer", Some(5_000_000), None);
    f.wait_on(&["t1"]);
    let wakes = cancel(&f, "owner").unwrap();
    assert_eq!(wakes.len(), 1);
    assert_eq!(f.wake("t1").state, "cancelled");
    assert_eq!(
        live_registrations(&f),
        0,
        "the agent is no longer registered"
    );
    let task = f.task();
    assert_eq!(task.state, ExecutionState::Done);
    assert!(task.wait.is_none());
    assert!(
        task.obligations.iter().all(|o| !o.requires_resolution()),
        "{:?}",
        task.obligations
    );
    assert!(
        f.inbox().is_empty(),
        "cancelling is not a wake: {:?}",
        f.inbox()
    );
    f.apply(Operation::FireWakes { now: 9_000_000 });
    assert!(f.inbox().is_empty(), "the stopped timer never fires");
}

#[test]
fn cancelling_a_process_wait_waits_for_the_exit_and_still_wakes_nobody() {
    let f = Fixture::new();
    f.create("proc", "process", None, None);
    f.apply(Operation::ProcessStarted {
        id: "proc".into(),
        pid: 42,
        now: 20,
    });
    f.wait_on(&["proc"]);
    let wakes = cancel(&f, "owner").unwrap();
    assert_eq!(
        wakes[0].state, "cancelling",
        "the supervisor still has to stop the process"
    );
    assert_eq!(f.task().state, ExecutionState::Done);
    f.apply(Operation::ProcessExited {
        id: "proc".into(),
        exit_code: Some(143),
        tail: String::new(),
        now: 50,
    });
    assert!(
        f.inbox().is_empty(),
        "the exit of a cancelled wait wakes nobody: {:?}",
        f.inbox()
    );
}

#[test]
fn only_the_task_owner_cancels_a_wait_that_is_going_on() {
    let f = Fixture::new();
    f.create("t1", "timer", Some(5_000_000), None);
    f.wait_on(&["t1"]);
    assert!(cancel(&f, "intruder").is_err());
    assert_eq!(f.task().state, ExecutionState::Waiting);
    assert_eq!(f.wake("t1").state, "scheduled");
    let started_at_ms = f.task().wait.unwrap().started_at_ms;
    cancel(&f, "owner").unwrap();
    let again = f.store.agent_lifecycle(Operation::CancelOwnerWait {
        task: "p".into(),
        room: "room".into(),
        owner: "owner".into(),
        started_at_ms,
    });
    assert!(
        again.is_err(),
        "a task that no longer waits has nothing to cancel"
    );
}
