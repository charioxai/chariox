//! Blocking calls retain their admission on the owning kernel task.
use super::*;

impl DurableKernelStateStore {
    /// Replay acknowledgment goes through the writer barrier, not a potentially
    /// visible query snapshot of a concurrent uncertain COMMIT.
    pub(crate) fn replay_first_app_install(
        &self,
        owner: &str,
        request_id: &str,
        digest: &str,
        budget: AppOperationBudget,
    ) -> Result<InstallOperation> {
        match self.first_install(Command::Replay {
            owner: owner.into(),
            request_id: request_id.into(),
            digest: digest.into(),
            budget,
        })? {
            Reply::Operation(value) => Ok(value),
            _ => Err(InstallOperationError::Storage),
        }
    }
    fn first_install(&self, command: Command) -> Result<Reply> {
        let (response, receiver) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::AppInstallationOperation(Box::new(
                AppInstallationOperationRequest { command, response },
            )))
            .map_err(|_| InstallOperationError::Storage)?;
        receiver
            .recv()
            .map_err(|_| InstallOperationError::CommitUnknown)?
    }
    pub(crate) fn begin_first_app_install(
        &self,
        owner: &str,
        request_id: &str,
        candidate: VerifiedInstallCandidate,
        budget: AppOperationBudget,
    ) -> Result<InstallOperation> {
        match self.first_install(Command::Begin {
            owner: owner.into(),
            request_id: request_id.into(),
            candidate,
            budget,
        })? {
            Reply::Operation(value) => Ok(value),
            _ => Err(InstallOperationError::Storage),
        }
    }
    pub(crate) fn claim_first_app_install(
        &self,
        owner: &str,
        request_id: &str,
        attempt: &str,
        budget: AppOperationBudget,
    ) -> Result<ApprovedFirstInstall> {
        match self.first_install(Command::Claim {
            owner: owner.into(),
            request_id: request_id.into(),
            attempt: attempt.into(),
            budget,
        })? {
            Reply::Approved(value) => Ok(value),
            _ => Err(InstallOperationError::Storage),
        }
    }
    pub(crate) fn verify_first_app_install(
        &self,
        admission: Arc<ApprovedFirstInstall>,
        budget: AppOperationBudget,
    ) -> Result<()> {
        self.first_install(Command::Verify { admission, budget })?;
        Ok(())
    }
    pub(crate) fn commit_first_app_install(
        &self,
        admission: Arc<ApprovedFirstInstall>,
        health: FirstInstallHealth,
        budget: AppOperationBudget,
    ) -> Result<FirstInstallCommitted> {
        match self.first_install(Command::Commit {
            admission,
            health,
            budget,
            #[cfg(test)]
            fault: None,
        })? {
            Reply::Committed(value) => Ok(value),
            _ => Err(InstallOperationError::Storage),
        }
    }
    #[cfg(test)]
    pub(crate) fn fixture_commit_first_app_install(
        &self,
        admission: Arc<ApprovedFirstInstall>,
        health: FirstInstallHealth,
        budget: AppOperationBudget,
        fault: CommitTestFault,
    ) -> Result<FirstInstallCommitted> {
        match self.first_install(Command::Commit {
            admission,
            health,
            budget,
            fault: Some(fault),
        })? {
            Reply::Committed(value) => Ok(value),
            _ => Err(InstallOperationError::Storage),
        }
    }
    /// Retained lifecycle owner calls after native reap/broker drain. Cancellation
    /// cannot turn a committed receipt into an aborted generation or delete data.
    pub(crate) fn finish_first_app_install(
        &self,
        admission: Arc<ApprovedFirstInstall>,
        cancelled: bool,
        failure: &str,
        budget: AppOperationBudget,
    ) -> Result<()> {
        self.first_install(Command::Finish {
            admission,
            cancelled,
            failure: failure.into(),
            budget,
        })?;
        Ok(())
    }
    pub(crate) fn first_app_install_status(
        &self,
        owner: &str,
        request_id: &str,
    ) -> Result<InstallOperation> {
        let connection = self
            .lock_connection("durable_state.first_app_install_status")
            .map_err(|_| InstallOperationError::Storage)?;
        store::load(&connection, owner, request_id)?.ok_or(InstallOperationError::NotFound)
    }
    /// Metadata withdrawal is immediate; physical cleanup still waits for the
    /// owning worker/domain lease to be reaped by the lifecycle service.
    pub(crate) fn cancel_first_app_install(
        &self,
        owner: &str,
        request_id: &str,
        budget: AppOperationBudget,
    ) -> Result<InstallOperation> {
        match self.first_install(Command::Cancel {
            owner: owner.into(),
            request_id: request_id.into(),
            budget,
        })? {
            Reply::Operation(value) => Ok(value),
            _ => Err(InstallOperationError::Storage),
        }
    }
    pub(crate) fn first_app_install_recovery_candidates(
        &self,
        after: Option<(&str, &str)>,
    ) -> Result<Vec<(String, String)>> {
        let connection = self
            .lock_connection("durable_state.first_app_install_recovery")
            .map_err(|_| InstallOperationError::Storage)?;
        store::candidates(&connection, after)
    }
}
