//! A plain turn (no goal, obligation or wait) is never held to a wait. Its end
//! needs a final answer: an empty end gets one correction, then blocks for the owner.
use super::tests::Fixture;
use super::*;

fn settled(outcome: Outcome) -> (AgentTaskExecution, bool) {
    match outcome {
        Outcome::Settled { task, correction } => (task, correction),
        other => panic!("{other:?}"),
    }
}

fn begin_correction(f: &Fixture, task: &AgentTaskExecution) -> String {
    let prompt = task.pending_prompt_id.clone().expect("correction prompt");
    f.apply(Operation::Begin {
        owner: "owner".into(),
        room: "room".into(),
        agent: "parent".into(),
        prompt: prompt.clone(),
        run: Some("run".into()),
        now: 4,
    });
    prompt
}

#[test]
fn plain_answered_turn_is_done_without_a_correction() {
    let f = Fixture::new();
    f.begin("p");
    let (task, correction) = settled(f.settle("p", true));
    assert!(!correction);
    assert_eq!(task.state, ExecutionState::Done);
    assert!(task.pending_prompt_id.is_none() && !task.correction_used);
    let (again, correction) = settled(f.settle("p", true));
    assert!(!correction);
    assert_eq!(again.state, ExecutionState::Done);
}

#[test]
fn empty_final_answer_is_corrected_once_and_done_after_a_real_answer() {
    let f = Fixture::new();
    f.begin("p");
    let (task, correction) = settled(f.settle("p", false));
    assert!(correction);
    assert_eq!(task.state, ExecutionState::Working, "{task:?}");
    assert!(task.correction_used);
    assert!(task.reason.contains("final answer"), "{}", task.reason);
    let prompt = begin_correction(&f, &task);
    assert!(goals::is_correction_prompt_id(&prompt));
    let (task, correction) = settled(f.settle(&prompt, true));
    assert!(!correction);
    assert_eq!(task.state, ExecutionState::Done, "{task:?}");
}

#[test]
fn second_empty_final_answer_blocks_the_task_for_the_owner() {
    let f = Fixture::new();
    f.begin("p");
    let (task, _) = settled(f.settle("p", false));
    let prompt = begin_correction(&f, &task);
    let (task, correction) = settled(f.settle(&prompt, false));
    assert!(!correction);
    assert_eq!(task.state, ExecutionState::Blocked, "{task:?}");
    assert_eq!(task.reason, goals::EMPTY_FINAL_ANSWER_REASON);
    assert!(task.blocked_revision > 0 && task.pending_prompt_id.is_none());
    // A duplicate end of the same turn neither spends another correction nor completes it.
    let (again, correction) = settled(f.settle(&prompt, false));
    assert!(!correction);
    assert_eq!(again.state, ExecutionState::Blocked);
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

#[test]
fn agent_archive_cancels_parent_participation_without_cancelling_child_work() {
    let f = Fixture::new();
    f.begin("p");
    f.register();
    let Outcome::Task(child) = f.apply(Operation::Begin {
        owner: "owner".into(),
        room: "room".into(),
        agent: "child".into(),
        prompt: "child-turn".into(),
        run: Some("child-run".into()),
        now: 2,
    }) else {
        panic!("child task")
    };
    let parent = f.task();
    let Outcome::Task(archived) = f.apply(Operation::ArchiveTask {
        task: parent.task_id,
        owner: parent.owner_user_id,
        revision: parent.revision,
    }) else {
        panic!("archived task")
    };
    assert_eq!(archived.state, ExecutionState::Cancelled);
    assert!(archived.reason.contains("Agent archived"));
    assert!(archived.stopped_by_owner());
    assert!(archived
        .obligations
        .iter()
        .all(|o| o.kind != "delegate" || o.dispatch_state != "cancel_requested"));
    let retained = f.store.agent_tasks(Some("room"), Some("child")).unwrap();
    assert_eq!(retained[0], child);
}

#[test]
fn agent_archive_cancels_handoff_and_late_completion_does_not_reopen_task() {
    let f = Fixture::new();
    f.begin("p");
    f.apply(Operation::RegisterObligation {
        owner: "owner".into(),
        room: "room".into(),
        agent: "parent".into(),
        prompt: "p".into(),
        run: Some("run".into()),
        id: "handoff".into(),
        kind: "hand_off".into(),
        resource: Some("handoff-source".into()),
        now: 1,
    });
    f.apply(Operation::DispatchReceipt {
        id: "handoff".into(),
        accepted: true,
        resource: Some("handoff-source".into()),
    });
    let task = f.task();
    f.apply(Operation::ArchiveTask {
        task: task.task_id,
        owner: task.owner_user_id,
        revision: task.revision,
    });
    f.apply(Operation::SourceOutcome {
        public_answer: None,
        room: "room".into(),
        source: "handoff-source".into(),
        occurrence: "late".into(),
        success: true,
        now: 3,
    });
    let task = f.task();
    assert!(task.stopped_by_owner());
    assert!(task.obligations.iter().all(|o| o.status == "cancelled"));
    assert!(f
        .store
        .agent_registrations("p")
        .unwrap()
        .iter()
        .all(|r| !r.live));
}
