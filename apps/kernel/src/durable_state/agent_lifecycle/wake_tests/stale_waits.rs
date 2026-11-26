//! An agent that got several prompts has waits in earlier prompts' tasks. A newer wait takes over
//! the earlier task's timers and watches; an earlier wait that times out beside a newer task starts
//! no turn; a wait the agent cancels itself leaves no notice behind (row 470).
use super::*;

/// The turn of prompt `prompt` begins as its own task, beside the earlier ones.
fn begin(f: &Fixture, prompt: &str) {
    f.apply(Operation::Begin {
        owner: "owner".into(),
        room: "room".into(),
        agent: "agent".into(),
        prompt: prompt.into(),
        run: Some(format!("run-{prompt}")),
        now: 40,
    });
}

fn yield_in(f: &Fixture, prompt: &str, ids: &[&str], deadline: u64) {
    f.apply(Operation::Yield {
        task: prompt.into(),
        prompt: prompt.into(),
        registrations: ids.iter().map(|id| format!("completion-{id}")).collect(),
        cursor: 0,
        deadline: Some(deadline),
        reason: "wake".into(),
        now: 50,
    });
}

fn settle_in(f: &Fixture, prompt: &str) {
    f.apply(Operation::Settle {
        room: "room".into(),
        agent: "agent".into(),
        prompt: prompt.into(),
        run: if prompt == "p" {
            "run".into()
        } else {
            format!("run-{prompt}")
        },
        has_answer: true,
        cancelled: false,
        now: 60,
    });
}

fn sweep(f: &Fixture, now: u64) {
    f.apply(Operation::Sweep {
        now,
        vault_wait_recipients: Vec::new(),
        busy_recipients: Vec::new(),
        held_work: Vec::new(),
    });
}

fn task(f: &Fixture, id: &str) -> AgentTaskExecution {
    f.store
        .agent_tasks(Some("room"), Some("agent"))
        .unwrap()
        .into_iter()
        .find(|t| t.task_id == id)
        .unwrap()
}

fn waiting(f: &Fixture) -> Vec<String> {
    f.store
        .agent_tasks(Some("room"), Some("agent"))
        .unwrap()
        .into_iter()
        .filter(|t| t.state == ExecutionState::Waiting)
        .map(|t| t.task_id)
        .collect()
}

/// The turns the agent's pending events would start: one per task they name.
fn woken_tasks(f: &Fixture) -> std::collections::BTreeSet<String> {
    f.inbox()
        .into_iter()
        .filter(crate::durable_state::agent_lifecycle::is_open)
        .map(|e| {
            e.payload["task_id"]
                .as_str()
                .map_or_else(|| e.source_id.clone(), str::to_owned)
        })
        .collect()
}

/// C1: a newer wait takes over the earlier task's timer, so one source firing is one task's wake.
#[test]
fn c1_a_newer_wait_takes_over_the_earlier_tasks_timer() {
    let f = Fixture::new();
    f.create("old", "timer", Some(1_000), None);
    f.wait_on(&["old"]);
    begin(&f, "q");
    f.create_in("q", "new", "timer", Some(1_000), None);
    yield_in(&f, "q", &["new"], 10_000_000);
    settle_in(&f, "q");
    assert_eq!(waiting(&f), ["q"], "one wait holds both timers");
    let q = task(&f, "q");
    let held = q.wait.unwrap().registration_ids;
    assert!(held.contains(&"completion-old".to_owned()), "{held:?}");
    assert_eq!(f.wake("old").state, "scheduled", "the timer keeps running");
    f.apply(Operation::FireWakes { now: 2_000 });
    let woken = woken_tasks(&f);
    assert_eq!(woken.len(), 1, "both timers wake one task: {woken:?}");
}

/// C2: an earlier wait that times out beside a newer running task starts no turn.
#[test]
fn c2_an_earlier_wait_that_times_out_beside_a_newer_task_starts_no_turn() {
    let f = Fixture::new();
    f.create("old", "timer", Some(9_000_000), None);
    f.wait_on(&["old"]);
    begin(&f, "q");
    sweep(&f, 10_000_001);
    let timed_out: Vec<_> = f
        .inbox()
        .into_iter()
        .filter(|e| e.kind == "deadline_reached")
        .collect();
    assert!(timed_out.is_empty(), "{timed_out:?}");
}

/// C2b: the same when the newer task already waits (its deadline is later).
#[test]
fn c2b_an_earlier_wait_that_times_out_beside_a_newer_wait_starts_no_turn() {
    let f = Fixture::new();
    f.create("old", "timer", Some(9_000_000), None);
    f.wait_on(&["old"]);
    begin(&f, "q");
    f.create_in("q", "new", "timer", Some(30_000_000), None);
    yield_in(&f, "q", &["new"], 40_000_000);
    settle_in(&f, "q");
    sweep(&f, 10_000_001);
    let timed_out: Vec<_> = f
        .inbox()
        .into_iter()
        .filter(|e| e.kind == "deadline_reached")
        .collect();
    assert!(timed_out.is_empty(), "{timed_out:?}");
}

/// C3: a wait the agent cancels from a later prompt leaves no notice that starts a turn.
#[test]
fn c3_a_wait_the_agent_cancels_itself_starts_no_turn() {
    let f = Fixture::new();
    f.create("old", "timer", Some(9_000_000), None);
    f.wait_on(&["old"]);
    begin(&f, "q");
    f.apply(Operation::CancelWake {
        id: "old".into(),
        task: "q".into(),
        prompt: Some("q".into()),
        pending: false,
    });
    sweep(&f, 70);
    let open: Vec<_> = f
        .inbox()
        .into_iter()
        .filter(crate::durable_state::agent_lifecycle::is_open)
        .collect();
    assert!(open.is_empty(), "{open:?}");
}

/// C6: an earlier task whose wait source ended and was handled, or that a newer task adopted,
/// starts no turn when its deadline passes.
#[test]
fn c6_an_earlier_wait_whose_source_was_handled_starts_no_turn_at_its_deadline() {
    let f = Fixture::new();
    f.create("old", "timer", Some(1_000), None);
    f.wait_on(&["old"]);
    f.apply(Operation::FireWakes { now: 2_000 });
    for e in f.inbox() {
        f.apply(Operation::Ack {
            room: "room".into(),
            agent: "agent".into(),
            sequence: e.sequence,
            handled: true,
            now: 2_500,
        });
    }
    begin(&f, "q");
    f.create_in("q", "new", "timer", Some(30_000_000), None);
    yield_in(&f, "q", &["new"], 40_000_000);
    settle_in(&f, "q");
    sweep(&f, 10_000_001);
    let timed_out: Vec<_> = f
        .inbox()
        .into_iter()
        .filter(|e| matches!(e.kind.as_str(), "deadline_reached" | "wait_recheck"))
        .collect();
    assert!(timed_out.is_empty(), "{timed_out:?}");
}
