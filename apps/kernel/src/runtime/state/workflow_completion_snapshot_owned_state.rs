//! Workflow completion evidence lookup.
//!
//! Owns history-derived completion snapshots, pending validated-output checks, terminal
//! diagnostics, and max-turn configuration reads used by workflow completion settlement.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn provider_run_terminal_diagnostic(&self, provider_run_id: &str) -> Option<String> {
        self.provider_store
            .get_run(provider_run_id)
            .ok()
            .and_then(|run| run.terminal_diagnostic().map(str::to_string))
            .filter(|message| !message.trim().is_empty())
    }

    #[allow(dead_code)]
    pub(super) fn workflow_completion_snapshot(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
        provider_run_id: Option<&str>,
    ) -> Option<crate::session::WorkflowCompletionSnapshot> {
        let provider_run_id = provider_run_id
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        let session = self.session_store.get_session(session_id).ok()?;
        let history = match self
            .operational_history_store
            .load_session_history_entries(session_id, None)
        {
            Ok(history) => history,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.workflow",
                    "failed to load session history for workflow completion snapshot",
                    serde_json::json!({
                        "session_id": session_id,
                        "workflow_run_id": workflow_run_id,
                        "workflow_node_run_id": workflow_node_run_id,
                        "provider_run_id": provider_run_id,
                        "error": error.to_string(),
                    }),
                );
                return None;
            }
        };
        crate::scheduler::runtime::build_workflow_completion_snapshot_from_history(
            &session,
            history,
            session_id,
            workflow_run_id,
            workflow_node_run_id,
            provider_run_id,
        )
    }

    pub(super) fn workflow_prompt_has_completion_output(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
        provider_run_id: &str,
    ) -> bool {
        if self.workflow_node_run_has_valid_pending_final_output(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        ) {
            return true;
        }
        self.workflow_completion_snapshot(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
            Some(provider_run_id),
        )
        .and_then(|snapshot| snapshot.output().cloned())
        .is_some()
    }

    pub(super) fn workflow_node_run_has_valid_pending_final_output(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> bool {
        self.session_store
            .get_session(session_id)
            .ok()
            .and_then(|session| {
                session
                    .workflow_run(workflow_run_id)
                    .and_then(|workflow_run| {
                        workflow_run
                            .node_runs()
                            .iter()
                            .find(|node_run| node_run.id() == workflow_node_run_id)
                    })
                    .map(|node_run| node_run.has_valid_pending_final_output())
            })
            .unwrap_or(false)
    }

    #[allow(dead_code)]
    pub(super) fn workflow_max_turns(&self, session_id: &str) -> Option<usize> {
        self.session_store
            .get_session(session_id)
            .ok()
            .and_then(|session| {
                session
                    .config_state()
                    .values()
                    .get("workflow.max_turns")
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .filter(|value| *value > 0)
            })
            .or(Some(
                crate::session::DEFAULT_WORKFLOW_RUN_MAX_TURNS_SAFETY_LIMIT,
            ))
    }
}
