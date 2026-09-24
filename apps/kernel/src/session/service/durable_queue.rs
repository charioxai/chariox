//! A tentative application of the existing queue/run rules. Only the target
//! session is cloned; IDs may advance but no tentative session is published.
use super::*;
pub(crate) type WorkflowQueueRun = (
    WorkflowQueuedPrompt,
    WorkflowRun,
    WorkflowDefinition,
    WorkflowEndpointDefinition,
);
pub(crate) struct PreparedWorkflowQueueRun {
    before: RuntimeSession,
    after: RuntimeSession,
    next: Option<WorkflowQueueRun>,
}
impl PreparedWorkflowQueueRun {
    pub(crate) fn before(&self) -> &RuntimeSession {
        &self.before
    }
    pub(crate) fn after(&self) -> &RuntimeSession {
        &self.after
    }
    pub(crate) fn next(&self) -> Option<&WorkflowQueueRun> {
        self.next.as_ref()
    }
}
impl SessionService {
    /// Caller holds the existing session write guard through durable commit and
    /// publication. The same queue selector/instance reservation/run constructor
    /// is reused for every source; there is no App-specific scheduler.
    pub(crate) fn prepare_durable_workflow_queue_run(
        &mut self,
        session_id: &str,
    ) -> Result<PreparedWorkflowQueueRun, DaemonError> {
        let before = self.get_session(session_id)?;
        let result = self.dequeue_next_workflow_prompt_and_create_run(session_id);
        let after = self.get_session(session_id);
        self.restore_session(before.clone());
        Ok(PreparedWorkflowQueueRun {
            before,
            after: after?,
            next: result?,
        })
    }
}
