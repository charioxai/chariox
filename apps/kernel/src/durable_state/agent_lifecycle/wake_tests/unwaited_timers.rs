//! An agent may always end its turn: open timers it does not wait on never refuse a yield or a
//! turn end, and the timers are dropped with the turn.
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

#[test]
fn a_turn_that_ends_without_a_wait_drops_its_one_shot_timers_and_ends_done() {
    let f = Fixture::new();
    f.create("once", "timer", Some(1_000), None);
    assert_eq!(settle(&f).state, ExecutionState::Done);
    assert_eq!(f.wake("once").state, "cancelled");
    assert!(
        f.inbox()
            .iter()
            .all(|e| !crate::durable_state::agent_lifecycle::is_open(e)),
        "a dropped timer leaves nothing to read"
    );
}

#[test]
fn a_recurring_timer_is_dropped_with_the_turn_too() {
    let f = Fixture::new();
    f.create("heartbeat", "timer", Some(2_000), Some(60_000));
    assert_eq!(settle(&f).state, ExecutionState::Done);
    assert_eq!(f.wake("heartbeat").state, "cancelled");
}
