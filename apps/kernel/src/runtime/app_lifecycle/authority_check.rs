//! One retained FIFO admission request for a native owner's periodic check.
use super::*;
use futures_util::FutureExt;
use std::{future::Future, pin::Pin};
use tokio::sync::AcquireError;

type Admission =
    Pin<Box<dyn Future<Output = std::result::Result<OwnedSemaphorePermit, AcquireError>> + Send>>;

pub(super) struct AuthorityCheck {
    budget: AppOperationBudget,
    admission: Admission,
}
impl AuthorityCheck {
    pub(super) fn new(budget: AppOperationBudget, admission: Arc<Semaphore>) -> Self {
        Self {
            budget,
            admission: Box::pin(admission.acquire_owned()),
        }
    }

    /// The owner's existing 100 ms loop polls this same future. Recreating a
    /// try-acquire or dropping a pending acquire loses its FIFO place to SDK
    /// traffic. No async task or renewed budget is needed for a native owner.
    pub(super) fn verify(
        &mut self,
        store: &DurableKernelStateStore,
        active: &ActiveStartAdmission,
    ) -> Result<bool> {
        self.budget.check().map_err(|_| LifecycleError::Authority)?;
        let Some(permit) = self.admission.as_mut().now_or_never() else {
            return Ok(false);
        };
        let _permit = permit.map_err(|_| LifecycleError::Authority)?;
        store.verify_app_start(active, self.budget.fork(|| false))?;
        Ok(true)
    }
}
