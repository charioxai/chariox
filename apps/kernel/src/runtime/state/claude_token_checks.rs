//! MP-08 / MP-10 / MP-11: first-use verdicts, deduplicated per account.
use crate::provider::ClaudeCredentialCheckError;
use std::{
    collections::BTreeMap,
    future::Future,
    sync::{Arc, Mutex},
};
use tokio::sync::OnceCell;

type Verdict = Result<(), ClaudeCredentialCheckError>;

#[derive(Default)]
pub(super) struct ClaudeTokenChecks(Mutex<BTreeMap<String, Arc<OnceCell<Verdict>>>>);

impl ClaudeTokenChecks {
    pub(super) async fn invalidate(&self, id: &str) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|key, _| key != id && !key.starts_with(&format!("{id}:")));
    }

    pub(super) fn is_rejected(&self, id: &str) -> bool {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(id)
            .and_then(|cell| cell.get())
            .is_some_and(|verdict| matches!(verdict, Err(ClaudeCredentialCheckError::Rejected)))
    }

    pub(super) async fn check<F, V>(&self, id: &str, verify: F) -> Option<Verdict>
    where
        F: FnOnce() -> V,
        V: Future<Output = Verdict>,
    {
        let check = {
            let mut checks = self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            checks.entry(id.to_string()).or_default().clone()
        };
        // The official CLI may wait on the network for 90 seconds. Only this
        // account's cell is held across that await, never the kernel cache lock.
        let checked = check.get_or_init(verify).await.clone();
        let checks = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Invalidation replaces the credential generation without waiting for
        // its old verifier. Its verdict cannot authenticate the replacement.
        checks
            .get(id)
            .filter(|current| Arc::ptr_eq(current, &check))
            .map(|_| checked)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use tokio::sync::{oneshot, Notify};
    use tokio::time::{timeout, Duration};

    async fn stalled(
        checks: Arc<ClaudeTokenChecks>,
    ) -> (
        tokio::task::JoinHandle<Option<Verdict>>,
        oneshot::Sender<()>,
    ) {
        let started = Arc::new(Notify::new());
        let signal = started.clone();
        let (release, wait) = oneshot::channel();
        let task = tokio::spawn(async move {
            checks
                .check("account-A", || async {
                    signal.notify_one();
                    let _ = wait.await;
                    Ok(())
                })
                .await
        });
        started.notified().await;
        (task, release)
    }

    #[tokio::test]
    async fn rejection_does_not_poison_a_transferred_credential_renewal() {
        let checks = ClaudeTokenChecks::default();
        assert_eq!(
            checks
                .check("account-A:old", || async {
                    Err(ClaudeCredentialCheckError::Rejected)
                })
                .await,
            Some(Err(ClaudeCredentialCheckError::Rejected))
        );
        assert_eq!(
            checks.check("account-A:new", || async { Ok(()) }).await,
            Some(Ok(()))
        );
        assert_eq!(
            checks
                .check("account-A", || async {
                    Err(ClaudeCredentialCheckError::Rejected)
                })
                .await,
            Some(Err(ClaudeCredentialCheckError::Rejected))
        );
        assert!(checks.is_rejected("account-A"));
        checks.invalidate("account-A").await;
        assert!(!checks.is_rejected("account-A"));
        assert_eq!(
            checks.check("account-A:old", || async { Ok(()) }).await,
            Some(Ok(()))
        );
        assert_eq!(
            checks
                .check("account-A:new", || async {
                    Err(ClaudeCredentialCheckError::Rejected)
                })
                .await,
            Some(Err(ClaudeCredentialCheckError::Rejected))
        );
    }

    #[tokio::test]
    async fn mp08_other_account_verifies_while_first_account_stalls() {
        let checks = Arc::new(ClaudeTokenChecks::default());
        let (task, release) = stalled(checks.clone()).await;
        let other = timeout(
            Duration::from_millis(200),
            checks.check("account-B", || async { Ok(()) }),
        )
        .await;
        let _ = release.send(());
        let _ = task.await;
        assert_eq!(
            other.expect("account A must not block account B"),
            Some(Ok(()))
        );
    }

    #[tokio::test]
    async fn mp08_replacement_invalidation_does_not_wait_for_another_account() {
        let checks = Arc::new(ClaudeTokenChecks::default());
        let (task, release) = stalled(checks.clone()).await;
        let invalidated = timeout(Duration::from_millis(200), checks.invalidate("account-B")).await;
        let _ = release.send(());
        let _ = task.await;
        invalidated.expect("replacement B must not wait for verifier A");
    }

    #[tokio::test]
    async fn mp08_invalidated_verifier_cannot_publish_a_replacement_verdict() {
        let checks = Arc::new(ClaudeTokenChecks::default());
        let (task, release) = stalled(checks.clone()).await;
        let invalidated = timeout(Duration::from_millis(200), checks.invalidate("account-A")).await;
        let _ = release.send(());
        let old = task.await.unwrap();
        invalidated.expect("replacement must not wait for the old verifier");
        assert_eq!(
            old, None,
            "old credentials cannot authenticate a replacement"
        );
        assert_eq!(
            checks
                .check("account-A", || async {
                    Err(ClaudeCredentialCheckError::Rejected)
                })
                .await,
            Some(Err(ClaudeCredentialCheckError::Rejected))
        );
    }

    #[tokio::test]
    async fn mp08_same_account_in_flight_verification_is_deduplicated() {
        let checks = Arc::new(ClaudeTokenChecks::default());
        let (first, release) = stalled(checks.clone()).await;
        let extra_calls = Arc::new(AtomicUsize::new(0));
        let calls = extra_calls.clone();
        let mut second = tokio::spawn(async move {
            checks
                .check("account-A", || async {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
                .await
        });
        assert!(
            timeout(Duration::from_millis(50), &mut second)
                .await
                .is_err(),
            "the same credential waits for its in-flight check"
        );
        assert_eq!(extra_calls.load(Ordering::SeqCst), 0);
        let _ = release.send(());
        assert_eq!(first.await.unwrap(), Some(Ok(())));
        assert_eq!(second.await.unwrap(), Some(Ok(())));
        assert_eq!(extra_calls.load(Ordering::SeqCst), 0);
    }
}
