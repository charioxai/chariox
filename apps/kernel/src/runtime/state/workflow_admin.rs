//! Workflow administrative mutations.
//!
//! Owns workflow CRUD, endpoint edits, watchdog updates, and queue-facing commands that alter
//! workflow definitions rather than executing an individual node.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn workflow_session(
        &self,
        session_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        self.session_snapshot(session_id)
    }

    pub(super) fn workflow_create_workflow(
        &self,
        request: crate::local::CreateWorkflowRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow = self
            .session_store
            .write()
            .create_workflow(&request.session_id, request.alias)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowCreated { workflow, session })
    }

    pub(super) fn workflow_apply_design_op(
        &self,
        request: crate::local::ApplyWorkflowDesignOpRequest,
        caller_user_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        self.session_store.write().apply_workflow_design_op(
            &request.session_id,
            request.op,
            caller_user_id.to_string(),
        )?;
        self.workflow_session(&request.session_id)
    }

    pub(super) fn workflow_alias_workflow(
        &self,
        request: crate::local::AliasWorkflowRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        let workflow = self.session_store.write().assign_workflow_alias(
            &request.session_id,
            &request.workflow_ref,
            request.alias,
        )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowAliased { workflow, session })
    }
}
