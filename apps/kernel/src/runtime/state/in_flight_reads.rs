//! Identical reads in flight share one read. A reconnecting web view asks for a
//! Room's state and history outline once per pane and once per attachment; on a
//! busy host each repeat costs the kernel the same work again (b3, 2026-11-18:
//! 59 outline reads in two minutes, each slower than the last).

use std::collections::HashMap;
use std::future::Future;
use std::sync::Mutex;

use tokio::sync::watch;

use crate::error::DaemonError;
use crate::local::LocalDaemonResponse;

/// A finished read: its response, or `None` when it failed. A failed read is not
/// shared; each waiting caller then reads for itself and gets its own error.
type Outcome = Option<Option<LocalDaemonResponse>>;

#[derive(Default)]
pub(crate) struct InFlightReads {
    reads: Mutex<HashMap<String, watch::Receiver<Outcome>>>,
}

/// Removes the leader's entry however the leader ends, cancelled included.
struct Leading<'a> {
    reads: &'a Mutex<HashMap<String, watch::Receiver<Outcome>>>,
    key: &'a str,
}

impl Drop for Leading<'_> {
    fn drop(&mut self) {
        self.reads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(self.key);
    }
}

impl InFlightReads {
    /// Runs `read` unless the same `key` is being read; then waits for that read
    /// and answers with its response. `key` must name everything the response
    /// depends on, the caller included.
    pub(crate) async fn read<F, Fut>(
        &self,
        key: String,
        read: F,
    ) -> Result<LocalDaemonResponse, DaemonError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<LocalDaemonResponse, DaemonError>>,
    {
        let joined = {
            let mut reads = self
                .reads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match reads.get(&key) {
                Some(leader) => Err(leader.clone()),
                None => {
                    let (finished, outcome) = watch::channel(None);
                    reads.insert(key.clone(), outcome);
                    Ok(finished)
                }
            }
        };
        match joined {
            Ok(finished) => {
                let _leading = Leading {
                    reads: &self.reads,
                    key: &key,
                };
                let result = read().await;
                finished.send_replace(Some(result.as_ref().ok().cloned()));
                result
            }
            Err(mut leader) => {
                let shared = leader
                    .wait_for(Option::is_some)
                    .await
                    .ok()
                    .and_then(|outcome| outcome.clone().flatten());
                match shared {
                    Some(response) => Ok(response),
                    // The leader failed or its caller left: read for this caller.
                    None => read().await,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn response() -> LocalDaemonResponse {
        LocalDaemonResponse::SessionHistoryOutline { agents: Vec::new() }
    }

    #[tokio::test]
    async fn identical_reads_in_flight_run_once_and_a_cancelled_leader_hands_over() {
        let reads = Arc::new(InFlightReads::default());
        let runs = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(tokio::sync::Notify::new());
        let read = |reads: Arc<InFlightReads>, key: &str| {
            let (runs, release, key) = (runs.clone(), release.clone(), key.to_string());
            async move {
                reads
                    .read(key, || async move {
                        runs.fetch_add(1, Ordering::SeqCst);
                        release.notified().await;
                        Ok(response())
                    })
                    .await
            }
        };
        let first = tokio::spawn(read(reads.clone(), "outline"));
        tokio::task::yield_now().await;
        let followers: Vec<_> = (0..4)
            .map(|_| tokio::spawn(read(reads.clone(), "outline")))
            .collect();
        let other = tokio::spawn(read(reads.clone(), "other room"));
        tokio::task::yield_now().await;
        assert_eq!(runs.load(Ordering::SeqCst), 2, "one read per distinct key");
        release.notify_waiters();
        first.await.unwrap().unwrap();
        for follower in followers {
            assert_eq!(follower.await.unwrap().unwrap(), response());
        }
        other.await.unwrap().unwrap();
        assert_eq!(runs.load(Ordering::SeqCst), 2);

        // A leader whose caller left hands the read to a waiting caller.
        let leader = tokio::spawn(read(reads.clone(), "outline"));
        tokio::task::yield_now().await;
        let waiting = tokio::spawn(read(reads.clone(), "outline"));
        tokio::task::yield_now().await;
        leader.abort();
        let _ = leader.await;
        tokio::task::yield_now().await;
        release.notify_waiters();
        assert_eq!(waiting.await.unwrap().unwrap(), response());
        assert_eq!(runs.load(Ordering::SeqCst), 4);
    }
}
