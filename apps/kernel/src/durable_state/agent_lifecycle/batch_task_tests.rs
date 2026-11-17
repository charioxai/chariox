//! One wake resumes one task: a prompt that woke two Waiting tasks left the
//! second Working under a prompt Begin never resumed it for.
use super::*;
use tests::Fixture;

fn wait_on(f: &Fixture, task: &str) {
    f.begin(task);
    f.apply(Operation::Subscribe {
        task: task.into(),
        prompt: task.into(),
        registration: Registration {
            id: format!("{task}-reg"),
            task_id: task.into(),
            source_id: "peer".into(),
            obligation_id: None,
            source_cursor: 0,
            live: true,
            peer: false,
        },
    });
    f.apply(Operation::Yield {
        task: task.into(),
        prompt: task.into(),
        registrations: vec![format!("{task}-reg")],
        cursor: 0,
        deadline: Some(60_000),
        reason: "peer".into(),
        now: 2,
    });
    f.settle(task, true);
}

fn completed(f: &Fixture, id: &str, task: &str) -> InboxEvent {
    let Outcome::Event(event) = f.apply(Operation::Occur(occurrence(
        "room",
        "parent",
        "peer",
        id,
        "source_completed",
        serde_json::json!({ "task_id": task }),
    ))) else {
        panic!("event")
    };
    event
}

fn attempt(f: &Fixture, lead: &InboxEvent, through: u64) {
    f.apply(Operation::Attempt {
        room: "room".into(),
        agent: "parent".into(),
        sequence: lead.sequence,
        prompt: "wake".into(),
        target: None,
        run: None,
        now: 3,
        work: None,
        through,
    });
}

fn two_waiting_tasks_with_a_result_each(f: &Fixture) -> (InboxEvent, InboxEvent) {
    wait_on(f, "first");
    wait_on(f, "second");
    (completed(f, "r1", "first"), completed(f, "r2", "second"))
}

#[test]
fn a_wake_batch_resumes_only_one_waiting_task() {
    let f = Fixture::new();
    let (lead, next) = two_waiting_tasks_with_a_result_each(&f);
    attempt(&f, &lead, next.sequence);
    let tasks = f.store.agent_tasks(Some("room"), Some("parent")).unwrap();
    let state = |id: &str| tasks.iter().find(|t| t.task_id == id).unwrap().clone();
    assert_eq!(state("first").pending_prompt_id.as_deref(), Some("wake"));
    assert_eq!(state("second").state, ExecutionState::Waiting);
    assert_eq!(state("second").pending_prompt_id, None);
    let rows = f.store.agent_inbox("room", "parent", 0).unwrap();
    assert_eq!(rows[1].state, "pending", "the second result waits its turn");
}

#[test]
fn the_delivery_batch_stops_before_a_second_waiting_task() {
    let f = Fixture::new();
    let (lead, _next) = two_waiting_tasks_with_a_result_each(&f);
    let batch = f.store.agent_delivery_batch(&lead, false, None).unwrap();
    assert!(batch.is_empty(), "a second task needs its own wake");
}

/// Two tasks wait on the same two sources: each source's end is one row naming both tasks.
fn two_shared_source_results(f: &Fixture) -> (InboxEvent, InboxEvent) {
    wait_on(f, "first");
    wait_on(f, "second");
    let shared = |id: &str| {
        let Outcome::Event(event) = f.apply(Operation::Occur(occurrence(
            "room",
            "parent",
            "peer",
            id,
            "source_completed",
            serde_json::json!({ "task_ids": ["first", "second"] }),
        ))) else {
            panic!("event")
        };
        event
    };
    (shared("r1"), shared("r2"))
}

#[test]
fn the_planned_batch_is_the_batch_the_attempt_admits() {
    let f = Fixture::new();
    let (lead, next) = two_shared_source_results(&f);
    let planned = f.store.agent_delivery_batch(&lead, false, None).unwrap();
    attempt(&f, &lead, next.sequence);
    let admitted = f
        .store
        .agent_events_for_prompt("room", "parent", "wake")
        .unwrap();
    let sequences = |rows: &[InboxEvent]| rows.iter().map(|e| e.sequence).collect::<Vec<_>>();
    let mut planned_rows = vec![lead.sequence];
    planned_rows.extend(sequences(&planned));
    assert_eq!(planned_rows, sequences(&admitted));
    let rows = f.store.agent_inbox("room", "parent", 0).unwrap();
    assert!(
        rows.iter()
            .all(|row| row.prompt_id.as_deref() == Some("wake") || row.state == "pending"),
        "a row is either in the prompt's receipt or waits for its own wake"
    );
}

