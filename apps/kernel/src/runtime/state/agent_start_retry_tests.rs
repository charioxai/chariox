//! A wake whose delivery cannot start its agent is retried with backoff, says so, and only
//! after the retry budget names the start error instead of a generic "did not start".
use super::agent_reply_tests::Fixture;
use crate::durable_state::agent_lifecycle::start_failures::START_ATTEMPTS;
use crate::durable_state::agent_lifecycle::{InboxEvent, Operation, Outcome, DELIVERY_TIMEOUT_MS};

impl Fixture {
    /// A due wake for the child, as a finished watched process makes one.
    fn due_wake(&self) -> InboxEvent {
        let Outcome::Event(wake) = self
            .runtime
            .owned
            .durable_state_store
            .agent_lifecycle(Operation::Occur(
                crate::durable_state::agent_lifecycle::occurrence(
                    &self.room,
                    &self.child,
                    "watched-process",
                    "watched-process-exit",
                    "source_completed",
                    serde_json::json!({"success": true}),
                ),
            ))
            .unwrap()
        else {
            panic!("event")
        };
        wake
    }

    fn sweep_at(&self, now: u64) {
        self.runtime
            .owned
            .durable_state_store
            .agent_lifecycle(Operation::Sweep {
                now,
                busy_recipients: vec![],
                vault_wait_recipients: vec![],
                held_work: vec![],
            })
            .unwrap();
    }

    /// The start keeps failing at every retry time until the attempts are spent.
    pub(super) async fn spend_start_attempts(&self, sequence: u64) {
        let store = &self.runtime.owned.durable_state_store;
        while store
            .agent_start_failure(sequence)
            .unwrap()
            .is_some_and(|f| f.attempts < START_ATTEMPTS)
        {
            store.agent_ledger_sql_for_test(&format!(
                "UPDATE agent_inbox_start_failures SET retry_at_ms=1 WHERE sequence={sequence}"
            ));
            self.runtime
                .deliver_agent_inbox(&self.room, &self.child)
                .await
                .unwrap_err();
        }
    }

    fn notices(&self) -> Vec<String> {
        self.runtime
            .owned
            .operational_history_store
            .load_session_history_entries(&self.room, Some(&self.child))
            .unwrap()
            .into_iter()
            .map(|entry| entry.text)
            .collect()
    }
}

/// WS1: the start fails once; the 120 s refusal budget must not give up on it, and the agent's
/// pane says it is retrying, with the attempt, the reason and the next time.
#[tokio::test]
async fn a_wake_whose_start_failed_is_retried_not_failed_after_the_old_window() {
    let f = Fixture::with_child_provider("dev-stub");
    let _moved = f.remove_workspace();
    let wake = f.due_wake();
    f.runtime
        .deliver_agent_inbox(&f.room, &f.child)
        .await
        .unwrap_err();
    let started = crate::session::unix_epoch_ms();

    f.sweep_at(started + DELIVERY_TIMEOUT_MS + 1_000);

    assert_eq!(
        f.state_of(wake.sequence),
        "pending",
        "the wake keeps retrying"
    );
    assert_eq!(
        f.blocked_delivery_tasks(),
        0,
        "no card while retries remain"
    );
    let said = f.notices().join("\n").to_lowercase();
    assert!(
        said.contains("attempt 1")
            && said.contains("does not exist")
            && said.contains("next retry"),
        "the pane shows the retry with attempt, reason and next time: {said}"
    );
}

/// WS2: the start never succeeds; once the retry budget is spent the delivery ends with the start
/// error named, not the generic "did not start".
#[tokio::test]
async fn a_wake_that_never_starts_ends_naming_the_start_error() {
    let f = Fixture::with_child_provider("dev-stub");
    let _moved = f.remove_workspace();
    let wake = f.due_wake();
    f.runtime
        .deliver_agent_inbox(&f.room, &f.child)
        .await
        .unwrap_err();
    let started = crate::session::unix_epoch_ms();

    f.spend_start_attempts(wake.sequence).await;
    f.sweep_at(started + 60 * 60_000);

    assert_eq!(f.state_of(wake.sequence), "failed");
    // One closing notice names the error and says there are no more retries.
    let closing = f
        .notices()
        .into_iter()
        .filter(|notice| notice.contains("No more retries"))
        .collect::<Vec<_>>();
    assert_eq!(closing.len(), 1, "one outcome: {closing:?}");
    assert!(
        closing[0].contains("does not exist"),
        "the outcome names the start error: {}",
        closing[0]
    );
}

