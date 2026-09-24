//! Kernel-owned expiry for queued App work. This is a cancellation boundary,
//! not an authorization grant or a promise to roll back an effect already begun.

use chariox_app_runtime::worker_peer::BrokerRequest;
use std::sync::Arc;
use tokio::time::Instant;

pub(crate) struct AppOperationBudget {
    deadline: Instant,
    cancelled: Arc<dyn Fn() -> bool + Send + Sync>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum AppOperationStopped {
    #[error("app_operation_cancelled")]
    Cancelled,
    #[error("app_operation_deadline")]
    Deadline,
}

impl AppOperationBudget {
    pub(crate) fn from_broker(request: &BrokerRequest) -> Self {
        let cancellation = request.cancellation.clone();
        Self {
            deadline: request.deadline,
            cancelled: Arc::new(move || cancellation.is_cancelled()),
        }
    }

    /// Kernel-owned background work uses one fixed monotonic budget. The
    /// predicate observes retained lifecycle cancellation; it grants no authority
    /// and neither its deadline nor cancellation source comes from an App.
    pub(crate) fn from_supervisor(cancelled: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        Self {
            deadline: Instant::now() + std::time::Duration::from_secs(30),
            cancelled: Arc::new(cancelled),
        }
    }

    /// A child retains the original monotonic deadline and parent cancellation.
    /// Its additional predicate may only shorten admission, never renew a pass.
    pub(crate) fn fork(&self, cancelled: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        let parent = self.cancelled.clone();
        Self {
            deadline: self.deadline,
            cancelled: Arc::new(move || parent() || cancelled()),
        }
    }

    pub(crate) fn check(&self) -> Result<(), AppOperationStopped> {
        if (self.cancelled)() {
            return Err(AppOperationStopped::Cancelled);
        }
        if Instant::now() >= self.deadline {
            return Err(AppOperationStopped::Deadline);
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn fixture_observe_checks(
        self,
        observe: std::sync::Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        let cancelled = self.cancelled;
        Self {
            deadline: self.deadline,
            cancelled: Arc::new(move || {
                let result = cancelled();
                observe();
                result
            }),
        }
    }

    #[cfg(test)]
    pub(crate) fn fixture(
        deadline: Instant,
        cancelled: impl Fn() -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            deadline,
            cancelled: Arc::new(cancelled),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn supervisor_budget_has_fixed_deadline_and_observes_lifecycle_cancellation() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        let cancelled = Arc::new(AtomicBool::new(false));
        let observed = cancelled.clone();
        let before = Instant::now();
        let budget = AppOperationBudget::from_supervisor(move || observed.load(Ordering::Acquire));
        assert!(budget.deadline >= before + std::time::Duration::from_secs(30));
        assert!(budget.deadline <= Instant::now() + std::time::Duration::from_secs(30));
        budget.check().unwrap();
        cancelled.store(true, Ordering::Release);
        assert_eq!(budget.check(), Err(AppOperationStopped::Cancelled));
    }
    #[test]
    fn forks_retain_deadline_and_both_cancellation_sources_after_parent_drop() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let parent_cancelled = Arc::new(AtomicBool::new(false));
        let child_cancelled = Arc::new(AtomicBool::new(false));
        let observe_parent = parent_cancelled.clone();
        let parent =
            AppOperationBudget::from_supervisor(move || observe_parent.load(Ordering::Acquire));
        let observe_child = child_cancelled.clone();
        let child = parent.fork(move || observe_child.load(Ordering::Acquire));
        let sibling = parent.fork(|| false);
        assert_eq!(child.deadline, parent.deadline);
        assert_eq!(sibling.deadline, parent.deadline);
        child.check().unwrap();
        child_cancelled.store(true, Ordering::Release);
        assert_eq!(child.check(), Err(AppOperationStopped::Cancelled));
        sibling.check().unwrap();
        drop(parent);
        parent_cancelled.store(true, Ordering::Release);
        assert_eq!(sibling.check(), Err(AppOperationStopped::Cancelled));
        let expired = AppOperationBudget::fixture(Instant::now(), || false);
        assert_eq!(
            expired.fork(|| false).check(),
            Err(AppOperationStopped::Deadline)
        );
    }
}
