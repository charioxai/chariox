//! Workflow definition and run queries.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn workflow_list_workflows(
        &self,
        request: crate::local::ListWorkflowsRequest,
        controlled_by_metaagent_id: Option<&str>,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflows = self
            .session_store
            .read()
            .list_workflows(&request.session_id)?;
        let workflows = match controlled_by_metaagent_id {
            Some(metaagent_id) => workflows
                .into_iter()
                .filter(|workflow| workflow.controlled_by_metaagent_id() == Some(metaagent_id))
                .collect(),
            None => workflows,
        };
        Ok(LocalDaemonResponse::WorkflowsListed { workflows })
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
        controlled_by_metaagent_id: Option<&str>,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow_runs = self
            .session_store
            .read()
            .list_workflow_runs(&request.session_id, request.workflow_ref.as_deref())?;
        let workflow_runs = match controlled_by_metaagent_id {
            Some(metaagent_id) => {
                let sessions = self.session_store.read();
                workflow_runs
                    .into_iter()
                    .filter(|run| {
                        sessions
                            .resolve_workflow_ref(&request.session_id, run.workflow_id())
                            .is_ok_and(|workflow| {
                                workflow.controlled_by_metaagent_id() == Some(metaagent_id)
                            })
                    })
                    .collect()
            }
            None => workflow_runs,
        };
        Ok(LocalDaemonResponse::WorkflowRunsListed { workflow_runs })
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
