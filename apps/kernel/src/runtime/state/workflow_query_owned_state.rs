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
        let session = self.session_store.read().get_session(&request.session_id)?;
        let workflow_id = request
            .workflow_ref
            .as_deref()
            .map(|workflow_ref| {
                self.session_store
                    .read()
                    .resolve_workflow_ref(&request.session_id, workflow_ref)
                    .map(|workflow| workflow.id().to_string())
            })
            .transpose()?;
        let mut workflow_runs = session
            .workflow_runs()
            .iter()
            .filter(|run| {
                workflow_id
                    .as_deref()
                    .is_none_or(|workflow_id| run.workflow_id() == workflow_id)
            })
            .cloned()
            .collect::<Vec<_>>();
        let mut cursor: Option<(u64, String)> = None;
        loop {
            let page = self.durable_state_store.list_workflow_runs_page(
                session.host_daemon_id(),
                session.id(),
                workflow_id.as_deref(),
                cursor
                    .as_ref()
                    .map(|(created_at_ms, run_id)| (*created_at_ms, run_id.as_str())),
                500,
            )?;
            for stored_run in page.workflow_runs {
                if let Some(index) = workflow_runs
                    .iter()
                    .position(|current| current.id() == stored_run.id())
                {
                    workflow_runs[index] = stored_run;
                } else {
                    workflow_runs.push(stored_run);
                }
            }
            let Some(next_cursor) = page.next_cursor else {
                break;
            };
            cursor = Some(next_cursor);
        }
        workflow_runs.sort_by_key(|run| (run.created_at_ms(), run.id().to_string()));
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
        let session = self.session_store.read().get_session(&request.session_id)?;
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(&request.session_id, &request.workflow_run_ref)
            .ok()
            .or(self.durable_state_store.resolve_workflow_run(
                session.host_daemon_id(),
                session.id(),
                &request.workflow_run_ref,
            )?)
            .ok_or_else(|| DaemonError::WorkflowRunNotFound {
                session_id: request.session_id.clone(),
                workflow_run_id: request.workflow_run_ref.clone(),
            })?;
        Ok(LocalDaemonResponse::WorkflowRun { workflow_run })
    }
}
