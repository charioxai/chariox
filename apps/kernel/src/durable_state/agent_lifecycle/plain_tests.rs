//! A plain turn (no goal, obligation or wait) only gets a task row: it is never
//! corrected, held to a wait or blocked for the owner.
use super::tests::Fixture;
use super::*;

#[test]
fn plain_turn_end_is_done_without_a_correction() {
    for has_answer in [true, false] {
        let f = Fixture::new();
        f.begin("p");
        let Outcome::Settled { task, correction } = f.settle("p", has_answer) else {
            panic!()
        };
        assert!(!correction, "answer={has_answer}");
        assert_eq!(task.state, ExecutionState::Done, "answer={has_answer}");
        assert!(task.pending_prompt_id.is_none() && !task.correction_used);
        let again = f.settle("p", has_answer);
        assert!(matches!(
            again,
            Outcome::Settled {
                correction: false,
                ..
            }
        ));
        assert_eq!(f.task().state, ExecutionState::Done);
    }
}

#[test]
fn plain_failed_turn_is_cancelled_not_blocked() {
    let f = Fixture::new();
    f.begin("p");
    end_failed_turn(&f.store, &f.task(), "Provider run failed".into()).unwrap();
    let task = f.task();
    assert_eq!(task.state, ExecutionState::Cancelled, "{task:?}");
    assert!(task.reason.contains("Provider run failed"));
}

#[test]
fn failed_turn_with_ledger_work_still_blocks_for_the_owner() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    end_failed_turn(&f.store, &f.task(), "Provider run failed".into()).unwrap();
    assert_eq!(f.task().state, ExecutionState::Blocked);
}

#[test]
fn turn_with_a_registered_obligation_keeps_the_ledger_correction() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    let Outcome::Settled { task, correction } = f.settle("p", true) else {
        panic!()
    };
    assert!(correction);
    assert_ne!(task.state, ExecutionState::Done);
}
