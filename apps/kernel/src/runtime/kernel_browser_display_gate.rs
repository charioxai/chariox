//! MD-DISPLAY-04: bounded capture admission yields to pending source input.
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore};

pub(crate) struct DisplayGate {
    capture: Arc<Semaphore>,
    input: AtomicUsize,
    changed: Notify,
}
impl Default for DisplayGate {
    fn default() -> Self {
        Self {
            capture: Arc::new(Semaphore::new(1)),
            input: AtomicUsize::new(0),
            changed: Notify::new(),
        }
    }
}
impl DisplayGate {
    pub(crate) fn input(self: &Arc<Self>) -> InputAdmission {
        self.input.fetch_add(1, Ordering::AcqRel);
        InputAdmission(self.clone())
    }
    pub(crate) async fn capture(&self) -> Result<OwnedSemaphorePermit, String> {
        let at = std::time::Instant::now();
        let permit = self
            .capture
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| "MD-DISPLAY: capture gate closed")?;
        crate::transport::kernel_browser_display::timing("capture_gate_wait", at);
        let at = std::time::Instant::now();
        // There is one waiter after the semaphore. notify_one retains a permit
        // if input completes between the atomic check and awaiting the signal.
        while self.input.load(Ordering::Acquire) > 0 {
            self.changed.notified().await;
        }
        crate::transport::kernel_browser_display::timing("input_priority_wait", at);
        Ok(permit)
    }
}
pub(crate) struct InputAdmission(Arc<DisplayGate>);
impl Drop for InputAdmission {
    fn drop(&mut self) {
        self.0.input.fetch_sub(1, Ordering::AcqRel);
        self.0.changed.notify_one();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[tokio::test]
    async fn md_display_pending_input_precedes_next_capture_and_drop_releases_it() {
        let gate = Arc::new(DisplayGate::default());
        let first = gate.input();
        let second = gate.input();
        let pending = gate.capture();
        tokio::pin!(pending);
        assert!(tokio::time::timeout(Duration::from_millis(5), &mut pending)
            .await
            .is_err());
        drop(first);
        assert!(tokio::time::timeout(Duration::from_millis(5), &mut pending)
            .await
            .is_err());
        drop(second);
        let permit = tokio::time::timeout(Duration::from_secs(1), &mut pending)
            .await
            .unwrap()
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(5), gate.capture())
                .await
                .is_err()
        );
        drop(permit);
        assert!(gate.capture().await.is_ok());
    }
    #[tokio::test]
    async fn md_display_cancelling_a_waiting_credit_retains_input_and_releases_capture_slot() {
        let gate = Arc::new(DisplayGate::default());
        let input = gate.input();
        assert!(
            tokio::time::timeout(Duration::from_millis(5), gate.capture())
                .await
                .is_err()
        );
        assert_eq!(gate.capture.available_permits(), 1);
        assert_eq!(gate.input.load(Ordering::Acquire), 1);
        drop(input);
        assert!(gate.capture().await.is_ok());
    }
}
