use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::session::{unix_epoch_ms, WorkflowDefinition, WorkflowEndpointDefinition, WorkflowRun};

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

    pub fn pump_workflow_watchdogs(&mut self) {
        let plans = match self
            .sessions_mut()
            .collect_due_workflow_watchdog_invocations(unix_epoch_ms())
        {
            Ok(plans) => plans,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.app",
                    "workflow watchdog collection failed",
                    serde_json::json!({ "error": error.to_string() }),
                );
                return;
            }
        };
        for plan in plans {
            match self.invoke_workflow_endpoint_and_schedule(
                &plan.session_id,
                &plan.workflow_id,
                &plan.endpoint_id,
                Some(plan.invocation_prompt.clone()),
            ) {
                Ok((workflow_run, _, _)) => {
                    let _ = self.sessions_mut().mark_workflow_watchdog_invoked(
                        &plan.session_id,
                        &plan.watchdog_id,
                        workflow_run.id(),
                    );
                }
                Err(error) => {
                    let _ = self.sessions_mut().mark_workflow_watchdog_failed(
                        &plan.session_id,
                        &plan.watchdog_id,
                        error.to_string(),
                    );
                    crate::logging::warn_with_fields(
                        "daemon.app",
                        "workflow watchdog invoke failed",
                        serde_json::json!({
                            "session_id": plan.session_id,
                            "workflow_id": plan.workflow_id,
                            "endpoint_id": plan.endpoint_id,
                            "watchdog_id": plan.watchdog_id,
                            "error": error.to_string(),
                        }),
                    );
                }
            }
        }
    }
}
