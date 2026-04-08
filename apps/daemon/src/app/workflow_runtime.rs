use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::session::{
    unix_epoch_ms, QueuedWorkflowLaunch, QueuedWorkflowLaunchSource, WorkflowDefinition,
    WorkflowEndpointDefinition, WorkflowLaunchAdmission, WorkflowRun, WorkflowWatchdogTickPlan,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowLaunchOutcome {
    Started {
        workflow_run: WorkflowRun,
        workflow: WorkflowDefinition,
        endpoint: WorkflowEndpointDefinition,
    },
    Queued {
        queued_launch: QueuedWorkflowLaunch,
        workflow: WorkflowDefinition,
        endpoint: WorkflowEndpointDefinition,
    },
}

impl DaemonApp {
    pub fn invoke_workflow_endpoint_with_admission(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        prompt: Option<String>,
    ) -> Result<WorkflowLaunchOutcome, DaemonError> {
        let workflow = self.sessions().resolve_workflow_ref(session_id, workflow_ref)?;
        let endpoint = self
            .sessions()
            .resolve_workflow_endpoint_ref(session_id, workflow_ref, endpoint_ref)?;
        crate::scheduler::runtime::validate_workflow_agents(self, session_id, &workflow)?;
        match self.sessions_mut().admit_manual_workflow_launch(
            session_id,
            workflow.id(),
            endpoint.id(),
            prompt.clone(),
        )? {
            WorkflowLaunchAdmission::StartNow => {
                let workflow_run = self.sessions_mut().invoke_workflow_endpoint(
                    session_id,
                    workflow.id(),
                    endpoint.id(),
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
                Ok(WorkflowLaunchOutcome::Started {
                    workflow_run,
                    workflow,
                    endpoint,
                })
            }
            WorkflowLaunchAdmission::Queued(queued_launch) => Ok(WorkflowLaunchOutcome::Queued {
                queued_launch,
                workflow,
                endpoint,
            }),
        }
    }

    pub fn invoke_workflow_endpoint_and_schedule(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        prompt: Option<String>,
    ) -> Result<(WorkflowRun, WorkflowDefinition, WorkflowEndpointDefinition), DaemonError> {
        match self.invoke_workflow_endpoint_with_admission(session_id, workflow_ref, endpoint_ref, prompt)? {
            WorkflowLaunchOutcome::Started {
                workflow_run,
                workflow,
                endpoint,
            } => Ok((workflow_run, workflow, endpoint)),
            WorkflowLaunchOutcome::Queued { workflow, endpoint, .. } => Err(DaemonError::WorkflowLaunchRejected {
                session_id: session_id.to_string(),
                workflow_id: workflow.id().to_string(),
                endpoint_id: endpoint.id().to_string(),
                message: "workflow launch was queued instead of started".to_string(),
            }),
        }
    }

    pub fn drain_session_workflow_launch_queue(
        &mut self,
        session_id: &str,
    ) -> Result<Option<WorkflowLaunchOutcome>, DaemonError> {
        let Some(queued_launch) = self.sessions_mut().dequeue_next_workflow_launch(session_id)? else {
            return Ok(None);
        };
        if let Some(watchdog_id) = queued_launch.watchdog_id() {
            let _ = self
                .sessions_mut()
                .mark_workflow_watchdog_pending_started(session_id, watchdog_id);
        }
        let outcome = self.invoke_queued_workflow_launch(session_id, queued_launch.clone());
        match outcome {
            Ok(outcome) => Ok(Some(outcome)),
            Err(error) => {
                if let Some(watchdog_id) = queued_launch.watchdog_id() {
                    let _ = self.sessions_mut().mark_workflow_watchdog_failed(
                        session_id,
                        watchdog_id,
                        error.to_string(),
                    );
                }
                self.record_notice(
                    session_id,
                    None,
                    self.attachments().list_session_attachment_ids(session_id),
                    format!(
                        "Queued workflow launch `{}` failed: {}",
                        queued_launch.id(),
                        error
                    ),
                );
                Ok(None)
            }
        }
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
            match self.invoke_watchdog_workflow_launch(plan) {
                Ok(()) => {}
                Err(error) => {
                    crate::logging::warn_with_fields(
                        "daemon.app",
                        "workflow watchdog invoke failed",
                        serde_json::json!({ "error": error.to_string() }),
                    );
                }
            }
        }
    }

    fn invoke_watchdog_workflow_launch(
        &mut self,
        plan: WorkflowWatchdogTickPlan,
    ) -> Result<(), DaemonError> {
        match self.invoke_queued_workflow_launch(
            &plan.session_id,
            QueuedWorkflowLaunch::new(
                format!("watchdog-launch-{}", plan.watchdog_id),
                plan.workflow_id.clone(),
                plan.endpoint_id.clone(),
                Some(plan.invocation_prompt.clone()),
                QueuedWorkflowLaunchSource::Watchdog,
                Some(plan.watchdog_id.clone()),
            ),
        ) {
            Ok(WorkflowLaunchOutcome::Started { .. }) => Ok(()),
            Ok(WorkflowLaunchOutcome::Queued { .. }) => Ok(()),
            Err(error) => {
                let _ = self.sessions_mut().mark_workflow_watchdog_failed(
                    &plan.session_id,
                    &plan.watchdog_id,
                    error.to_string(),
                );
                Err(error)
            }
        }
    }

    fn invoke_queued_workflow_launch(
        &mut self,
        session_id: &str,
        queued_launch: QueuedWorkflowLaunch,
    ) -> Result<WorkflowLaunchOutcome, DaemonError> {
        let workflow = self
            .sessions()
            .resolve_workflow_ref(session_id, queued_launch.workflow_id())?;
        let endpoint = self.sessions().resolve_workflow_endpoint_ref(
            session_id,
            queued_launch.workflow_id(),
            queued_launch.endpoint_id(),
        )?;
        crate::scheduler::runtime::validate_workflow_agents(self, session_id, &workflow)?;
        let workflow_run = self.sessions_mut().invoke_workflow_endpoint(
            session_id,
            workflow.id(),
            endpoint.id(),
            queued_launch.invocation_prompt().map(str::to_string),
        )?;
        crate::scheduler::runtime::schedule_workflow_run_entry_node(self, session_id, &workflow_run)?;
        let workflow_run = self
            .sessions()
            .resolve_workflow_run_ref(session_id, workflow_run.id())?;
        if let Some(watchdog_id) = queued_launch.watchdog_id() {
            let _ = self
                .sessions_mut()
                .mark_workflow_watchdog_invoked(session_id, watchdog_id, workflow_run.id());
        }
        Ok(WorkflowLaunchOutcome::Started {
            workflow_run,
            workflow,
            endpoint,
        })
    }
}
