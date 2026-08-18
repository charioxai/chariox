//! Workflow definition and run queries.

use super::*;

const DEFAULT_WORKFLOW_RUN_PAGE_SIZE: usize = 50;
const MAX_WORKFLOW_RUN_PAGE_SIZE: usize = 200;

fn encode_workflow_run_cursor(created_at_ms: u64, run_id: &str) -> String {
    format!("v1:{created_at_ms}:{run_id}")
}

fn decode_workflow_run_cursor(cursor: Option<&str>) -> Result<Option<(u64, String)>, DaemonError> {
    let Some(cursor) = cursor else {
        return Ok(None);
    };
    let mut parts = cursor.splitn(3, ':');
    let valid_version = parts.next() == Some("v1");
    let created_at_ms = parts.next().and_then(|value| value.parse::<u64>().ok());
    let run_id = parts.next().filter(|value| !value.is_empty());
    match (valid_version, created_at_ms, run_id) {
        (true, Some(created_at_ms), Some(run_id)) => Ok(Some((created_at_ms, run_id.to_string()))),
        _ => Err(DaemonError::LocalTransport {
            operation: "list_workflow_runs",
            message: "workflow run cursor is invalid or unsupported".to_string(),
        }),
    }
}

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
        let cursor = decode_workflow_run_cursor(request.cursor.as_deref())?;
        let limit = request
            .limit
            .map(|limit| limit as usize)
            .unwrap_or(DEFAULT_WORKFLOW_RUN_PAGE_SIZE)
            .clamp(1, MAX_WORKFLOW_RUN_PAGE_SIZE);
        let before_cursor = cursor
            .as_ref()
            .map(|(created_at_ms, run_id)| (*created_at_ms, run_id.as_str()));
        let mut hot_workflow_runs = session
            .workflow_runs()
            .iter()
            .filter(|run| {
                workflow_id
                    .as_deref()
                    .is_none_or(|workflow_id| run.workflow_id() == workflow_id)
                    && before_cursor
                        .is_none_or(|before| (run.created_at_ms(), run.id()) < (before.0, before.1))
            })
            .cloned()
            .collect::<Vec<_>>();
        hot_workflow_runs.sort_by(|left, right| {
            (right.created_at_ms(), right.id()).cmp(&(left.created_at_ms(), left.id()))
        });
        let hot_has_more = hot_workflow_runs.len() > limit;
        hot_workflow_runs.truncate(limit);
        let legacy_page = self.legacy_workflow_history.list_page(
            session.id(),
            workflow_id.as_deref(),
            before_cursor,
            limit,
        );
        let durable_page = self.durable_state_store.list_workflow_runs_page(
            session.host_daemon_id(),
            session.id(),
            workflow_id.as_deref(),
            before_cursor,
            limit,
        )?;
        let source_has_more =
            hot_has_more || durable_page.next_cursor.is_some() || legacy_page.has_more;
        let mut workflow_runs_by_id = std::collections::BTreeMap::new();
        for run in legacy_page.workflow_runs {
            workflow_runs_by_id.insert(run.id().to_string(), run);
        }
        for run in durable_page.workflow_runs {
            workflow_runs_by_id.insert(run.id().to_string(), run);
        }
        for run in hot_workflow_runs {
            workflow_runs_by_id.insert(run.id().to_string(), run);
        }
        let mut scanned_runs = workflow_runs_by_id.into_values().collect::<Vec<_>>();
        scanned_runs.sort_by(|left, right| {
            (right.created_at_ms(), right.id()).cmp(&(left.created_at_ms(), left.id()))
        });
        let has_more = source_has_more || scanned_runs.len() > limit;
        scanned_runs.truncate(limit);
        let next_cursor = if has_more {
            scanned_runs
                .last()
                .map(|last| encode_workflow_run_cursor(last.created_at_ms(), last.id()))
        } else {
            None
        };
        let workflow_runs = match controlled_by_metaagent_id {
            Some(metaagent_id) => {
                let sessions = self.session_store.read();
                scanned_runs
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
            None => scanned_runs,
        };
        Ok(LocalDaemonResponse::WorkflowRunsListed {
            workflow_runs,
            next_cursor,
        })
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
            .or_else(|| {
                self.legacy_workflow_history
                    .resolve(session.id(), &request.workflow_run_ref)
            })
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
