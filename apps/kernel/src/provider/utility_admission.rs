//! MP-08 / MP-10 / MP-11: kernel-owned input protection at utility submission.
use crate::error::DaemonError;
use std::sync::Arc;

type AdmissionCheck = dyn Fn() -> Result<Box<dyn Send>, DaemonError> + Send + Sync;

#[derive(Clone)]
pub(crate) struct ProviderUtilityAdmission(Arc<AdmissionCheck>);

impl ProviderUtilityAdmission {
    pub(crate) fn new(
        check: impl Fn() -> Result<Box<dyn Send>, DaemonError> + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(check))
    }
    pub(crate) fn acquire(&self) -> Result<Box<dyn Send>, DaemonError> {
        (self.0)()
    }
}
