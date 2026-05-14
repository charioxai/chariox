//! Workflow definition and run queries.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn workflow_list_workflows(
        &self,
        request: crate::local::ListWorkflowsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowsListed {
            workflows: self
                .session_store
                .read()
                .list_workflows(&request.session_id)?,
        })
    }

    pub(super) fn workflow_resolve_workflow(
        &self,
        request: crate::local::ResolveWorkflowRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowResolved {
            workflow: self
                .session_store
                .read()
                .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?,
        })
    }

    pub(super) fn workflow_list_runs(
        &self,
        request: crate::local::ListWorkflowRunsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowRunsListed {
            workflow_runs: self
                .session_store
                .read()
                .list_workflow_runs(&request.session_id, request.workflow_ref.as_deref())?,
        })
    }

    pub(super) fn workflow_get_run(
        &self,
        request: crate::local::GetWorkflowRunRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowRun {
            workflow_run: self
                .session_store
                .read()
                .resolve_workflow_run_ref(&request.session_id, &request.workflow_run_ref)?,
        })
    }
}
