//! A delivery whose agent cannot start keeps its attempts across a restart and ends only after
//! the attempts are spent, naming the start error.
use super::start_failures::{describe_start_error, retry_pause_ms, START_ATTEMPTS};
use super::*;
use tests::Fixture;

const USAGE: &str = "codex account `lanes-codex`: usage allowance and credits are exhausted; select another account in Provider Accounts";

/// A message the agent never started for: attempted, then refused at `now`.
fn fail_start(f: &Fixture, sequence: u64, error: &str, now: u64) -> InboxEvent {
    f.apply(Operation::Attempt {
        room: "room".into(),
        agent: "parent".into(),
        sequence,
        prompt: format!("prompt-{now}"),
        target: None,
        run: None,
        now,
        work: None,
        through: sequence,
    });
    let Outcome::Event(e) = f.apply(Operation::StartFailed {
        room: "room".into(),
        agent: "parent".into(),
        sequence,
        error: error.into(),
        now,
    }) else {
        panic!("event")
    };
    e
}

fn pending_message(f: &Fixture) -> InboxEvent {
    f.begin("p");
    let Outcome::Event(e) = f.apply(Operation::Occur(occurrence(
        "room",
        "parent",
        "peer",
        "one",
        "message",
        serde_json::json!({"message":"result"}),
    ))) else {
        panic!("event")
    };
    e
}

fn sweep(f: &Fixture, now: u64) {
    f.apply(Operation::Sweep {
        now,
        busy_recipients: vec![],
        vault_wait_recipients: vec![],
        held_work: vec![],
    });
}

/// WS3: the attempts, the last error and the next try are kept through a restart, and the
/// delivery goes on counting from them.
#[test]
fn a_failed_start_keeps_its_attempts_through_a_restart() {
    let f = Fixture::new();
    let e = pending_message(&f);
    let t = 1_000_000;
    assert_eq!(
        fail_start(&f, e.sequence, "no such directory", t).state,
        "pending"
    );

    let reopened = DurableKernelStateStore::open(f.root.join("state.sqlite")).unwrap();
    let kept = reopened.agent_start_failure(e.sequence).unwrap().unwrap();
    assert_eq!(
        (kept.attempts, kept.error.as_str(), kept.retry_at_ms),
        (1, "no such directory", t + retry_pause_ms(1))
    );
    drop(reopened);

    fail_start(&f, e.sequence, "no such directory", t + 40_000);
    let kept = f.store.agent_start_failure(e.sequence).unwrap().unwrap();
    assert_eq!(kept.attempts, 2);
    assert_eq!(kept.retry_at_ms, t + 40_000 + retry_pause_ms(2));
    assert_eq!(f.store.agent_inbox("room", "parent", 0).unwrap().len(), 1);
}

/// WS1/WS2: the old 120 s window does not end a failing start; the attempts do.
#[test]
fn a_failing_start_ends_after_its_attempts_not_after_the_old_window() {
    let f = Fixture::new();
    let e = pending_message(&f);
    let t = 1_000_000;
    fail_start(&f, e.sequence, "no such directory", t);

    sweep(&f, t + DELIVERY_TIMEOUT_MS + 1_000);
    let row = f.store.agent_inbox("room", "parent", 0).unwrap().remove(0);
    assert_eq!(row.state, "pending", "still retrying");
    assert!(f
        .store
        .agent_tasks(Some("room"), Some("parent"))
        .unwrap()
        .iter()
        .all(|t| !t.task_id.starts_with("delivery-")));

    let mut at = t;
    for _ in 1..START_ATTEMPTS {
        at += retry_pause_ms(1);
        fail_start(&f, e.sequence, "no such directory", at);
    }
    sweep(&f, at + 1_000);
    let row = f.store.agent_inbox("room", "parent", 0).unwrap().remove(0);
    assert_eq!(row.state, "failed");
    let cards: Vec<_> = f
        .store
        .agent_tasks(Some("room"), Some("parent"))
        .unwrap()
        .into_iter()
        .filter(|t| t.task_id.starts_with("delivery-"))
        .collect();
    assert_eq!(cards.len(), 1);
    assert!(
        cards[0].reason.contains("no such directory")
            && cards[0]
                .reason
                .contains(&format!("{START_ATTEMPTS} attempts")),
        "{}",
        cards[0].reason
    );
    assert!(
        f.store.agent_start_failure(e.sequence).unwrap().is_none(),
        "the ended delivery keeps no retry state"
    );
}

/// A busy agent is not a failing start: the retry state goes.
#[test]
fn an_agent_that_became_busy_drops_its_start_failure() {
    let f = Fixture::new();
    let e = pending_message(&f);
    fail_start(&f, e.sequence, "no such directory", 1_000_000);
    f.apply(Operation::Sweep {
        now: 1_000_000 + 1_000,
        busy_recipients: vec![("room".into(), "parent".into())],
        vault_wait_recipients: vec![],
        held_work: vec![],
    });
    assert!(f.store.agent_start_failure(e.sequence).unwrap().is_none());
}

#[test]
fn the_pause_between_starts_grows_and_stops_growing() {
    let pauses: Vec<u64> = (1..=8).map(retry_pause_ms).collect();
    assert_eq!(pauses[..5], [30_000, 60_000, 120_000, 240_000, 480_000]);
    assert!(pauses[5..].iter().all(|p| *p == 480_000), "{pauses:?}");
}

/// WS4: a provider usage limit is named as one, with its reset time when the provider gave it.
#[test]
fn a_usage_limited_provider_is_named_as_one() {
    let said = describe_start_error(USAGE, 1_000_000);
    assert!(said.starts_with("provider usage limit"), "{said}");
    assert!(
        said.contains("usage allowance and credits are exhausted"),
        "{said}"
    );

    let with_reset = describe_start_error(
        "You've hit your usage limit. resets Dec 1, 4pm (UTC)",
        1_795_000_000_000,
    );
    assert!(
        with_reset.starts_with("provider usage limit, resets Dec 1, 16:00 UTC"),
        "{with_reset}"
    );

    assert_eq!(
        describe_start_error("working directory does not exist.", 1),
        "working directory does not exist"
    );
}

#[test]
fn a_delivery_that_never_starts_on_a_usage_limit_ends_naming_it() {
    let f = Fixture::new();
    let e = pending_message(&f);
    let mut at = 1_000_000;
    for _ in 0..START_ATTEMPTS {
        fail_start(&f, e.sequence, USAGE, at);
        at += retry_pause_ms(1);
    }
    sweep(&f, at);
    let cards: Vec<_> = f
        .store
        .agent_tasks(Some("room"), Some("parent"))
        .unwrap()
        .into_iter()
        .filter(|t| t.task_id.starts_with("delivery-"))
        .collect();
    assert_eq!(cards.len(), 1);
    assert!(
        cards[0].reason.contains("provider usage limit"),
        "{}",
        cards[0].reason
    );
    assert!(
        !cards[0].reason.contains("the agent did not start."),
        "{}",
        cards[0].reason
    );
}
