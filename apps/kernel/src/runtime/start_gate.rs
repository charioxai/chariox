//! Whether a request a revoked client queued has begun.
//!
//! A direct browser session spawns every request it reads and keeps no queue of its own: a mutation waits in its
//! Room's command lane. When the session's authority is revoked, a mutation that has not begun must not begin; one
//! that has begun settles and keeps its cached result, so a retry over the relay sees the original answer.
//! The step from queued to begun is one-way and taken by whichever side gets there first.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use crate::error::DaemonError;

const QUEUED: u8 = 0;
const BEGUN: u8 = 1;
const CANCELLED: u8 = 2;

const CANCELLED_OPERATION: &str = "request.cancelled";

#[derive(Debug, Clone, Default)]
pub(crate) struct StartGate(Arc<AtomicU8>);

impl StartGate {
    /// The command lane takes the request. False when it was cancelled first: the request must not run.
    pub(crate) fn begin(&self) -> bool {
        match self
            .0
            .compare_exchange(QUEUED, BEGUN, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => true,
            Err(state) => state == BEGUN,
        }
    }

    /// Revocation: a request that has not begun never will. A begun one is untouched.
    pub(crate) fn cancel(&self) {
        let _ = self
            .0
            .compare_exchange(QUEUED, CANCELLED, Ordering::AcqRel, Ordering::Acquire);
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire) == CANCELLED
    }

    /// Still worth cancelling: not begun, and its request task still holds the gate.
    pub(crate) fn is_waiting(&self) -> bool {
        self.0.load(Ordering::Acquire) == QUEUED && Arc::strong_count(&self.0) > 1
    }
}

tokio::task_local! {
    static REQUEST_START: Option<StartGate>;
}

/// Runs `request` with `gate` as the request's start gate (`None`: not cancellable, as for a read).
pub(crate) async fn scope<F: std::future::Future>(gate: Option<StartGate>, request: F) -> F::Output {
    REQUEST_START.scope(gate, request).await
}

/// The gate of the request running on this task, if its transport can revoke it.
pub(crate) fn current() -> Option<StartGate> {
    REQUEST_START.try_with(Clone::clone).ok().flatten()
}

pub(crate) fn cancelled() -> DaemonError {
    DaemonError::LocalTransport {
        operation: CANCELLED_OPERATION,
        message: "the request was cancelled before it started: its client lost authority".into(),
    }
}

pub(crate) fn is_cancelled_error(error: &DaemonError) -> bool {
    matches!(error, DaemonError::LocalTransport { operation, .. } if *operation == CANCELLED_OPERATION)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_that_has_begun_survives_cancel_and_one_that_has_not_never_begins() {
        let begun = StartGate::default();
        assert!(begun.begin());
        begun.cancel();
        assert!(!begun.is_cancelled());
        assert!(begun.begin(), "begin stays true for a request already running");

        let queued = StartGate::default();
        queued.cancel();
        assert!(queued.is_cancelled());
        assert!(!queued.begin());
    }

    #[test]
    fn only_a_gate_still_held_by_its_request_is_waiting() {
        let gate = StartGate::default();
        assert!(!gate.is_waiting(), "no request holds it");
        let held = gate.clone();
        assert!(gate.is_waiting());
        drop(held);
        assert!(!gate.is_waiting());
    }

    #[tokio::test]
    async fn the_gate_travels_with_the_request_task() {
        assert!(current().is_none());
        let gate = StartGate::default();
        scope(Some(gate.clone()), async {
            assert!(current().is_some());
            scope(None, async { assert!(current().is_none()) }).await;
        })
        .await;
    }
}
