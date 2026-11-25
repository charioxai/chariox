//! Row 432: after a cold kernel restart a watched process that cannot be
//! supervised again is reported lost. The report must tell its owner, in plain
//! words, when the process was last seen, that it and the job behind it are
//! gone, and what to do next.
use super::*;

const HOUR_MS: u64 = 3_600_000;

/// A process wake an earlier kernel armed and started `age_ms` ago, whose
/// owner waits on it.
fn watched_before_the_restart(f: &Fixture, id: &str, age_ms: u64) -> AgentWake {
    let task = f.begin("turn-1");
    let mut wake = f.wake(&task, id);
    wake.kind = "process".into();
    wake.label = "cargo test unit".into();
    wake.command = vec!["systemctl".into(), "--wait".into()];
    wake.created_at_ms = crate::session::unix_epoch_ms() - age_ms;
    f.store()
        .agent_lifecycle(Operation::ArmWake {
            task: task.task_id.clone(),
            prompt: task.prompt_id.clone(),
            wake: Box::new(wake.clone()),
        })
        .unwrap();
    f.store()
        .agent_lifecycle(Operation::ProcessStarted {
            id: id.into(),
            pid: 4_000_000,
            now: wake.created_at_ms + 1_000,
        })
        .unwrap();
    f.wait_on(&task, &wake.registration_id);
    f.store().agent_wake(id).unwrap().unwrap()
}

fn lost_report(f: &Fixture) -> (String, String) {
    f.runtime
        .recover_lost_agent_processes(crate::session::unix_epoch_ms())
        .unwrap();
    let events = f.store().agent_inbox(&f.room, &f.agent, 0).unwrap();
    assert_eq!(events.len(), 1, "the waiting agent hears of the loss once");
    assert_eq!(events[0].payload["process_lost"], true, "{:?}", events[0]);
    (
        events[0].payload["output_tail"]
            .as_str()
            .unwrap()
            .to_owned(),
        f.notices(),
    )
}

// Cell 2: a watch lost across a cold restart says when it was last seen, that
// it is gone, and what to do.
#[tokio::test]
async fn a_watch_lost_across_a_restart_says_when_it_was_last_seen_that_it_is_gone_and_what_to_do() {
    let f = fixture("lost-process-cell2", false).await;
    watched_before_the_restart(&f, "watch-a", 3 * HOUR_MS);
    let (tail, notice) = lost_report(&f);
    for (what, text) in [("the answer", &tail), ("the room notice", &notice)] {
        assert!(
            text.contains("last seen") && text.contains("3 h"),
            "{what} says when the process was last seen: {text}"
        );
        assert!(
            text.contains("gone"),
            "{what} says the process is gone: {text}"
        );
        assert!(
            text.contains("start it again"),
            "{what} says what to do next: {text}"
        );
    }
}

// Cell 3: the process had already ended while the kernel was down. No process
// of the watch was left to stop, so the report says it ended with the kernel
// down and that its exit is unknown, rather than that the kernel stopped it.
#[tokio::test]
async fn a_watch_whose_process_ended_while_the_kernel_was_down_says_so() {
    let f = fixture("lost-process-cell3", false).await;
    watched_before_the_restart(&f, "watch-b", 2 * HOUR_MS);
    let (tail, notice) = lost_report(&f);
    for (what, text) in [("the answer", &tail), ("the room notice", &notice)] {
        assert!(
            text.contains("ended while the kernel was down"),
            "{what} says it ended while the kernel was down: {text}"
        );
        assert!(
            text.contains("exit is unknown"),
            "{what} says its exit is unknown: {text}"
        );
    }
}
