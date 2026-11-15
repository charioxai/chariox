//! An orchestrator can give a long-running child an explicit long wait. A deadline that
//! finds the child still running keeps the running child wait.
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

fn child_finishes(f: &Fixture) -> u64 {
    f.apply(Operation::SourceOutcome {
        room: "room".into(),
        source: "child".into(),
        occurrence: "child-result".into(),
        success: true,
        public_answer: Some(serde_json::json!({"text":"finished"})),
        now: 4,
    });
    f.store.agent_inbox("room", "parent", 0).unwrap()[0].sequence
}

fn handle(f: &Fixture, sequence: u64, handled: bool) {
    f.apply(Operation::Ack {
        room: "room".into(),
        agent: "parent".into(),
        sequence,
        handled,
        now: 5,
    });
}

#[test]
fn kfinish_handling_the_last_outcome_ends_the_earlier_wait() {
    for deadline in [None, Some(60_000)] {
        let f = Fixture::new();
        wait_on_child(&f, deadline);
        let sequence = child_finishes(&f);
        // An unrelated event turn handles the original task's final receipt.
        f.begin("event-turn");
        handle(&f, sequence, true);
        f.settle("event-turn", true);
        let finished = f.store.agent_task("p").unwrap().unwrap();
        assert_eq!(finished.state, ExecutionState::Done, "{finished:?}");
        assert!(finished.wait.is_none());
        assert!(!finished.reason.is_empty(), "record why the old wait ended");
        let revision = finished.revision;
        sweep(&f, OLD_DEADLINE_MS * 4);
        assert_eq!(f.store.agent_task("p").unwrap().unwrap().revision, revision);
        assert!(
            f.store
                .agent_inbox("room", "parent", sequence)
                .unwrap()
                .is_empty(),
            "a settled task has no deadline or recheck wake"
        );
        let reopened = DurableKernelStateStore::open(f.root.join("state.sqlite")).unwrap();
        assert_eq!(reopened.agent_task("p").unwrap().unwrap(), finished);
    }
}

#[test]
fn kfinish_a_receipt_ack_without_handling_keeps_the_wait_unfinished() {
    let f = Fixture::new();
    wait_on_child(&f, Some(60_000));
    let sequence = child_finishes(&f);
    handle(&f, sequence, false);
    sweep(&f, 6);
    assert_eq!(f.task().state, ExecutionState::Waiting);
    assert_eq!(f.task().obligations[0].status, "settling");
    handle(&f, sequence, true);
    assert_eq!(f.task().state, ExecutionState::Done);
}

#[test]
fn kfinish_sweep_ends_a_wait_whose_last_spawn_was_cancelled() {
    let f = Fixture::new();
    wait_on_child(&f, Some(60_000));
    f.apply(Operation::ReleaseSpawn {
        room: "room".into(),
        source: "child".into(),
    });
    let event = f.store.agent_inbox("room", "parent", 0).unwrap().remove(0);
    handle(&f, event.sequence, true);
    sweep(&f, 60_001);
    let finished = f.task();
    assert_eq!(finished.state, ExecutionState::Done, "{finished:?}");
    assert_eq!(finished.obligations[0].status, "cancelled");
    assert!(finished.wait.is_none());
    assert!(f
        .store
        .agent_inbox("room", "parent", event.sequence)
        .unwrap()
        .is_empty());
}

#[test]
fn kfinish_cancelling_the_last_spawn_still_requires_handling_its_loss() {
    let f = Fixture::new();
    wait_on_child(&f, None);
    f.apply(Operation::ReleaseSpawn {
        room: "room".into(),
        source: "child".into(),
    });
    sweep(&f, 6);
    assert_eq!(f.task().state, ExecutionState::Waiting);
    let event = f.store.agent_inbox("room", "parent", 0).unwrap().remove(0);
    handle(&f, event.sequence, true);
    assert_eq!(f.task().state, ExecutionState::Done);
}