/// What a batch left behind before one prompt resumed one task: the task is Working under a
/// prompt nothing runs, the sweep blocks it as interrupted, and its wait is already answered.
fn stranded_blocked_task(f: &Fixture) -> AgentTaskExecution {
    wait_on(f, "first");
    // The source's end closes the registration and tells the agent, as in production.
    f.apply(Operation::SourceOutcome {
        public_answer: None,
        room: "room".into(),
        source: "peer".into(),
        occurrence: "r1".into(),
        success: true,
        now: 3,
    });
    let result = f.store.agent_inbox("room", "parent", 0).unwrap().remove(0);
    attempt(f, &result, result.sequence);
    let woken = f
        .store
        .agent_tasks(Some("room"), Some("parent"))
        .unwrap()
        .remove(0);
    assert_eq!(woken.pending_prompt_id.as_deref(), Some("wake"));
    for operation in [
        Operation::Receipt {
            room: "room".into(),
            agent: "parent".into(),
            sequence: result.sequence,
            state: "accepted".into(),
            now: 4,
        },
        Operation::Ack {
            room: "room".into(),
            agent: "parent".into(),
            sequence: result.sequence,
            handled: true,
            now: 5,
        },
        Operation::EndFailedTurn {
            task: "first".into(),
            prompt: "first".into(),
            reason: "Interrupted before it finished.".into(),
        },
    ] {
        f.apply(operation);
    }
    let task = f
        .store
        .agent_tasks(Some("room"), Some("parent"))
        .unwrap()
        .remove(0);
    assert_eq!(task.state, ExecutionState::Blocked);
    assert!(
        task.pending_prompt_id.is_some(),
        "the wake prompt is stale: {task:?}"
    );
    assert!(!task
        .obligations
        .iter()
        .any(AgentObligation::requires_resolution));
    let registrations = f.store.agent_registrations("first").unwrap();
    assert!(
        registrations.iter().all(|r| !r.live),
        "its source ended: {registrations:?}"
    );
    task
}

fn retire(f: &Fixture, task: &AgentTaskExecution, later_prompt: &str) -> AgentTaskExecution {
    let Outcome::Task(task) = f.apply(Operation::RetireRestartFailure {
        task: task.task_id.clone(),
        revision: task.blocked_revision,
        later_prompt: later_prompt.into(),
        now: 9,
    }) else {
        panic!("task")
    };
    task
}

#[test]
fn a_later_turn_supersedes_an_interrupted_task_whose_work_is_over() {
    let f = Fixture::new();
    let stranded = stranded_blocked_task(&f);
    let after = retire(&f, &stranded, "later");
    assert_eq!(after.state, ExecutionState::Cancelled);
    assert_eq!(after.pending_prompt_id, None);
    assert!(
        after.reason.starts_with("Superseded by a later turn"),
        "the reason is recorded: {}",
        after.reason
    );
}

#[test]
fn a_later_turn_does_not_supersede_a_task_while_other_rows_wait() {
    let f = Fixture::new();
    let stranded = stranded_blocked_task(&f);
    let message = occurrence(
        "room",
        "parent",
        "peer",
        "m",
        "peer_event",
        serde_json::json!({}),
    );
    let Outcome::Event(message) = f.apply(Operation::Occur(message)) else {
        panic!("event")
    };
    assert_eq!(
        retire(&f, &stranded, "later"),
        stranded,
        "a pending row is unresolved"
    );
    attempt(&f, &message, message.sequence);
    assert_eq!(
        retire(&f, &stranded, "other"),
        stranded,
        "an admitted row of another prompt is unresolved"
    );
    assert_eq!(
        retire(&f, &stranded, "wake").state,
        ExecutionState::Cancelled,
        "the rows of the later turn itself are its delivery, not work left behind"
    );
}
