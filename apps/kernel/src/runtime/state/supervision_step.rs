//! What a supervision loop waits on right now, so a report of its stall names the step and the
//! agent or task it is held by instead of only the time it was last seen.
use std::sync::Mutex;

#[derive(Default)]
pub(super) struct SupervisionStep(Mutex<Option<(String, u64)>>);

impl SupervisionStep {
    /// The loop now waits on `step`.
    pub(super) fn enter(&self, step: impl Into<String>) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) =
            Some((step.into(), crate::session::unix_epoch_ms()));
    }

    /// The loop finished its pass and waits on nothing.
    pub(super) fn leave(&self) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// The step the loop waits on and since when, or None between passes.
    pub(super) fn current(&self) -> Option<(String, u64)> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}
