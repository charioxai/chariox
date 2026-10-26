//! MP-08 / MP-09 / MP-10 / MP-11: live goal waits survive check-ins and empty native ends.
use super::*;

pub(super) fn start(f: &Fixture, due: u64) {
    f.apply(Operation::StartGoal {
        task: "p".into(),
        owner: "owner".into(),
        objective: "Wait for the requested timer, then report its receipt".into(),
        minutes: 10,
        now: 1,
    });
    f.create("timer", "timer", Some(due), None);
}

pub(super) fn yield_on(f: &Fixture, prompt: &str, now: u64, deadline: u64) {
    f.apply(Operation::Yield {
        task: "p".into(),
        prompt: prompt.into(),
        registrations: vec!["completion-timer".into()],
        cursor: 0,
        deadline,
        reason: "Requested timer remains armed".into(),
        now,
    });
}

pub(super) fn settle(f: &Fixture, prompt: &str, now: u64, has_answer: bool) {
    f.apply(Operation::Settle {
        room: "room".into(),
        agent: "agent".into(),
        prompt: prompt.into(),
        run: "run".into(),
        has_answer,
        cancelled: false,
        now,
    });
}

pub(super) fn begin(f: &Fixture, prompt: &str, now: u64) {
    f.apply(Operation::Begin {
        owner: "owner".into(),
        room: "room".into(),
        agent: "agent".into(),
        prompt: prompt.into(),
        run: Some("run".into()),
        now,
    });
}

pub(super) fn assess_waiting(f: &Fixture, now: u64, deadline: u64) {
    let check = f.task().pending_prompt_id.unwrap();
    begin(f, &check, now);
    f.apply(Operation::EvaluateGoal {
        task: "p".into(), prompt: check,
        result: serde_json::json!({"status":"waiting","reason":"Requested timer remains armed","registration_ids":["completion-timer"],"deadline_ms":deadline}).to_string(), now: now + 1,
    });
}

#[test]
fn a12_long_timer_survives_two_checkins_without_spending_progress_guard() {
    let f = Fixture::new();
    let due = 55 * 60_000 + 10;
    start(&f, due);
    yield_on(&f, "p", 20, due);
    settle(&f, "p", 30, true);
    assess_waiting(&f, 31, due);
    assert_eq!(f.task().state, ExecutionState::Waiting);
    assert_eq!(
        f.task().wait.as_ref().unwrap().deadline_ms,
        due,
        "check-in time must not replace the admitted source wait deadline"
    );
    for index in 0..2 {
        let now = f.task().goal.as_ref().unwrap().next_check_in_ms.unwrap();
        assert!(now < due);
        f.apply(Operation::Sweep {
            now,
            busy_recipients: vec![],
            held_work: vec![],
        });
        let event = f
            .inbox()
            .into_iter()
            .find(|e| e.state == "pending" && e.kind == "deadline_reached")
            .expect("one scheduled goal check-in");
        let prompt = format!("goal-checkin-{index}");
        f.apply(Operation::Attempt {
            room: "room".into(),
            agent: "agent".into(),
            sequence: event.sequence,
            prompt: prompt.clone(),
            target: None,
            work: Some("p".into()),
            run: Some("run".into()),
            now: now + 1,
        });
        f.apply(Operation::Receipt {
            room: "room".into(),
            agent: "agent".into(),
            sequence: event.sequence,
            state: "accepted".into(),
            now: now + 2,
        });
        begin(&f, &prompt, now + 3);
        f.apply(Operation::Ack {
            room: "room".into(),
            agent: "agent".into(),
            sequence: event.sequence,
            handled: true,
            now: now + 4,
        });
        yield_on(&f, &prompt, now + 5, due);
        settle(&f, &prompt, now + 6, true);
        assess_waiting(&f, now + 7, due);
        let task = f.task();
        assert_eq!(
            task.state,
            ExecutionState::Waiting,
            "check-in {index}: {}",
            task.reason
        );
        assert_eq!(
            task.no_progress_wakes, 0,
            "a live source still before its deadline is an allowed wait"
        );
        assert!(
            task.goal.as_ref().unwrap().next_check_in_ms.unwrap() >= now + 600_000,
            "do not burst through immediate repeated continuations"
        );
    }
    assert_eq!(f.wake("timer").state, "scheduled");
}

#[test]
fn a12_refused_checkin_does_not_erase_an_earlier_progress_guard_strike() {
    for fired_before_refusal in [false, true] {
        let f = Fixture::new();
        let due = 55 * 60_000 + 10;
        start(&f, due);
        settle(&f, "p", 30, true);
        let check = f.task().pending_prompt_id.unwrap();
        begin(&f, &check, 31);
        f.apply(Operation::EvaluateGoal {
            task: "p".into(),
            prompt: check,
            result: serde_json::json!({"status":"continue","reason":"one remaining work turn"})
                .to_string(),
            now: 32,
        });
        assert_eq!(f.task().no_progress_wakes, 1);
        let work = f.task().pending_prompt_id.unwrap();
        begin(&f, &work, 40);
        yield_on(&f, &work, 41, due);
        settle(&f, &work, 42, true);
        assess_waiting(&f, 43, due);
        let now = f.task().goal.as_ref().unwrap().next_check_in_ms.unwrap();
        f.apply(Operation::Sweep {
            now,
            busy_recipients: vec![],
            held_work: vec![],
        });
        let event = f
            .inbox()
            .into_iter()
            .find(|event| event.state == "pending" && event.kind == "deadline_reached")
            .unwrap();
        f.apply(Operation::Attempt {
            room: "room".into(),
            agent: "agent".into(),
            sequence: event.sequence,
            prompt: "refused-goal-checkin".into(),
            target: None,
            work: Some("p".into()),
            run: Some("run".into()),
            now: now + 1,
        });
        if fired_before_refusal {
            f.apply(Operation::FireWakes { now: due });
        }
        f.apply(Operation::Receipt {
            room: "room".into(),
            agent: "agent".into(),
            sequence: event.sequence,
            state: "rejected".into(),
            now: if fired_before_refusal {
                due + 1
            } else {
                now + 2
            },
        });
        assert_eq!(f.task().state, ExecutionState::Waiting);
        assert_eq!(
            f.task().no_progress_wakes,
            1,
            "an exempt check-in cannot refund an earlier strike, including after a source fires"
        );
    }
}
