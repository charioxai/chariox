//! Row 690: a task whose obligations all ended leaves the supervision sweep's set. Only a task with
//! an obligation that still requires resolution (`AgentObligation::requires_resolution`), or one of
//! the other unfinished or unrecorded states, is swept; the settled history is not read, decoded or
//! visited on every pass.
use super::*;
use tests::Fixture;

fn obligation(id: &str, kind: &str, status: &str, dispatch_state: &str) -> AgentObligation {
    AgentObligation {
        id: id.into(),
        kind: kind.into(),
        resource_id: Some(format!("source-{id}")),
        completion_task_id: None,
        status: status.into(),
        dispatch_state: dispatch_state.into(),
    }
}

fn insert(
    db: &Connection,
    base: &AgentTaskExecution,
    id: &str,
    state: ExecutionState,
    obligations: Vec<AgentObligation>,
) {
    let mut task = base.clone();
    task.task_id = id.into();
    task.prompt_id = format!("prompt-{id}");
    task.state = state;
    task.obligations = obligations;
    // The terminal outcome of a finished task is recorded; only the obligations are under test.
    let recorded = i32::from(matches!(
        task.state,
        ExecutionState::Done | ExecutionState::Cancelled
    ));
    db.execute(
        "INSERT INTO agent_tasks(task_id,room_id,agent_id,prompt_id,payload,terminal_outcome_recorded) VALUES(?1,?2,?3,?4,?5,?6)",
        params![task.task_id, task.room_id, task.agent_id, task.prompt_id, serde_json::to_string(&task).unwrap(), recorded],
    )
    .unwrap();
}

fn swept(f: &Fixture) -> Vec<String> {
    let mut ids = f
        .store
        .agent_tasks_to_sweep()
        .unwrap()
        .into_iter()
        .map(|task| task.task_id)
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

/// Cell 1: 2,000 settled tasks (a satisfied reply, a cancelled delegate, a failed delegate whose
/// dispatch was rejected) are not in the set; each task that still owes something is.
#[test]
fn settled_obligations_leave_the_sweep_set_and_open_ones_stay() {
    let f = Fixture::new();
    f.begin("current");
    let base = f.task();
    let db = Connection::open(f.root.join("state.sqlite")).unwrap();
    db.execute_batch("BEGIN").unwrap();
    for n in 0..2_000 {
        let settled = match n % 3 {
            0 => obligation(&format!("r{n}"), "reply", "satisfied", "accepted"),
            1 => obligation(&format!("d{n}"), "delegate", "cancelled", "accepted"),
            _ => obligation(&format!("f{n}"), "delegate", "failed", "rejected"),
        };
        insert(&db, &base, &format!("settled-{n}"), ExecutionState::Done, vec![settled]);
    }
    insert(
        &db,
        &base,
        "owes-open",
        ExecutionState::Done,
        vec![
            obligation("a", "reply", "satisfied", "accepted"),
            obligation("b", "delegate", "open", "accepted"),
        ],
    );
    insert(
        &db,
        &base,
        "owes-settling",
        ExecutionState::Done,
        vec![obligation("c", "timer", "settling", "accepted")],
    );
    insert(
        &db,
        &base,
        "owes-failed-dispatch-accepted",
        ExecutionState::Done,
        vec![obligation("d", "delegate", "failed", "accepted")],
    );
    insert(
        &db,
        &base,
        "owes-open-cancelled",
        ExecutionState::Cancelled,
        vec![obligation("e", "delegate", "open", "accepted")],
    );
    db.execute_batch("COMMIT").unwrap();

    assert_eq!(
        swept(&f),
        [
            "owes-failed-dispatch-accepted",
            "owes-open",
            "owes-open-cancelled",
            "owes-settling",
            base.task_id.as_str(),
        ]
        .map(String::from)
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>(),
        "the 2,000 settled tasks must not be read by the sweep"
    );
}

/// An obligation that is added to a settled task, or reopened, puts the task back in the set.
#[test]
fn a_task_that_takes_a_new_open_obligation_is_swept_again() {
    let f = Fixture::new();
    f.begin("current");
    let base = f.task();
    let db = Connection::open(f.root.join("state.sqlite")).unwrap();
    insert(
        &db,
        &base,
        "history",
        ExecutionState::Done,
        vec![obligation("r", "reply", "satisfied", "accepted")],
    );
    assert!(!swept(&f).contains(&"history".to_string()));
    let mut task = f
        .store
        .agent_tasks(Some("room"), Some("parent"))
        .unwrap()
        .into_iter()
        .find(|t| t.task_id == "history")
        .unwrap();
    task.obligations.push(obligation("again", "delegate", "open", "accepted"));
    db.execute(
        "UPDATE agent_tasks SET payload=?1 WHERE task_id='history'",
        params![encode(&task).unwrap()],
    )
    .unwrap();
    assert!(swept(&f).contains(&"history".to_string()));
}
