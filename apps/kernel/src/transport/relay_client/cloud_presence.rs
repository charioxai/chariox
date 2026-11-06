//! Cloud presence publication: one request at a time, newest intent wins.
//!
//! A blocking HTTP request cannot be cancelled once sent, so overlapping
//! publishes could land out of order and leave Cloud showing an online kernel
//! offline. One worker sends the newest intent, retries it promptly after a
//! failure, and drops intents a newer one has replaced.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::runtime::router::CommandRouter;

// The final offline intent can be behind one request already in flight.
const CLOUD_PRESENCE_SHUTDOWN_TIMEOUT: Duration =
    crate::runtime::cloud_api_client::CLOUD_PRESENCE_REQUEST_TIMEOUT
        .saturating_add(Duration::from_secs(1))
        .saturating_mul(2);

const CLOUD_PRESENCE_RETRY_BASE_DELAY: Duration = Duration::from_secs(1);
const CLOUD_PRESENCE_RETRY_MAX_DELAY: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
struct PresenceIntent {
    online: bool,
    reason: String,
}

pub(super) struct CloudPresencePublisher {
    intent: watch::Sender<Option<PresenceIntent>>,
    worker: JoinHandle<()>,
}

impl CloudPresencePublisher {
    pub(super) fn start(router: Arc<CommandRouter>) -> Self {
        Self::start_with(move |online, reason| {
            let router = Arc::clone(&router);
            async move {
                match super::connection_state::publish_cloud_presence(&router, online, &reason)
                    .await
                {
                    super::connection_state::CloudPresencePublishOutcome::Published => Ok(()),
                    outcome => Err(format!("{outcome:?}")),
                }
            }
        })
    }

    fn start_with<P, F>(publish: P) -> Self
    where
        P: Fn(bool, String) -> F + Send + Sync + 'static,
        F: Future<Output = Result<(), String>> + Send + 'static,
    {
        let (intent, intents) = watch::channel(None);
        let worker = tokio::spawn(run_presence_worker(intents, publish));
        Self { intent, worker }
    }

    /// Replace whatever is pending with this state; it is sent next.
    pub(super) fn publish(&self, online: bool, reason: impl Into<String>) {
        self.intent.send_replace(Some(PresenceIntent {
            online,
            reason: reason.into(),
        }));
    }

    /// Drain the final intent before the runtime drops its tasks on shutdown.
    pub(super) async fn finish(self) {
        let Self { intent, mut worker } = self;
        drop(intent);
        match tokio::time::timeout(CLOUD_PRESENCE_SHUTDOWN_TIMEOUT, &mut worker).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => crate::logging::warn_with_fields(
                "daemon.relay_client",
                "cloud presence worker failed during shutdown",
                serde_json::json!({ "error": error.to_string() }),
            ),
            Err(_) => {
                worker.abort();
                let _ = worker.await;
                crate::logging::warn_with_fields(
                    "daemon.relay_client",
                    "cloud presence shutdown timed out",
                    serde_json::json!({
                        "timeout_ms": CLOUD_PRESENCE_SHUTDOWN_TIMEOUT.as_millis(),
                    }),
                );
            }
        }
    }
}

async fn run_presence_worker<P, F>(mut intents: watch::Receiver<Option<PresenceIntent>>, publish: P)
where
    P: Fn(bool, String) -> F,
    F: Future<Output = Result<(), String>>,
{
    // An intent sent just before the publisher drops (shutdown) still goes
    // out once; retries end with the publisher.
    while intents.changed().await.is_ok() {
        let Some(intent) = intents.borrow_and_update().clone() else {
            continue;
        };
        let mut attempt = 0_u32;
        while publish_once(&publish, &intent, attempt).await.is_err() {
            let delay = presence_retry_delay(attempt);
            attempt = attempt.saturating_add(1);
            tokio::select! {
                changed = intents.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    // A newer intent replaces this one; the outer loop takes it.
                    intents.mark_changed();
                    break;
                }
                _ = tokio::time::sleep(delay) => {}
            }
        }
    }
}

async fn publish_once<P, F>(publish: &P, intent: &PresenceIntent, attempt: u32) -> Result<(), ()>
where
    P: Fn(bool, String) -> F,
    F: Future<Output = Result<(), String>>,
{
    publish(intent.online, intent.reason.clone())
        .await
        .map_err(|outcome| {
            crate::logging::warn_with_fields(
                "daemon.relay_client",
                "cloud presence publish will retry",
                serde_json::json!({
                    "online": intent.online,
                    "reason": intent.reason,
                    "outcome": outcome,
                    "attempt": attempt,
                    "retry_ms": presence_retry_delay(attempt).as_millis(),
                }),
            );
        })
}

fn presence_retry_delay(attempt: u32) -> Duration {
    CLOUD_PRESENCE_RETRY_BASE_DELAY
        .saturating_mul(1_u32 << attempt.min(3))
        .min(CLOUD_PRESENCE_RETRY_MAX_DELAY)
}

#[cfg(test)]
mod tests;
