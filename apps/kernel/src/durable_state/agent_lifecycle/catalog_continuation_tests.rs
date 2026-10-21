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
fn catalog_handoff_retains_task_across_provider_reload() {
    let f = Fixture::new();
    f.begin("p");
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
    assert!(!correction);
    assert_eq!(task.state, ExecutionState::Done);
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
    f.apply(Operation::StartGoal {
        task: "p".into(),
        owner: "owner".into(),
        objective: "Implement and verify real work".into(),
        minutes: 10,
        now: 1,
    });
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
    assert_eq!(task.no_progress_wakes, 1);
}

// MP-08 / MP-10 / MP-11: tool reloads cannot evade the existing no-progress bound.
#[test]
fn catalog_reload_chain_blocks_without_real_progress() {
    let f = Fixture::new();
    f.begin("p");
    for _ in 0..NO_PROGRESS_LIMIT {
        let prompt = f.task().prompt_id;
        f.apply(prepare("room", "parent", &prompt, "run"));
        let next = f.task().pending_prompt_id.unwrap();
        f.begin(&next);
    }
    let prompt = f.task().prompt_id;
    f.apply(prepare("room", "parent", &prompt, "run"));
    let stopped = f.task();
    assert_eq!(stopped.state, ExecutionState::Blocked);
    assert!(stopped.pending_prompt_id.is_none());
    assert!(stopped.reason.contains("resume or cancel"));
    assert_eq!(stopped.blocked_revision, stopped.revision);
    f.apply(prepare("room", "parent", &stopped.prompt_id, "run"));
    assert_eq!(
        f.task(),
        stopped,
        "a blocked chain keeps one owner decision"
    );
}

#[test]
fn catalog_begin_rechecks_owner_disposition_and_exact_scope() {
    for disposition in [
        "block",
        "cancel",
        "room",
        "agent",
        "task",
        "prompt",
        "duplicate",
    ] {
        let f = Fixture::new();
        f.begin("p");
        f.apply(prepare("room", "parent", "p", "run"));
        let mut room = "room";
        let mut agent = "parent";
        let mut task = "p";
        let mut prompt = "task-catalog:p";
        match disposition {
            "block" => {
                f.apply(Operation::Block {
                    task: "p".into(),
                    prompt: "p".into(),
                    reason: "owner action".into(),
                });
            }
            "cancel" => {
                f.apply(Operation::CancelTask {
                    task: "p".into(),
                    owner: "owner".into(),
                    revision: f.task().revision,
                });
            }
            "room" => room = "foreign",
            "agent" => agent = "foreign",
            "task" => task = "foreign",
            "prompt" => prompt = "task-catalog:foreign",
            _ => f.begin("task-catalog:p"),
        }
        let before = f.task();
        let result = f.store.agent_lifecycle(Operation::BeginContinuation {
            task: task.into(),
            room: room.into(),
            agent: agent.into(),
            prompt: prompt.into(),
            run: Some("run".into()),
            now: 5,
        });
        if disposition == "duplicate" {
            // Admission and the later provider dispatch both validate the same
            // prompt; the second validation must be idempotent, not a new turn.
            assert!(result.is_ok());
        } else {
            assert!(result.is_err(), "{disposition} must retire admission");
        }
        assert_eq!(f.task(), before);
    }
}

#[test]
fn handled_progress_renews_catalog_wake_budget_without_spending_completion_checks() {
    let f = Fixture::new();
    f.begin("p");
    for _ in 0..NO_PROGRESS_LIMIT {
        let prompt = f.task().prompt_id;
        f.apply(prepare("room", "parent", &prompt, "run"));
        f.begin(&f.task().pending_prompt_id.unwrap());
    }
    let prompt = f.task().prompt_id;
    f.apply(Operation::Progress {
        task: "p".into(),
        prompt: prompt.clone(),
        receipt: "handled-public-result".into(),
        now: 5,
    });
    f.apply(prepare("room", "parent", &prompt, "run"));
    let task = f.task();
    assert_eq!(task.state, ExecutionState::Working);
    assert!(task.pending_prompt_id.is_some());
    assert_eq!(task.no_progress_wakes, 1);
    assert_eq!(task.progress_sequence, 1);
}

#[test]
fn catalog_handoff_cannot_replace_a_wait_or_another_pending_intent() {
    for wait in [true, false] {
        let f = Fixture::new();
        f.begin("p");
        if wait {
            f.register();
            f.subscribe();
            f.apply(Operation::Yield {
                task: "p".into(),
                prompt: "p".into(),
                registrations: vec!["reg".into()],
                cursor: 0,
                deadline: 60_000,
                reason: "source result".into(),
                now: 2,
            });
        } else {
            f.apply(Operation::StartGoal {
                task: "p".into(),
                owner: "owner".into(),
                objective: "Implement and verify real work".into(),
                minutes: 10,
                now: 1,
            });
            f.settle("p", true);
        }
        let before = f.task();
        f.apply(prepare("room", "parent", "p", "run"));
        assert_eq!(f.task(), before);
    }
}
