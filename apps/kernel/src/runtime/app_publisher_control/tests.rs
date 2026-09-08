use super::*;
mod fixture;
mod terminal;
use fixture::Fixture;

#[test]
fn round_robin_selection_reaches_past_a_retrying_prefix() {
    let mut state = State::default();
    for id in 0..10 {
        state
            .entries
            .insert(("alice".into(), format!("{id:02}")), Entry::new());
    }
    let first = pump::ready_keys(&state, 3);
    assert_eq!(first.last().unwrap().1, "02");
    state.cursor = first.last().cloned();
    let next = pump::ready_keys(&state, 3);
    assert_eq!(next.first().unwrap().1, "03");
    state.cursor = Some(("alice".into(), "09".into()));
    assert_eq!(pump::ready_keys(&state, 1)[0].1, "00");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zero_agent_session_enrolls_only_after_its_owners_exact_human_decision() {
    let f = Fixture::new();
    assert_eq!(
        f.control()
            .begin(&f.state, "bob", "request", f.input())
            .await,
        Err(PublisherOperationError::Invalid)
    );
    let begin = f.begin("request").await;
    assert_eq!(begin.phase, PublisherOperationPhase::Pending);
    assert_eq!(f.begin("request").await, begin);
    assert!(f
        .control()
        .0
        .shared
        .store
        .list_app_publishers("alice")
        .unwrap()
        .is_empty());
    assert_eq!(
        f.control().status("bob", "request").await,
        Err(PublisherOperationError::NotFound)
    );
    f.until(|| f.waiting().is_some()).await;
    let interaction = f.waiting().unwrap();
    assert!(f
        .state
        .resolve_terminal_runtime_interaction(
            &f.session,
            &interaction,
            "approve",
            None,
            Some("bob")
        )
        .await
        .is_err());
    assert!(f
        .control()
        .0
        .shared
        .store
        .list_app_publishers("alice")
        .unwrap()
        .is_empty());
    f.state
        .resolve_terminal_runtime_interaction(
            &f.session,
            &interaction,
            "approve",
            None,
            Some("alice"),
        )
        .await
        .unwrap();
    f.until(|| f.phase("request") == Some(PublisherOperationPhase::Approved))
        .await;
    let approved = f.control().status("alice", "request").await.unwrap();
    assert!(approved.receipt.is_some());
    assert_eq!(
        f.control()
            .0
            .shared
            .store
            .list_app_publishers("alice")
            .unwrap()
            .len(),
        1
    );
    // Cancellation is not revocation, and replay is historical only.
    assert_eq!(
        f.control().cancel("alice", "request").await,
        Err(PublisherOperationError::Conflict)
    );
    f.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn declined_and_abandoned_prompts_never_enroll_a_key() {
    let f = Fixture::new();
    f.begin("declined").await;
    f.until(|| f.waiting().is_some()).await;
    f.state
        .resolve_terminal_runtime_interaction(
            &f.session,
            &f.waiting().unwrap(),
            "decline",
            None,
            Some("alice"),
        )
        .await
        .unwrap();
    f.until(|| f.phase("declined") == Some(PublisherOperationPhase::Denied))
        .await;
    f.begin("cancelled").await;
    f.until(|| f.waiting().is_some()).await;
    let interaction = f.waiting().unwrap();
    f.control().cancel("alice", "cancelled").await.unwrap();
    f.until(|| f.control().0.state.lock().unwrap().entries.is_empty())
        .await;
    assert!(f
        .state
        .resolve_terminal_runtime_interaction(
            &f.session,
            &interaction,
            "approve",
            None,
            Some("alice")
        )
        .await
        .is_err());
    assert!(f
        .control()
        .0
        .shared
        .store
        .list_app_publishers("alice")
        .unwrap()
        .is_empty());
    f.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recovery_arms_a_fresh_nonce_without_reusing_a_prior_human_wait() {
    let f = Fixture::new();
    f.begin("recover").await;
    f.until(|| f.waiting().is_some()).await;
    let old = f.waiting().unwrap();
    f.shutdown().await;
    let replacement = AppPublisherControl::new(
        f.control().0.shared.store.clone(),
        f.control().0.shared.admission.clone(),
    );
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            replacement.pump(&f.state).await;
            let waiting = replacement
                .0
                .state
                .lock()
                .unwrap()
                .entries
                .values()
                .find_map(|entry| match &entry.step {
                    Step::Waiting { challenge, .. } => Some(challenge.interaction_id().to_owned()),
                    _ => None,
                });
            if let Some(current) = waiting {
                assert_ne!(old, current);
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    // An old terminal ACK may be accepted before generic abandoned-wait
    // sweeping, but its closed responder cannot resolve the new challenge.
    let _ = f
        .state
        .resolve_terminal_runtime_interaction(&f.session, &old, "approve", None, Some("alice"))
        .await;
    assert_eq!(f.phase("recover"), Some(PublisherOperationPhase::Pending));
    assert!(f
        .control()
        .0
        .shared
        .store
        .list_app_publishers("alice")
        .unwrap()
        .is_empty());
    let handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || replacement.shutdown_blocking(handle))
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn due_recovery_scan_retains_a_slot_ahead_of_busy_foreground_entries() {
    let f = Fixture::new();
    for i in 0..8 {
        f.control()
            .notify(("alice".into(), format!("missing-{i}")), false);
    }
    f.control().pump(&f.state).await;
    let state = f.control().0.state.lock().unwrap();
    assert!(state.scanning);
    assert_eq!(state.jobs.len(), JOBS);
    assert_eq!(state.entries.values().filter(|e| e.busy).count(), JOBS - 1);
    drop(state);
    f.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disconnected_begin_remains_owned_and_rolls_back_after_shutdown() {
    disconnected_request_shutdown(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admitted_cancel_is_drained_durably_after_client_disconnect_and_shutdown() {
    disconnected_request_shutdown(true).await;
}

async fn disconnected_request_shutdown(cancelling: bool) {
    let f = Fixture::new();
    if cancelling {
        f.begin("request").await;
    }
    let mut blocked = rusqlite::Connection::open(f.control().0.shared.store.path()).unwrap();
    let transaction = blocked
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let (entered, receiving) = std::sync::mpsc::channel();
    let entered = Mutex::new(Some(entered));
    *f.control().0.request_checkpoint.lock().unwrap() = Some(Arc::new(move || {
        if let Some(sender) = entered.lock().unwrap().take() {
            sender.send(()).unwrap();
        }
    }));
    let state = f.state.clone();
    let input = f.input();
    let waiting = tokio::spawn(async move {
        let control = state.app_control().publishers();
        if cancelling {
            control.cancel("alice", "request").await
        } else {
            control.begin(&state, "alice", "request", input).await
        }
    });
    tokio::task::spawn_blocking(move || receiving.recv_timeout(Duration::from_secs(3)))
        .await
        .unwrap()
        .unwrap();
    // The first writer budget check has captured false before BEGIN's lock wait.
    waiting.abort();
    assert!(waiting.await.unwrap_err().is_cancelled());
    assert_eq!(f.control().0.state.lock().unwrap().requests.len(), 1);
    assert_eq!(f.control().0.shared.admission.available_permits(), 7);
    let control = f.control().clone();
    let handle = tokio::runtime::Handle::current();
    let mut shutdown = tokio::task::spawn_blocking(move || control.shutdown_blocking(handle));
    tokio::time::timeout(Duration::from_secs(2), async {
        while !f.control().0.stopped.load(Ordering::Acquire) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut shutdown)
            .await
            .is_err()
    );
    transaction.rollback().unwrap();
    tokio::time::timeout(Duration::from_secs(3), shutdown)
        .await
        .unwrap()
        .unwrap();
    assert!(f.control().0.state.lock().unwrap().requests.is_empty());
    assert_eq!(f.control().0.shared.admission.available_permits(), 8);
    assert_eq!(
        f.phase("request"),
        if cancelling {
            Some(PublisherOperationPhase::Cancelled)
        } else {
            None
        }
    );
}
