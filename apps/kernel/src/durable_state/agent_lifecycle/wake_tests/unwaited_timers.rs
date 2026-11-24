//! An agent may always end its turn: open timers it does not wait on never refuse a yield or a
//! turn end, and they outlive the turn like a watched process: arming a timer means "wake me".
//! Row 525: a timer dropped with the turn left its agent idle with nothing armed.
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

#[test]
fn a_yield_that_leaves_a_timer_out_waits_on_the_named_one_and_keeps_the_other_open() {
    let f = Fixture::new();
    f.create("waited", "timer", Some(1_000), None);
    f.create("left", "timer", Some(2_000), None);
    f.wait_on(&["waited"]);
    assert_eq!(
        f.task().wait.unwrap().registration_ids,
        ["completion-waited"]
    );
    assert_eq!(f.wake("left").state, "scheduled");
    assert_eq!(f.wake("waited").state, "scheduled");
}

fn open_after_turn_events(f: &Fixture) -> usize {
    f.inbox()
        .iter()
        .filter(|e| crate::durable_state::agent_lifecycle::is_open(e) && e.payload["after_turn"] == true)
        .count()
}

#[test]
fn a_turn_that_ends_without_a_wait_keeps_its_timer_and_the_timer_wakes_the_agent() {
    let f = Fixture::new();
    f.create("once", "timer", Some(1_000), None);
    assert_eq!(settle(&f).state, ExecutionState::Done);
    assert_eq!(f.wake("once").state, "scheduled", "the timer outlives the turn");
    assert_eq!(open_after_turn_events(&f), 0);
    f.apply(Operation::FireWakes { now: 2_000 });
    assert_eq!(f.wake("once").state, "fired");
    assert_eq!(open_after_turn_events(&f), 1, "{:?}", f.inbox());
}

#[test]
fn a_recurring_timer_left_by_a_turn_wakes_the_agent_once_and_ends() {
    let f = Fixture::new();
    f.create("heartbeat", "timer", Some(2_000), Some(60_000));
    assert_eq!(settle(&f).state, ExecutionState::Done);
    assert_eq!(f.wake("heartbeat").state, "scheduled");
    f.apply(Operation::FireWakes { now: 3_000 });
    assert_eq!(f.wake("heartbeat").state, "fired");
    assert_eq!(open_after_turn_events(&f), 1);
    f.apply(Operation::FireWakes { now: 200_000 });
    assert_eq!(open_after_turn_events(&f), 1, "it does not fire again");
}

#[test]
fn a_timer_left_by_a_turn_still_wakes_the_agent_after_a_kernel_restart() {
    let mut f = Fixture::new();
    f.create("once", "timer", Some(1_000), None);
    assert_eq!(settle(&f).state, ExecutionState::Done);
    let temporary = DurableKernelStateStore::open_owned(f.root.join("temporary.sqlite")).unwrap();
    drop(std::mem::replace(&mut f.store, temporary));
    f.store = DurableKernelStateStore::open_owned(f.root.join("state.sqlite")).unwrap();
    assert_eq!(f.wake("once").state, "scheduled");
    f.apply(Operation::FireWakes { now: 2_000 });
    assert_eq!(f.wake("once").state, "fired");
    assert_eq!(open_after_turn_events(&f), 1);
}

#[test]
fn cancel_wake_still_cancels_a_timer_left_by_a_turn() {
    let f = Fixture::new();
    f.create("once", "timer", Some(1_000), None);
    assert_eq!(settle(&f).state, ExecutionState::Done);
    f.apply(Operation::CancelWake {
        id: "once".into(),
        task: "p".into(),
        prompt: None,
        pending: false,
    });
    assert_eq!(f.wake("once").state, "cancelled");
    f.apply(Operation::FireWakes { now: 2_000 });
    assert_eq!(f.wake("once").state, "cancelled");
    assert_eq!(open_after_turn_events(&f), 0);
}

#[test]
fn a_yielded_timer_still_wakes_its_waiting_task() {
    let f = Fixture::new();
    f.create("waited", "timer", Some(1_000), None);
    f.wait_on(&["waited"]);
    f.apply(Operation::FireWakes { now: 2_000 });
    assert_eq!(f.wake("waited").state, "fired");
    assert_eq!(open_after_turn_events(&f), 0, "the task is waiting, not done");
    assert!(
        f.inbox().iter().any(crate::durable_state::agent_lifecycle::is_open),
        "{:?}",
        f.inbox()
    );
}
