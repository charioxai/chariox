//! Workflow run resume transitions.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn workflow_resume_run(
        &self,
        session_id: &str,
        workflow_run_ref: &str,
    ) -> Result<(crate::session::WorkflowRun, WorkflowPromptDispatches), DaemonError> {
        // Classify an unsubmitted original entry while the stopped session is
        // still read-locked, before publishing it as runnable to the owned pump.
        // A concurrent pump admission afterward must suppress this retry, not
        // turn it into an intentional second user invocation.
        let (resumable_node_ids, pending_entry_node) = {
            let sessions = self.session_store.read();
            let original = sessions.resolve_workflow_run_ref(session_id, workflow_run_ref)?;
            let pending = self
                .workflow_entry_intent(session_id, original.id())?
                .filter(|intent| !intent.submitted)
                .map(|intent| intent.node_id);
            let nodes = original
                .node_runs()
                .iter()
                .filter(|node_run| {
                    node_run.status() == crate::session::WorkflowNodeRunStatus::Stopped
                        && node_run.completion().is_none()
                        && node_run
                            .turn_envelope()
                            .and_then(|envelope| envelope.rendered_prompt())
                            .is_some()
                })
                .map(|node_run| node_run.id().to_string())
                .collect::<std::collections::BTreeSet<_>>();
            (nodes, pending)
        };
        let workflow_run = self
            .session_store
            .write()
            .resume_workflow_run(session_id, workflow_run_ref)?;
        self.persist_workflow_runtime_session(session_id, "workflow_resumed")?;
        let resumable = workflow_run
            .node_runs()
            .iter()
            .filter(|node_run| resumable_node_ids.contains(node_run.id()))
            .filter_map(|node_run| {
                let prompt = node_run.turn_envelope()?.rendered_prompt()?.to_string();
                Some((
                    node_run.id().to_string(),
                    node_run.agent_id().to_string(),
                    prompt,
                ))
            })
            .collect::<Vec<_>>();
        let mut dispatches = WorkflowPromptDispatches::default();
        for (workflow_node_run_id, agent_id, prompt_text) in resumable {
            if pending_entry_node.as_deref() == Some(workflow_node_run_id.as_str()) {
                if let Some(prepared) = self.workflow_retry_durable_entry(
                    session_id,
                    workflow_run.id(),
                    &workflow_node_run_id,
                )? {
                    dispatches.extend(prepared);
                    continue;
                }
            }
            // A deliberate user resume of previously submitted work is a new
            // invocation; only an unsubmitted original entry reuses its intent.
            let prompt = crate::session::PromptQueueItem::new(
                format!(
                    "pending-draft:workflow-resume:{}:{}",
                    workflow_run.id(),
                    workflow_node_run_id
                ),
                crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
                agent_id,
                prompt_text,
                crate::session::PromptStatus::Queued,
            )
            .with_workflow_context(workflow_run.id(), &workflow_node_run_id);
            match self.workflow_submit_prepared_prompt(
                crate::app::KernelPreparedPromptSubmission {
                    session_id: session_id.to_string(),
                    prompt,
                    force_queue: false,
                    refresh_projection: true,
                },
                workflow_run.id(),
                &workflow_node_run_id,
            ) {
                Ok(prepared) => dispatches.extend(prepared),
                Err(error) => {
                    self.record_notice(
                        session_id,
                        None,
                        self.attachment_store
                            .list_session_attachment_ids(session_id),
                        format!(
                            "Workflow run `{}` could not resume node prompt: {}",
                            workflow_run.id(),
                            error
                        ),
                    );
                }
            }
        }
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run.id())?;
        Ok((workflow_run, dispatches))
    }
}
