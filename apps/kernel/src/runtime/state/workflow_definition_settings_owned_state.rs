//! Workflow definition runtime settings mutations.
//!
//! This module owns definition-level settings that do not edit endpoint, node, or edge topology.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn workflow_set_flush_context(
        &self,
        request: crate::local::SetWorkflowFlushContextRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        let workflow = self
            .session_store
            .write()
            .set_workflow_flush_agent_context_before_run(
                &request.session_id,
                &request.workflow_ref,
                request.flush_agent_context_before_run,
            )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowFlushContextUpdated { workflow, session })
    }

    pub(super) fn workflow_set_run_output_schema(
        &self,
        request: crate::local::SetWorkflowRunOutputSchemaRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.ensure_workflow_revision(
            &request.session_id,
            &request.workflow_ref,
            request.expected_workflow_revision,
        )?;
        let workflow = self
            .session_store
            .write()
            .set_workflow_run_output_schema_ref(
                &request.session_id,
                &request.workflow_ref,
                request.run_output_schema_ref,
            )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowRunOutputSchemaUpdated { workflow, session })
    }

}
