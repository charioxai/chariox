//! A separate local precommit health round trip. Normal startup remains an
//! active-generation notification with the App's approved SDK capabilities.
use super::*;

/// Retains the actual process/peer while its single-use health report is on the
/// writer. Neither a terminal, App request nor package field can construct it.
pub(crate) struct HealthyAppWorker {
    registered: RegisteredAppWorker,
    report: Option<FirstInstallHealth>,
}
pub(crate) struct FirstInstallHealth {
    catalog: Arc<EventCatalog>,
    budget: AppOperationBudget,
}
impl FirstInstallHealth {
    /// SQL transaction tests only. Actual native health/ownership is exercised
    /// separately by the retained lifecycle fixture; this never ships.
    #[cfg(test)]
    pub(crate) fn fixture(catalog: Arc<EventCatalog>, budget: AppOperationBudget) -> Self {
        Self { catalog, budget }
    }
    pub(crate) fn catalog(&self) -> &Arc<EventCatalog> {
        &self.catalog
    }
    pub(crate) fn budget(&self) -> &AppOperationBudget {
        &self.budget
    }
}
impl RegisteredAppWorker {
    /// SDK 0.5 defines health_check as local initialization/validation only.
    /// The worker is still Starting: state, network, files and every other
    /// broker effect remain denied. Private Node I/O is bounded by containment.
    /// An absent handler is a successful trusted-SDK round trip; declaring a
    /// handler allows the App to fail this check before activation commits.
    pub(crate) fn check_local_health_blocking(
        mut self,
    ) -> Result<HealthyAppWorker, AppWorkerError> {
        let budget = self.take_activation_budget()?;
        budget.check().map_err(|_| AppWorkerError::Deadline)?;
        self.owner
            .lifecycle_blocking("health_check", Duration::from_secs(3))?;
        budget.check().map_err(|_| AppWorkerError::Deadline)?;
        if self.owner.peer.is_closed() {
            return Err(AppWorkerError::Unavailable);
        }
        let report = FirstInstallHealth {
            catalog: self.owner.catalog.clone(),
            budget,
        };
        Ok(HealthyAppWorker {
            registered: self,
            report: Some(report),
        })
    }
}
impl HealthyAppWorker {
    pub(crate) fn take_health(&mut self) -> Result<FirstInstallHealth, AppWorkerError> {
        self.report.take().ok_or(AppWorkerError::Unavailable)
    }
    pub(crate) fn activate_blocking(
        self,
        proof: CommittedAppActivation,
    ) -> Result<(AppWorkerOwner, ActivatedApp), AppWorkerError> {
        if self.report.is_some() {
            return Err(AppWorkerError::Identity);
        }
        self.registered.activate_blocking(proof)
    }
}
