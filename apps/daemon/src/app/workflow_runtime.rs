use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::session::{WorkflowDefinition, WorkflowEndpointDefinition, WorkflowRun};

impl DaemonApp {
    pub fn invoke_workflow_endpoint_and_schedule(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        prompt: Option<String>,
    ) -> Result<(WorkflowRun, WorkflowDefinition, WorkflowEndpointDefinition), DaemonError> {
        let workflow = self
            .sessions()
            .resolve_workflow_ref(session_id, workflow_ref)?;
        let endpoint = self.sessions().resolve_workflow_endpoint_ref(
            session_id,
            workflow_ref,
            endpoint_ref,
        )?;
        crate::scheduler::runtime::validate_workflow_agents(self, session_id, &workflow)?;
        let workflow_run = self.sessions_mut().invoke_workflow_endpoint(
            session_id,
            workflow_ref,
            endpoint_ref,
            prompt,
        )?;
        crate::scheduler::runtime::schedule_workflow_run_entry_node(
            self,
            session_id,
            &workflow_run,
        )?;
        let workflow_run = self
            .sessions()
            .resolve_workflow_run_ref(session_id, workflow_run.id())?;
        Ok((workflow_run, workflow, endpoint))
    }
}
