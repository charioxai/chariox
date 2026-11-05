use std::{sync::Arc, time::Duration};
use tokio::sync::watch;
use tokio::time::Instant;

#[derive(Clone)]
pub struct Scope(Arc<Inner>);
struct Inner {
    pub id: String,
    pub deadline: Instant,
    cancel: watch::Sender<Option<Cancellation>>,
}
#[derive(Clone, Copy)]
struct Cancellation {
    reason: &'static str,
    at: Instant,
}
impl Scope {
    pub fn new(id: String, deadline: Instant) -> Self {
        Self(Arc::new(Inner {
            id,
            deadline,
            cancel: watch::channel(None).0,
        }))
    }
    pub fn id(&self) -> &str {
        &self.0.id
    }
    pub fn deadline(&self) -> Instant {
        self.0.deadline
    }
    pub fn reason(&self) -> Option<&'static str> {
        self.0.cancel.borrow().map(|value| value.reason)
    }
    pub fn cancel(&self, reason: &'static str) {
        self.0.cancel.send_if_modified(|value| {
            if value.is_none() {
                *value = Some(Cancellation {
                    reason,
                    at: Instant::now(),
                });
                true
            } else {
                false
            }
        });
    }
    pub(crate) fn expired_cancellation(&self, grace: Duration) -> Option<&'static str> {
        self.0
            .cancel
            .borrow()
            .filter(|value| value.reason != "APP_CALL_FINISHED" && value.at.elapsed() >= grace)
            .map(|value| value.reason)
    }
    pub fn finish(&self) {
        self.cancel("APP_CALL_FINISHED");
    }
    /// Normal completion releases resources without resolving the guest's
    /// cancellation future. That future ends when the guest drops its reader.
    pub async fn guest_cancelled(&self) -> &'static str {
        let reason = self.cancelled().await;
        if reason == "APP_CALL_FINISHED" {
            std::future::pending().await
        } else {
            reason
        }
    }
    pub async fn cancelled(&self) -> &'static str {
        let mut receiver = self.0.cancel.subscribe();
        loop {
            if let Some(reason) = *receiver.borrow_and_update() {
                return reason.reason;
            }
            if receiver.changed().await.is_err() {
                return "APP_INSTANCE_RESTARTED";
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn repeated_cancellation_does_not_renew_the_cleanup_grace() {
        let scope = Scope::new("test".into(), Instant::now() + Duration::from_secs(1));
        scope.cancel("CANCELLED");
        assert_eq!(scope.expired_cancellation(Duration::from_secs(1)), None);
        tokio::time::sleep(Duration::from_millis(10)).await;
        scope.cancel("DEADLINE_EXCEEDED");
        scope.finish();
        assert_eq!(scope.reason(), Some("CANCELLED"));
        assert_eq!(
            scope.expired_cancellation(Duration::from_millis(5)),
            Some("CANCELLED")
        );
        let completed = Scope::new("done".into(), Instant::now());
        completed.finish();
        assert_eq!(completed.expired_cancellation(Duration::ZERO), None);
    }
}