/// WS1, end to end: the start fails, is not tried again before its time, then succeeds once the
/// cause is gone: the wake is delivered exactly once and no card ever asked the owner.
#[tokio::test]
async fn a_wake_whose_start_failed_is_delivered_once_when_the_cause_is_gone() {
    let f = Fixture::with_child_provider("dev-stub");
    let moved = f.remove_workspace();
    let wake = f.due_wake();
    f.runtime
        .deliver_agent_inbox(&f.room, &f.child)
        .await
        .unwrap_err();
    std::fs::rename(&moved, f._worktree.path()).unwrap();
    let said_once = f.notices().len();

    // Not before the time the pane named.
    f.runtime
        .deliver_agent_inbox(&f.room, &f.child)
        .await
        .unwrap();
    assert_eq!(f.state_of(wake.sequence), "pending");
    assert_eq!(
        f.notices().len(),
        said_once,
        "no attempt, so nothing new to say"
    );

    f.runtime
        .owned
        .durable_state_store
        .agent_ledger_sql_for_test(&format!(
            "UPDATE agent_inbox_start_failures SET retry_at_ms=1 WHERE sequence={}",
            wake.sequence
        ));
    f.runtime
        .deliver_agent_inbox(&f.room, &f.child)
        .await
        .unwrap();

    assert_eq!(f.state_of(wake.sequence), "handled");
    assert_eq!(f.blocked_delivery_tasks(), 0);
    assert!(f
        .runtime
        .owned
        .durable_state_store
        .agent_start_failure(wake.sequence)
        .unwrap()
        .is_none());
    assert_eq!(
        f.child_inbox()
            .iter()
            .filter(|e| e.sequence == wake.sequence)
            .count(),
        1,
        "delivered once"
    );
}

/// Review P1: a reply request whose start fails once is not failed at the first refusal. It is
/// delivered once the cause is gone, and nobody was told it failed.
#[tokio::test]
async fn a_reply_request_survives_a_start_that_fails_once_and_recovers() {
    let f = Fixture::with_child_provider("dev-stub");
    let moved = f.remove_workspace();
    let request = f.request("reply-after-retry");
    f.runtime
        .deliver_agent_inbox(&f.room, &f.child)
        .await
        .unwrap_err();
    assert_eq!(f.state_of(request.sequence), "pending");
    f.runtime.sweep_agent_lifecycle().await.unwrap();
    assert!(
        f.replies().is_empty(),
        "no failure outcome while attempts remain: {:?}",
        f.replies()
    );
    assert_eq!(f.state_of(request.sequence), "pending");

    std::fs::rename(&moved, f._worktree.path()).unwrap();
    f.runtime
        .owned
        .durable_state_store
        .agent_ledger_sql_for_test(&format!(
            "UPDATE agent_inbox_start_failures SET retry_at_ms=1 WHERE sequence={}",
            request.sequence
        ));
    f.runtime
        .deliver_agent_inbox(&f.room, &f.child)
        .await
        .unwrap();
    f.runtime.sweep_agent_lifecycle().await.unwrap();

    assert_ne!(f.state_of(request.sequence), "failed");
    assert!(
        f.replies().is_empty(),
        "the request was delivered, so no early failure outcome exists: {:?}",
        f.replies()
    );
    assert_eq!(
        f.child_inbox()
            .iter()
            .filter(|event| event.sequence == request.sequence)
            .count(),
        1,
        "delivered once"
    );
}

/// The other end: when the attempts are spent the request ends once, as failed, and its sender is
/// told once.
#[tokio::test]
async fn a_reply_request_ends_once_when_the_start_attempts_are_spent() {
    let f = Fixture::with_child_provider("dev-stub");
    let _moved = f.remove_workspace();
    let request = f.request("reply-never-starts");
    f.runtime
        .deliver_agent_inbox(&f.room, &f.child)
        .await
        .unwrap_err();
    assert!(f.replies().is_empty(), "attempts remain");

    f.spend_start_attempts(request.sequence).await;
    f.runtime.sweep_agent_lifecycle().await.unwrap();
    f.sweep_at(crate::session::unix_epoch_ms() + 60 * 60_000);
    f.runtime.sweep_agent_lifecycle().await.unwrap();

    assert_eq!(f.state_of(request.sequence), "failed");
    f.assert_gone(1);
}

/// Review P2: a start that waits on the Vault owner spends no attempt and says nothing more per
/// retry: the open unlock card was announced when it opened.
#[tokio::test]
async fn a_start_waiting_on_the_vault_owner_adds_no_retry_notice() {
    let f = Fixture::with_child_provider("dev-stub");
    let wake = f.due_wake();
    let waiting = crate::error::DaemonError::VaultUnlock {
        code: "vault_locked",
        message: "Vault locked: unlock to start (account work).".into(),
        retryable: true,
    };
    let said = f.notices().len();
    for _ in 0..3 {
        let retried = f
            .runtime
            .record_agent_start_failure(&f.room, &f.child, wake.sequence, &waiting)
            .unwrap();
        assert!(retried, "a start that waits on the owner is tried again");
    }
    assert_eq!(f.notices().len(), said, "no notice per retry");
    let failure = f
        .runtime
        .owned
        .durable_state_store
        .agent_start_failure(wake.sequence)
        .unwrap()
        .expect("the wait is recorded");
    assert_eq!(failure.attempts, 0, "no attempt was spent");
}
