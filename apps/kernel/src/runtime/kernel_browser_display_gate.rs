//! MD-DISPLAY-04: bounded capture admission briefly yields to source input.
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
        // Observational host work now waits outside the controller mutex. Input
        // and capture may coexist; continuous input must not starve the display.
        // This brief yield is scheduling only, never an authority/policy fence.
        if self.input.load(Ordering::Acquire) > 0 {
            let _ =
                tokio::time::timeout(std::time::Duration::from_millis(1), self.changed.notified())
                    .await;
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
    async fn md_display_continuous_input_cannot_starve_independent_capture() {
        let gate = Arc::new(DisplayGate::default());
        let first = gate.input();
        let second = gate.input();
        let admitted = tokio::time::timeout(Duration::from_millis(30), gate.capture()).await;
        drop(first);
        drop(second);
        let permit = admitted
            .expect("continuous input starved independently scheduled display")
            .unwrap();
        drop(permit);
    }
    #[tokio::test]
    async fn md_display_input_is_independent_of_the_serial_capture_slot() {
        let gate = Arc::new(DisplayGate::default());
        let first = gate.capture().await.unwrap();
        let input = gate.input();
        assert_eq!(gate.input.load(Ordering::Acquire), 1);
        assert!(
            tokio::time::timeout(Duration::from_millis(5), gate.capture())
                .await
                .is_err()
        );
        drop(input);
        drop(first);
        assert!(gate.capture().await.is_ok());
    }
    #[tokio::test]
    async fn md_display_cancelling_a_waiting_credit_retains_input_and_capture_ownership() {
        let gate = Arc::new(DisplayGate::default());
        let first = gate.capture().await.unwrap();
        let input = gate.input();
        assert!(
            tokio::time::timeout(Duration::from_millis(5), gate.capture())
                .await
                .is_err()
        );
        assert_eq!(gate.capture.available_permits(), 0);
        assert_eq!(gate.input.load(Ordering::Acquire), 1);
        drop(first);
        drop(input);
        assert!(gate.capture().await.is_ok());
    }
}
