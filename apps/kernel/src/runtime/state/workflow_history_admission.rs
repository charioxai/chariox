//! MP-08 / MP-10 / MP-11: pre-proof runs can only be read, never controlled.
use super::*;
impl KernelRuntimeOwnedState {
    pub(super) fn ensure_workflow_run_has_control_provenance(
        &self,
        session: &crate::session::RuntimeSession,
        run: &crate::session::WorkflowRun,
    ) -> Result<(), DaemonError> {
        if self
            .durable_state_store
            .workflow_run_predates_ownership_tracking(session.host_daemon_id(), session.id(), run)?
        {
            return Err(DaemonError::LocalTransport {
                operation: "control historical workflow run",
                message: "Created before ownership tracking; this historical run is read-only"
                    .into(),
            });
        }
        Ok(())
    }
}
