//! MP-08 / MP-09 / MP-10 / MP-11 A12-S05: assessor references remain task-bound.
use super::goal_waits::{begin, settle, start};
use super::*;

const WAKE: &str = "wake-a9eedf22677192661f03c4b44171fc0b";

#[test]
fn a12_assessment_unique_live_prefix_preserves_the_original_timer() {
    let f = Fixture::new();
    let due = 4_080_010;
    start(&f, due);
    let wake = f.create(WAKE, "timer", Some(due), None);
    f.apply(Operation::Yield {
        task: "p".into(),
        prompt: "p".into(),
        registrations: vec!["completion-timer".into(), wake.registration_id.clone()],
        cursor: 0,
        deadline: due + 300_000,
        reason: "wait for timers".into(),
        now: 20,
    });
    settle(&f, "p", 30, true);
    let check = f.task().pending_prompt_id.unwrap();
    begin(&f, &check, 31);
    let shortened = &wake.registration_id[..wake.registration_id.len() - 1];
    f.apply(Operation::EvaluateGoal {
        task: "p".into(), prompt: check,
        result: serde_json::json!({"status":"waiting","reason":"same timer remains live","registration_ids":["completion-timer",shortened],"deadline_ms":due+300_000}).to_string(), now: 32,
    });
    let task = f.task();
    assert_eq!(task.state, ExecutionState::Waiting, "{}", task.reason);
    assert!(task
        .wait
        .as_ref()
        .unwrap()
        .registration_ids
        .contains(&wake.registration_id));
    assert_eq!(
        task.wait.unwrap().deadline_ms,
        due,
        "resolve before source deadline lookup"
    );
    assert_eq!(
        f.wake(WAKE),
        wake,
        "assessment must preserve the original scheduled timer"
    );
}

#[test]
fn a12_assessment_ambiguous_live_prefix_never_selects_a_source() {
    let f = Fixture::new();
    start(&f, 4_080_010);
    f.create("wake-shared-a", "timer", Some(4_080_010), None);
    f.create("wake-shared-b", "timer", Some(4_080_010), None);
    settle(&f, "p", 30, true);
    let check = f.task().pending_prompt_id.unwrap();
    begin(&f, &check, 31);
    f.apply(Operation::EvaluateGoal {
        task: "p".into(), prompt: check,
        result: serde_json::json!({"status":"waiting","reason":"ambiguous reference","registration_ids":["completion-wake-shared-"],"deadline_ms":60_000}).to_string(), now: 32,
    });
    assert_eq!(f.task().state, ExecutionState::Blocked);
}

#[test]
fn a12_assessment_unique_prefix_cannot_skip_another_obligation() {
    let f = Fixture::new();
    start(&f, 4_080_010);
    f.create(WAKE, "timer", Some(4_080_010), None);
    settle(&f, "p", 30, true);
    let check = f.task().pending_prompt_id.unwrap();
    begin(&f, &check, 31);
    f.apply(Operation::EvaluateGoal {
        task: "p".into(), prompt: check,
        result: serde_json::json!({"status":"waiting","reason":"only one source selected","registration_ids":["completion-wake-a9eedf22677192661f03c4b44171fc0"],"deadline_ms":60_000}).to_string(), now: 32,
    });
    assert_eq!(f.task().state, ExecutionState::Blocked);
}

#[test]
fn a12_assessment_exact_inactive_id_cannot_rebind_to_a_live_sibling() {
    let f = Fixture::new();
    f.apply(Operation::StartGoal {
        task: "p".into(),
        owner: "owner".into(),
        objective: "Wait only on the selected source".into(),
        minutes: 10,
        now: 1,
    });
    let inactive = "completion-wake-abc";
    for id in [inactive, "completion-wake-abcd"] {
        f.apply(Operation::Subscribe {
            task: "p".into(),
            prompt: "p".into(),
            registration: Registration {
                id: id.into(),
                task_id: "p".into(),
                source_id: id.into(),
                obligation_id: None,
                source_cursor: 0,
                live: true,
            },
        });
    }
    f.apply(Operation::Unsubscribe {
        task: "p".into(),
        prompt: "p".into(),
        registration: inactive.into(),
    });
    settle(&f, "p", 30, true);
    let check = f.task().pending_prompt_id.unwrap();
    begin(&f, &check, 31);
    f.apply(Operation::EvaluateGoal {task:"p".into(),prompt:check,result:serde_json::json!({"status":"waiting","reason":"the exact selected source","registration_ids":[inactive],"deadline_ms":60_000}).to_string(),now:32});
    assert_eq!(
        f.task().state,
        ExecutionState::Blocked,
        "an inactive exact reference must never select a different timer"
    );
}
