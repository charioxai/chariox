//! An orchestrator can give a long-running child an explicit long wait. A deadline that
//! finds the child still running is not a no-progress strike.
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
fn an_explicit_long_wait_gets_no_early_timed_wake_while_the_child_runs() {
    let f = Fixture::new();
    wait_on_child(&f, Some(6 * 60 * 60_000));
    for round in 1..=4 {
        sweep(&f, 3 * OLD_DEADLINE_MS * round);
    }
    assert!(
        f.store.agent_inbox("room", "parent", 0).unwrap().is_empty(),
        "no timer wakes the parent"
    );
    let task = f.task();
    assert_eq!(task.state, ExecutionState::Waiting);
    assert_eq!(task.no_progress_wakes, 0);
}

#[test]
fn a_deadline_that_finds_the_child_running_is_not_a_no_progress_strike() {
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
    assert_eq!(f.task().no_progress_wakes, 0, "the child is still running");
}

// The orchestrator that waited on one long worker with 20-minute deadlines was blocked after the
// third deadline wake ("three consecutive wakes without handled progress").
#[test]
fn three_deadline_wakes_on_a_running_child_never_block_the_orchestrator() {
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
        assert_eq!(task.no_progress_wakes, 0, "wake {n}");
    }
}
