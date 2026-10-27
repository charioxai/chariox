//! Home-side settlement after an authoritative worker failure completion.

use super::*;

impl KernelRuntimeState {
    pub(super) fn finish_remote_provider_failure(
        &self,
        session_id: &str,
        completions: &[crate::session::PromptCompletion],
    ) -> Result<(), DaemonError> {
        // Projection has settled the failed turn. Release its workspace and
        // preserve queued invocations; never replay the failed invocation.
        for completion in completions {
            if let (Some(run_id), Some(node_id)) = (
                completion.completed.workflow_run_id(),
                completion.completed.workflow_node_run_id(),
            ) {
                self.owned
                    .release_workflow_node_workspace_claim(session_id, run_id, node_id);
            }
        }
        self.owned
            .persist_workflow_runtime_session(session_id, "remote_workflow_provider_prompt_failed")
            .map(|_| ())
    }
}
