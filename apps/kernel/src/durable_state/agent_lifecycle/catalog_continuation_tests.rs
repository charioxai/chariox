//! MP-08 / MP-10 / MP-11: catalog handoff does not consume task completion or authority.
use super::tests::Fixture;
use super::*;

fn prepare(room: &str, agent: &str, prompt: &str, run: &str) -> Operation {
    Operation::PrepareCatalogContinuation {
        room: room.into(),
        agent: agent.into(),
        prompt: prompt.into(),
        run: run.into(),
        next_prompt: format!("task-catalog:{prompt}"),
        now: 2,
    }
}

#[test]
fn catalog_handoff_retains_task_and_completion_budget_across_provider_reload() {
    let f = Fixture::new();
    f.begin("p");
    f.apply(Operation::EnableCompletionCheck { task: "p".into() });
    let before = f.task();
    f.apply(prepare("room", "parent", "p", "run"));
    let intent = f.task();
    f.apply(prepare("room", "parent", "p", "run"));
    assert_eq!(intent, f.task(), "same catalog intent is idempotent");
    let Outcome::Settled { task, correction } = f.settle("p", true) else {
        panic!()
    };
    assert!(!correction);
    assert!(!task.correction_used);
    assert_eq!(task.completion_check, before.completion_check);
    assert_eq!(task.task_id, before.task_id);
    assert_eq!(task.state, ExecutionState::Working);
    f.apply(Operation::BeginContinuation {
        task: "p".into(),
        room: "room".into(),
        agent: "parent".into(),
        prompt: "task-catalog:p".into(),
        run: Some("run".into()),
        now: 4,
    });
    assert_eq!(f.task().task_id, "p");
    assert_eq!(
        f.store
            .agent_tasks(Some("room"), Some("parent"))
            .unwrap()
            .len(),
        1
    );
    assert!(f.task().pending_prompt_id.is_none());
    let Outcome::Settled { task, correction } = f.settle("task-catalog:p", true) else {
        panic!()
    };
    assert!(
        correction,
        "actual completed work still gets normal completion confirmation"
    );
    assert_eq!(task.completion_check.unwrap().check_count, 1);
}

#[test]
fn catalog_handoff_refuses_stale_scope_and_run_without_mutation() {
    let f = Fixture::new();
    f.begin("p");
    let before = f.task();
    for (room, agent, prompt, run) in [
        ("wrong", "parent", "p", "run"),
        ("room", "wrong", "p", "run"),
        ("room", "parent", "wrong", "run"),
        ("room", "parent", "p", "stale"),
    ] {
        assert!(f
            .store
            .agent_lifecycle(prepare(room, agent, prompt, run))
            .is_err());
        assert_eq!(f.task(), before);
    }
}

#[test]
fn catalog_handoff_cannot_resume_blocked_cancelled_or_done_task() {
    for state in ["blocked", "cancelled", "done"] {
        let f = Fixture::new();
        f.begin("p");
        match state {
            "blocked" => {
                f.apply(Operation::Block {
                    task: "p".into(),
                    prompt: "p".into(),
                    reason: "owner action".into(),
                });
            }
            "cancelled" => {
                f.apply(Operation::CancelTask {
                    task: "p".into(),
                    owner: "owner".into(),
                    revision: f.task().revision,
                });
            }
            _ => {
                f.settle("p", true);
            }
        }
        let before = f.task();
        f.apply(prepare("room", "parent", "p", "run"));
        assert_eq!(f.task(), before, "catalog must preserve {state}");
        assert!(f.task().pending_prompt_id.is_none());
    }
}

#[test]
fn catalog_continuation_retired_during_reload_cannot_create_a_fresh_task() {
    let f = Fixture::new();
    f.begin("p");
    f.apply(Operation::EnableCompletionCheck { task: "p".into() });
    f.settle("p", true);
    let check = f.task().pending_prompt_id.unwrap();
    f.begin(&check);
    let evaluation = completion::evaluation(&f.task()).unwrap().clone();
    f.apply(prepare("room", "parent", &check, "run"));
    let pending = f.task().pending_prompt_id.unwrap();
    assert_eq!(
        completion::evaluation(&f.task()).unwrap().prompt_id,
        pending
    );
    f.apply(Operation::TimeoutGoal {
        task: "p".into(),
        prompt: pending.clone(),
        now: evaluation.deadline_ms + 1,
    });
    let stopped = f.task();
    assert_eq!(stopped.state, ExecutionState::Blocked);
    assert!(stopped.pending_prompt_id.is_none());
    assert!(f
        .store
        .agent_lifecycle(Operation::BeginContinuation {
            task: "p".into(),
            room: "room".into(),
            agent: "parent".into(),
            prompt: pending,
            run: Some("reloaded".into()),
            now: evaluation.deadline_ms + 2,
        })
        .is_err());
    assert_eq!(f.task(), stopped);
    assert_eq!(
        f.store
            .agent_tasks(Some("room"), Some("parent"))
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn catalog_handoff_has_a_bounded_fresh_executor_window_after_long_work() {
    let f = Fixture::new();
    f.begin("p");
    let now = DELIVERY_TIMEOUT_MS + 10;
    f.apply(Operation::PrepareCatalogContinuation {
        room: "room".into(),
        agent: "parent".into(),
        prompt: "p".into(),
        run: "run".into(),
        next_prompt: "task-catalog:p".into(),
        now,
    });
    let task = f.task();
    assert!(!lacks_live_executor(&task, now + 1, false, false));
    assert!(lacks_live_executor(
        &task,
        now + DELIVERY_TIMEOUT_MS,
        false,
        false
    ));
    assert_eq!(task.progress_sequence, 0);
    assert_eq!(task.no_progress_wakes, 0);
}
