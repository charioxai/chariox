//! First installs reuse the exact process/peer owner, limits and publisher used
//! by restart. This module cannot enroll keys or manufacture capability consent.
use super::*;

impl AppLifecycleService {
    /// Internal post-approval entry. The writer still resolves the existing
    /// approved staged decision; this call's arguments confer no authority.
    pub(crate) fn start_first_blocking(
        &self,
        owner: &str,
        request_id: &str,
        runtime: Handle,
    ) -> Result<StartDisposition> {
        let operation = self.0.store.first_app_install_status(owner, request_id)?;
        match operation.phase {
            InstallPhase::Committed => {
                self.start(owner, &operation.token.installation_id, true, runtime)
            }
            InstallPhase::AwaitingApproval | InstallPhase::Starting => self.start_kind(
                owner,
                &operation.token.installation_id,
                StartKind::First {
                    request_id: request_id.into(),
                },
                runtime,
            ),
            InstallPhase::Cancelled | InstallPhase::Failed => Err(LifecycleError::Authority),
        }
    }
}

pub(super) fn activate(
    context: &owner::Context,
    admission: Arc<ApprovedFirstInstall>,
    preparation: OwnedSemaphorePermit,
    operation: OwnedSemaphorePermit,
) -> Result<(start::Started, ActiveStartAdmission)> {
    let _preparation = preparation;
    let _operation = operation;
    let budget = context.control.budget();
    let registered = start::register(
        context,
        admission.binding(),
        admission.trust(),
        &budget,
        || {
            context
                .store
                .verify_first_app_install(admission.clone(), budget.fork(|| false))
                .map_err(Into::into)
        },
    )?;
    let mut healthy = registered
        .registered
        .check_local_health_blocking()
        .map_err(|_| LifecycleError::Health)?;
    budget.check().map_err(|_| LifecycleError::Stopped)?;
    let report = healthy.take_health().map_err(|_| LifecycleError::Health)?;
    let committed =
        context
            .store
            .commit_first_app_install(admission, report, budget.fork(|| false))?;
    let (owner, handle) = healthy
        .activate_blocking(committed.activation)
        .map_err(|_| LifecycleError::Registration)?;
    Ok((
        start::Started {
            owner,
            handle,
            events: registered.events,
            #[cfg(test)]
            fixture_release: registered.fixture_release,
        },
        committed.active,
    ))
}
