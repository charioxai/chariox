use std::sync::Arc;
use tokio::sync::watch;
use tokio::time::Instant;

#[derive(Clone)]
pub struct Scope(Arc<Inner>);
struct Inner {
    pub id: String,
    pub deadline: Instant,
    cancel: watch::Sender<Option<&'static str>>,
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
        *self.0.cancel.borrow()
    }
    pub fn cancel(&self, reason: &'static str) {
        self.0.cancel.send_if_modified(|value| {
            if value.is_none() {
                *value = Some(reason);
                true
            } else {
                false
            }
        });
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
                return reason;
            }
            if receiver.changed().await.is_err() {
                return "APP_INSTANCE_RESTARTED";
            }
        }
    }
}
