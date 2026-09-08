//! Kernel-owned expiry for queued App work. This is a cancellation boundary,
//! not an authorization grant or a promise to roll back an effect already begun.

use chariox_app_runtime::worker_peer::BrokerRequest;
use tokio::time::Instant;

pub(crate) struct AppOperationBudget {
    deadline: Instant,
    cancelled: Box<dyn Fn() -> bool + Send + Sync>,
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
            cancelled: Box::new(move || cancellation.is_cancelled()),
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
            cancelled: Box::new(move || {
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
            cancelled: Box::new(cancelled),
        }
    }
}
