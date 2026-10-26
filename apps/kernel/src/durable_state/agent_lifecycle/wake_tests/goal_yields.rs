//! MP-08 / MP-09 / MP-10 / MP-11: a tool-only native end can commit a goal yield.
use super::goal_waits::{assess_waiting, settle, start, yield_on};
use super::*;

#[test]
fn a12_empty_native_end_commits_the_accepted_timer_yield() {
    for fired_before_end in [false, true] {
        let f = Fixture::new();
        let due = 300_010;
        start(&f, due);
        yield_on(&f, "p", 20, due);
        let now = if fired_before_end {
            f.apply(Operation::FireWakes { now: due });
            due + 1
        } else {
            30
        };
        settle(&f, "p", now, false);
        let task = f.task();
        assert_eq!(
            task.state,
            ExecutionState::Working,
            "accepted yield must reach independent assessment even without text: {}",
            task.reason
        );
        assert!(completion::evaluation(&task).is_some());
        assert_eq!(
            task.wait.as_ref().unwrap().registration_ids,
            vec!["completion-timer".to_string()]
        );
        assess_waiting(&f, now + 1, due);
        let checked = f.task();
        if fired_before_end {
            assert_eq!(
                checked.state,
                ExecutionState::Working,
                "a racing timer receipt must be reconciled by the provider"
            );
            assert!(checked.pending_prompt_id.is_some());
        } else {
            assert_eq!(checked.state, ExecutionState::Waiting);
            assert!(checked.pending_prompt_id.is_none());
        }
    }
}

#[test]
fn a12_empty_native_end_without_an_accepted_yield_still_blocks() {
    let f = Fixture::new();
    start(&f, 300_010);
    settle(&f, "p", 30, false);
    assert_eq!(f.task().state, ExecutionState::Blocked);
    assert!(f.task().reason.contains("without a public answer"));
}
