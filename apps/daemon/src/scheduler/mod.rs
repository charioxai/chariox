use crate::app::DaemonApp;
use crate::error::DaemonError;

pub mod runtime;
pub struct SchedulerService;

impl SchedulerService {
    pub fn schedule_workflow_node_prompt(
        app: &mut DaemonApp,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
        target_agent_id: &str,
        node_id: &str,
        prompt: &str,
    ) -> Result<(), DaemonError> {
        runtime::schedule_workflow_node_prompt(
            app,
            session_id,
            workflow_run_id,
            workflow_node_run_id,
            target_agent_id,
            node_id,
            prompt,
        )
    }

}
