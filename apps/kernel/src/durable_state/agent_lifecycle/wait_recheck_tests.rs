//! A wait whose source ended is rechecked only when no outcome of it is left for the agent: the
//! sweep asks the same question as the wait's own settlement, so an unread outcome is delivered
//! instead of a "re-evaluate" row in front of it.
use super::*;
use tests::Fixture;

fn waiting_on_the_child(f: &Fixture, cursor: u64) {
    f.begin("p");
    f.register();
    f.subscribe();
    f.apply(Operation::Yield {
        task: "p".into(),
        prompt: "p".into(),
        registrations: vec!["reg".into()],
        cursor,
        deadline: Some(60_000),
        reason: "child result".into(),
        now: 2,
    });
    f.settle("p", true);
    assert_eq!(f.task().state, ExecutionState::Waiting);
}

fn sweep_at(f: &Fixture, now: u64) {
    f.apply(Operation::Sweep {
        now,
        vault_wait_recipients: Vec::new(),
        busy_recipients: Vec::new(),
        held_work: Vec::new(),
    });
}

fn kinds(f: &Fixture) -> Vec<String> {
    f.store
        .agent_inbox("room", "parent", 0)
        .unwrap()
        .into_iter()
        .map(|event| event.kind)
        .collect()
}

/// The agent named an inbox cursor past the outcome that later arrived: the outcome is still
/// unread, so it is what the agent gets.
#[test]
fn an_outcome_behind_the_yield_cursor_is_not_replaced_by_a_recheck() {
    let f = Fixture::new();
    waiting_on_the_child(&f, 1_000);
    f.apply(Operation::SourceOutcome {
        now: 5,
        public_answer: None,
        room: "room".into(),
        source: "child".into(),
        occurrence: "child-result".into(),
        success: true,
    });
    sweep_at(&f, 10);
    assert_eq!(kinds(&f), ["source_completed"]);
    sweep_at(&f, 60_000);
    assert_eq!(kinds(&f), ["source_completed"]);
    assert_eq!(f.task().state, ExecutionState::Waiting);
}

/// The child's receipt arrived already handled; its child_finished is the outcome the agent
/// has not read yet.
#[test]
fn an_unread_child_finished_is_not_replaced_by_a_recheck() {
    let f = Fixture::new();
    waiting_on_the_child(&f, 0);
    let mut finished = occurrence(
        "room",
        "parent",
        "child",
        "child-finished-child",
        "child_finished",
        serde_json::json!({"child_task_id":"child","success":true}),
    );
    finished.urgent = true;
    f.apply(Operation::Occur(finished));
    f.apply(Operation::SourceOutcome {
        now: 5,
        public_answer: None,
        room: "room".into(),
        source: "child".into(),
        occurrence: "child-result".into(),
        success: true,
    });
    f.store.agent_ledger_sql_for_test(
        "UPDATE agent_inbox SET payload=json_set(payload,'$.state','handled') WHERE json_extract(payload,'$.kind')='source_completed'",
    );
    sweep_at(&f, 10);
    assert_eq!(kinds(&f), ["child_finished", "source_completed"]);
    assert_eq!(f.task().state, ExecutionState::Waiting);
}

/// Every outcome handled while the obligation still settles: nothing is on its way to the agent,
/// so the ended source is rechecked.
#[test]
fn a_wait_with_nothing_unread_is_rechecked() {
    let f = Fixture::new();
    waiting_on_the_child(&f, 0);
    f.apply(Operation::SourceOutcome {
        now: 5,
        public_answer: None,
        room: "room".into(),
        source: "child".into(),
        occurrence: "child-result".into(),
        success: true,
    });
    f.store.agent_ledger_sql_for_test(
        "UPDATE agent_inbox SET payload=json_set(payload,'$.state','handled')",
    );
    sweep_at(&f, 10);
    assert_eq!(kinds(&f), ["source_completed", "wait_recheck"]);
}
