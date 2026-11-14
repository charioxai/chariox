//! A turn that ends while the work it started still runs keeps waiting on it: the kernel knows
//! each source, so the agent is not made to yield on it by hand.
use super::tests::Fixture;
use super::*;

fn register_second(f: &Fixture) {
    f.apply(Operation::RegisterObligation {
        owner: "owner".into(),
        room: "room".into(),
        agent: "parent".into(),
        prompt: "p".into(),
        run: Some("run".into()),
        id: "obligation-2".into(),
        kind: "delegate".into(),
        resource: Some("child-2".into()),
        now: 1,
    });
}

#[test]
fn a_turn_ended_with_a_running_source_keeps_waiting_on_it() {
    for has_answer in [true, false] {
        let f = Fixture::new();
        f.begin("p");
        f.register();
        f.subscribe();
        let Outcome::Settled { task, correction } = f.settle("p", has_answer) else {
            panic!()
        };
        assert!(!correction, "answer={has_answer}");
        assert_eq!(task.state, ExecutionState::Waiting, "answer={has_answer}");
        assert!(!task.correction_used && task.pending_prompt_id.is_none());
        let wait = task.wait.expect("the kernel kept the wait");
        assert_eq!(wait.registration_ids, ["reg"]);
        assert_eq!(wait.deadline_ms, Some(3 + DEFAULT_WAIT_TIMEOUT_MS));
    }
}

#[test]
fn the_kept_wait_ends_with_its_source_like_an_explicit_yield() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    f.subscribe();
    f.settle("p", true);
    f.apply(Operation::SourceOutcome {
        now: 6,
        public_answer: None,
        room: "room".into(),
        source: "child".into(),
        occurrence: "result".into(),
        success: true,
    });
    let events = f.store.agent_inbox("room", "parent", 0).unwrap();
    assert_eq!(events.len(), 1, "the source's end reaches the agent");
    assert_eq!(events[0].kind, "source_completed");
    assert_eq!(f.task().state, ExecutionState::Waiting);
}

#[test]
fn a_source_with_nothing_to_report_it_is_still_corrected() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    let Outcome::Settled { task, correction } = f.settle("p", true) else {
        panic!()
    };
    assert!(correction);
    assert_ne!(task.state, ExecutionState::Waiting);
}

#[test]
fn one_running_source_does_not_cover_another_that_has_none() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    register_second(&f);
    f.subscribe();
    let Outcome::Settled { task, correction } = f.settle("p", true) else {
        panic!()
    };
    assert!(correction, "{task:?}");
    assert!(task.wait.is_none());
}

#[test]
fn a_source_that_ended_unread_is_left_to_the_agent_to_read() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    f.subscribe();
    f.apply(Operation::SourceOutcome {
        now: 2,
        public_answer: None,
        room: "room".into(),
        source: "child".into(),
        occurrence: "result".into(),
        success: true,
    });
    let Outcome::Settled { task, correction } = f.settle("p", true) else {
        panic!()
    };
    assert!(correction, "{task:?}");
    assert!(task.wait.is_none());
}
