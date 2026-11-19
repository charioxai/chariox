//! A workflow's responsible and the kernel-kept trigger that carries its runs' needs to an
//! agent, written to the session store together.
use super::*;
use crate::session::{ResponsibleNotice, WorkflowResponsible};

impl SessionService {
    /// Sets who answers for the workflow; a fallback passes its notice, a choice passes none.
    pub(crate) fn set_workflow_responsible(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        responsible: WorkflowResponsible,
        notice: Option<ResponsibleNotice>,
    ) -> Result<WorkflowDefinition, DaemonError> {
        let workflow_id = self
            .resolve_workflow_ref(session_id, workflow_ref)?
            .id()
            .to_string();
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let workflow =
            session
                .workflow_mut(&workflow_id)
                .ok_or_else(|| DaemonError::WorkflowNotFound {
                    session_id: session_id.to_string(),
                    workflow_id: workflow_id.clone(),
                })?;
        workflow.set_responsible(responsible, notice);
        Ok(workflow.clone())
    }

    /// The person opened the run that needed them.
    pub(crate) fn clear_workflow_run_attention(
        &mut self,
        session_id: &str,
        run_ref: &str,
    ) -> Result<WorkflowRun, DaemonError> {
        let run_id = self
            .resolve_workflow_run_ref(session_id, run_ref)?
            .id()
            .to_string();
        let run = self
            .store
            .get_mut(session_id)
            .and_then(|session| session.workflow_run_mut(&run_id))
            .ok_or_else(|| DaemonError::WorkflowRunNotFound {
                session_id: session_id.to_string(),
                workflow_run_id: run_ref.to_string(),
            })?;
        run.clear_attention();
        Ok(run.clone())
    }
}
