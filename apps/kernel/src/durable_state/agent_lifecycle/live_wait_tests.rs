//! An orchestrator waiting on a running child is not stopped for it: with no deadline nothing wakes
//! it by time, and deadline wakes retain the running child wait.
use super::tests::Fixture;
use super::*;

const OLD_DEADLINE_MS: u64 = 20 * 60_000;

fn sweep(f: &Fixture, now: u64) {
    f.apply(Operation::Sweep {
        now,
        vault_wait_recipients: Vec::new(),
        busy_recipients: Vec::new(),
        held_work: Vec::new(),
    });
}

fn wait_on_child(f: &Fixture, deadline: Option<u64>) {
    f.begin("p");
    f.register();
    f.subscribe();
    f.apply(Operation::Yield {
        task: "p".into(),
        prompt: "p".into(),
        registrations: vec!["reg".into()],
        cursor: 0,
        deadline,
        reason: "child result".into(),
        now: 2,
    });
    f.settle("p", true);
    assert_eq!(f.task().state, ExecutionState::Waiting);
}

#[test]
fn a_wait_without_a_deadline_gets_no_timed_wake_however_long_the_child_runs() {
    let f = Fixture::new();
    wait_on_child(&f, None);
    for round in 1..=4 {
        sweep(&f, 3 * OLD_DEADLINE_MS * round);
    }
    assert!(
        f.store.agent_inbox("room", "parent", 0).unwrap().is_empty(),
        "no timer wakes the parent"
    );
    let task = f.task();
    assert_eq!(task.state, ExecutionState::Waiting);
}

#[test]
fn a_deadline_that_finds_the_child_running_keeps_work_live() {
    let f = Fixture::new();
    wait_on_child(&f, Some(OLD_DEADLINE_MS));
    sweep(&f, OLD_DEADLINE_MS + 1);
    let wake = f
        .store
        .agent_inbox("room", "parent", 0)
        .unwrap()
        .into_iter()
        .find(|event| event.kind == "deadline_reached")
        .expect("the deadline wakes the parent");
    f.apply(Operation::Attempt {
        room: "room".into(),
        agent: "parent".into(),
        sequence: wake.sequence,
        prompt: "wake".into(),
        target: None,
        run: Some("run".into()),
        now: OLD_DEADLINE_MS + 2,
        work: None,
        through: 0,
    });
    assert_eq!(f.task().state, ExecutionState::Working);
    assert_eq!(f.task().wait.unwrap().registration_ids, vec!["reg"]);
}

#[test]
fn repeated_deadline_wakes_on_a_running_child_keep_the_orchestrator_live() {
    let f = Fixture::new();
    wait_on_child(&f, Some(OLD_DEADLINE_MS));
    for n in 1..=4u64 {
        let Outcome::Event(event) = f.apply(Operation::Occur(occurrence(
            "room",
            "parent",
            "parent-task",
            &format!("deadline-{n}"),
            "deadline_reached",
            serde_json::json!({"task_id":"p"}),
        ))) else {
            panic!("event")
        };
        let prompt = format!("wake-{n}");
        f.apply(Operation::Attempt {
            room: "room".into(),
            agent: "parent".into(),
            sequence: event.sequence,
            prompt: prompt.clone(),
            target: None,
            run: None,
            now: 10 + n,
            work: None,
            through: 0,
        });
        f.apply(Operation::Receipt {
            room: "room".into(),
            agent: "parent".into(),
            sequence: event.sequence,
            state: "accepted".into(),
            now: 10 + n,
        });
        f.begin(&prompt);
        f.apply(Operation::Yield {
            task: "p".into(),
            prompt: prompt.clone(),
            registrations: vec!["reg".into()],
            cursor: event.sequence,
            deadline: Some(OLD_DEADLINE_MS * (n + 1)),
            reason: "the child is still running".into(),
            now: 20 + n,
        });
        f.settle(&prompt, true);
        let task = f.task();
        assert_eq!(task.state, ExecutionState::Waiting, "wake {n}: {task:?}");
    }
}
