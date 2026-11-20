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

    fn blocked_delivery_reasons(&self) -> Vec<String> {
        self.runtime
            .owned
            .durable_state_store
            .agent_tasks(Some(&self.room), Some(&self.child))
            .unwrap()
            .into_iter()
            .filter(|t| t.task_id.starts_with("delivery-"))
            .map(|t| t.reason)
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
    let reasons = f.blocked_delivery_reasons();
    assert_eq!(reasons.len(), 1, "one outcome: {reasons:?}");
    assert!(
        reasons[0].contains("does not exist")
            && !reasons[0].ends_with("the agent did not start. Resume sends it again, or Cancel"),
        "the outcome names the start error: {}",
        reasons[0]
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
