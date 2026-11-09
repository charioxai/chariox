//! MP-08 / MP-09 / MP-10 / MP-11: the agents a managed machine must stay up for.
use super::*;

fn awaiting(f: &Fixture) -> Vec<(String, String)> {
    f.store
        .agents_awaiting_wake()
        .unwrap()
        .into_iter()
        .collect()
}

fn agent() -> Vec<(String, String)> {
    vec![("room".into(), "agent".into())]
}

fn cancel(f: &Fixture, id: &str) {
    f.wakes(Operation::CancelWake {
        id: id.into(),
        task: "p".into(),
        prompt: Some("p".into()),
        pending: false,
    });
}

fn subscribe(f: &Fixture) {
    f.apply(Operation::Subscribe {
        task: "p".into(),
        prompt: "p".into(),
        registration: Registration {
            id: "reg".into(),
            task_id: "p".into(),
            source_id: "peer".into(),
            obligation_id: None,
            source_cursor: 0,
            live: true,
            peer: false,
        },
    });
}

#[test]
fn managed_working_agents_nothing_armed_is_not_awaiting() {
    let f = Fixture::new();
    assert!(awaiting(&f).is_empty());
}

#[test]
fn managed_working_agents_armed_timer_counts_until_cancelled() {
    let f = Fixture::new();
    f.create("timer", "timer", Some(1_000_000), None);
    assert_eq!(awaiting(&f), agent());
    cancel(&f, "timer");
    assert_eq!(f.wake("timer").state, "cancelled");
    assert!(awaiting(&f).is_empty());
}

#[test]
fn managed_working_agents_one_shot_timer_stops_counting_once_fired() {
    let f = Fixture::new();
    f.create("timer", "timer", Some(1_000), None);
    assert_eq!(awaiting(&f), agent());
    f.wakes(Operation::FireWakes { now: 1_500 });
    assert_eq!(f.wake("timer").state, "fired");
    assert!(awaiting(&f).is_empty());
}

#[test]
fn managed_working_agents_repeating_timer_keeps_counting_after_it_fires() {
    let f = Fixture::new();
    f.create("interval", "timer", Some(1_000), Some(60_000));
    f.wakes(Operation::FireWakes { now: 1_500 });
    assert_eq!(f.wake("interval").state, "scheduled");
    assert_eq!(awaiting(&f), agent());
}

#[test]
fn managed_working_agents_watched_process_counts_until_it_exits() {
    let f = Fixture::new();
    f.create("process", "process", None, None);
    f.apply(Operation::ProcessStarted {
        id: "process".into(),
        pid: 42,
        now: 20,
    });
    assert_eq!(f.wake("process").state, "running");
    assert_eq!(awaiting(&f), agent());
    f.apply(Operation::ProcessExited {
        id: "process".into(),
        exit_code: Some(0),
        tail: String::new(),
        now: 30,
    });
    assert_eq!(f.wake("process").state, "exited");
    assert!(awaiting(&f).is_empty());
}

#[test]
fn managed_working_agents_lost_process_does_not_count() {
    let f = Fixture::new();
    f.create("process", "process", None, None);
    f.apply(Operation::ProcessStarted {
        id: "process".into(),
        pid: 42,
        now: 20,
    });
    f.apply(Operation::ProcessExited {
        id: "process".into(),
        exit_code: None,
        tail: String::new(),
        now: 30,
    });
    assert_eq!(f.wake("process").state, "lost");
    assert!(awaiting(&f).is_empty());
}

#[test]
fn managed_working_agents_live_subscription_counts_until_dropped() {
    let f = Fixture::new();
    subscribe(&f);
    assert_eq!(awaiting(&f), agent());
    f.apply(Operation::Unsubscribe {
        task: "p".into(),
        prompt: "p".into(),
        registration: "reg".into(),
    });
    assert!(awaiting(&f).is_empty());
}

#[test]
fn managed_working_agents_subscription_ends_with_its_task() {
    let f = Fixture::new();
    subscribe(&f);
    f.apply(Operation::Settle {
        room: "room".into(),
        agent: "agent".into(),
        prompt: "p".into(),
        run: "run".into(),
        has_answer: true,
        cancelled: false,
        now: 30,
    });
    assert_eq!(f.task().state, ExecutionState::Done);
    assert!(awaiting(&f).is_empty());
}

#[test]
fn managed_working_agents_waiting_task_keeps_counting_while_its_timer_is_armed() {
    let f = Fixture::new();
    f.create("timer", "timer", Some(1_000_000), None);
    f.wait_on(&["timer"]);
    assert_eq!(awaiting(&f), agent());
    f.apply(Operation::CancelTask {
        task: "p".into(),
        owner: "owner".into(),
        revision: f.task().revision,
    });
    assert_eq!(f.task().state, ExecutionState::Cancelled);
    // The armed timer is still a source that can wake the agent.
    assert_eq!(awaiting(&f), agent());
}

#[test]
fn managed_working_agents_task_blocked_on_the_owner_counts_until_answered() {
    let f = Fixture::new();
    f.apply(Operation::Block {
        task: "p".into(),
        prompt: "p".into(),
        reason: "owner action".into(),
    });
    assert_eq!(f.task().state, ExecutionState::Blocked);
    assert_eq!(awaiting(&f), agent());
    f.apply(Operation::CancelTask {
        task: "p".into(),
        owner: "owner".into(),
        revision: f.task().revision,
    });
    assert!(awaiting(&f).is_empty());
}

#[test]
fn managed_working_agents_paused_goal_waits_for_nobody() {
    let f = Fixture::new();
    f.apply(Operation::StartGoal {
        task: "p".into(),
        owner: "owner".into(),
        objective: "Keep going until the owner pauses the goal".into(),
        minutes: 10,
        now: 1,
    });
    f.apply(Operation::ControlGoal {
        task: "p".into(),
        owner: "owner".into(),
        revision: f.task().revision,
        action: "pause".into(),
        value: None,
        now: 2,
    });
    assert!(f.task().goal.as_ref().is_some_and(|goal| goal.paused));
    assert!(awaiting(&f).is_empty());
}

#[test]
fn managed_working_agents_each_agent_is_listed_once_per_room() {
    let f = Fixture::new();
    f.create("one", "timer", Some(1_000_000), None);
    f.create("two", "timer", Some(2_000_000), None);
    subscribe(&f);
    f.apply(Operation::Begin {
        owner: "owner".into(),
        room: "other-room".into(),
        agent: "other-agent".into(),
        prompt: "q".into(),
        run: Some("run-2".into()),
        now: 5,
    });
    f.apply(Operation::Subscribe {
        task: "q".into(),
        prompt: "q".into(),
        registration: Registration {
            id: "reg-q".into(),
            task_id: "q".into(),
            source_id: "peer".into(),
            obligation_id: None,
            source_cursor: 0,
            live: true,
            peer: false,
        },
    });
    assert_eq!(
        awaiting(&f),
        vec![
            ("other-room".to_string(), "other-agent".to_string()),
            ("room".to_string(), "agent".to_string()),
        ]
    );
}
