//! MP-08 / MP-10 / MP-11: historical reads and the kernel-owner Stop exception.
use super::*;
impl KernelRuntimeOwnedState {
    pub(super) fn ensure_workflow_run_interrupt_allowed(
        &self,
        session: &crate::session::RuntimeSession,
        run: &crate::session::WorkflowRun,
        pause: bool,
        stop_user: Option<&str>,
    ) -> Result<(), DaemonError> {
        // MP-08/MP-10/MP-11: coordinator 2026-10-13 17:12. Only the
        // authenticated kernel owner may stop a still-active historical run.
        if !pause
            && !run.status().is_terminal()
            && stop_user.is_some_and(|user| {
                self.durable_state_store
                    .workflow_history_is_kernel_owner(session.host_daemon_id(), user)
            })
        {
            return Ok(());
        }
        self.ensure_workflow_run_has_control_provenance(session, run)
    }

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
