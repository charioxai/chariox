use crate::app::{provider_runtime::ProviderProcessTracker, DaemonApp};
use crate::error::DaemonError;
use crate::session::{
    unix_epoch_ms, PromptQueueItem, QueuedWorkflowLaunch, QueuedWorkflowLaunchSource,
    WorkflowDefinition, WorkflowEndpointDefinition, WorkflowLaunchAdmission, WorkflowRun,
    WorkflowWatchdogTickPlan,
};
use std::collections::BTreeSet;

struct WorkflowProgression;

impl WorkflowProgression {
    fn is_workflow_prompt_attachment(attachment_id: &str) -> bool {
        crate::scheduler::runtime::is_workflow_prompt_attachment(attachment_id)
    }

    fn ensure_provider_run(
        app: &mut DaemonApp,
        session_id: &str,
        agent_id: &str,
    ) -> Result<String, DaemonError> {
        crate::scheduler::runtime::ensure_workflow_provider_run_for_agent(app, session_id, agent_id)
    }

    fn validate_agents(
        app: &DaemonApp,
        session_id: &str,
        workflow: &WorkflowDefinition,
    ) -> Result<(), DaemonError> {
        crate::scheduler::runtime::validate_workflow_agents(app, session_id, workflow)
    }

    fn schedule_entry_node(
        app: &mut DaemonApp,
        session_id: &str,
        workflow_run: &WorkflowRun,
    ) -> Result<(), DaemonError> {
        crate::scheduler::runtime::schedule_workflow_run_entry_node(app, session_id, workflow_run)
    }

    fn on_prompt_started(
        app: &mut DaemonApp,
        session_id: &str,
        prompt: &PromptQueueItem,
    ) -> Result<(), DaemonError> {
        crate::scheduler::runtime::on_workflow_prompt_started(app, session_id, prompt)
    }

    fn on_prompt_completed(
        app: &mut DaemonApp,
        session_id: &str,
        prompt: &PromptQueueItem,
        provider_run_id: Option<&str>,
    ) -> Result<(), DaemonError> {
        crate::scheduler::runtime::on_workflow_prompt_completed(
            app,
            session_id,
            prompt,
            provider_run_id,
        )
    }

    fn on_prompt_cancelled(
        app: &mut DaemonApp,
        session_id: &str,
        prompt: &PromptQueueItem,
    ) -> Result<(), DaemonError> {
        crate::scheduler::runtime::on_workflow_prompt_cancelled(app, session_id, prompt)
    }

    fn retry_blocked_claims(app: &mut DaemonApp) -> BTreeSet<String> {
        crate::scheduler::runtime::retry_blocked_workflow_claims(app)
    }
}

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
        let workflow = self
            .sessions()
            .resolve_workflow_ref(session_id, workflow_ref)?;
        let endpoint = self.sessions().resolve_workflow_endpoint_ref(
            session_id,
            workflow_ref,
            endpoint_ref,
        )?;
        WorkflowProgression::validate_agents(self, session_id, &workflow)?;
        let admission = {
            self.sessions_mut().admit_manual_workflow_launch(
                session_id,
                workflow.id(),
                endpoint.id(),
                prompt.clone(),
            )?
        };
        match admission {
            WorkflowLaunchAdmission::StartNow => {
                self.flush_workflow_agent_context_if_needed(session_id, &workflow)?;
                let workflow_run = self.sessions_mut().invoke_workflow_endpoint(
                    session_id,
                    workflow.id(),
                    endpoint.id(),
                    prompt,
                )?;
                WorkflowProgression::schedule_entry_node(self, session_id, &workflow_run)?;
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
        match self.invoke_workflow_endpoint_with_admission(
            session_id,
            workflow_ref,
            endpoint_ref,
            prompt,
        )? {
            WorkflowLaunchOutcome::Started {
                workflow_run,
                workflow,
                endpoint,
            } => Ok((workflow_run, workflow, endpoint)),
            WorkflowLaunchOutcome::Queued {
                workflow, endpoint, ..
            } => Err(DaemonError::WorkflowLaunchRejected {
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
        let Some(queued_launch) = self
            .sessions_mut()
            .dequeue_next_workflow_launch(session_id)?
        else {
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
        WorkflowProgression::validate_agents(self, session_id, &workflow)?;
        self.flush_workflow_agent_context_if_needed(session_id, &workflow)?;
        let workflow_run = self.sessions_mut().invoke_workflow_endpoint(
            session_id,
            workflow.id(),
            endpoint.id(),
            queued_launch.invocation_prompt().map(str::to_string),
        )?;
        WorkflowProgression::schedule_entry_node(self, session_id, &workflow_run)?;
        let workflow_run = self
            .sessions()
            .resolve_workflow_run_ref(session_id, workflow_run.id())?;
        if let Some(watchdog_id) = queued_launch.watchdog_id() {
            let _ = self.sessions_mut().mark_workflow_watchdog_invoked(
                session_id,
                watchdog_id,
                workflow_run.id(),
            );
        }
        Ok(WorkflowLaunchOutcome::Started {
            workflow_run,
            workflow,
            endpoint,
        })
    }

    pub(crate) fn flush_workflow_agent_context_if_needed(
        &mut self,
        session_id: &str,
        workflow: &WorkflowDefinition,
    ) -> Result<(), DaemonError> {
        if !workflow.flush_agent_context_before_run() {
            return Ok(());
        }
        let workflow_agent_ids = workflow
            .nodes()
            .iter()
            .map(|node| node.agent_id().to_string())
            .collect::<BTreeSet<_>>();
        if workflow_agent_ids.is_empty() {
            return Ok(());
        }
        let should_cancel_active_prompt = self
            .sessions()
            .get_session(session_id)?
            .active_prompt()
            .map(|prompt| prompt.target_agent_id())
            .is_some_and(|agent_id| workflow_agent_ids.contains(agent_id));
        if should_cancel_active_prompt {
            let _ = self.cancel_active_prompt_for_runtime(session_id)?;
        }
        for agent_id in workflow_agent_ids {
            if let Some(run) = self.providers().get_run_for_agent(session_id, &agent_id) {
                if run.state() == crate::provider::ProviderRunState::Ended {
                    continue;
                }
                let outcome = self
                    .providers
                    .terminate_run_provider_only(session_id, run.id())?;
                let run = outcome.into_run();
                if self
                    .sessions
                    .get_session(session_id)?
                    .active_provider_run_id()
                    == Some(run.id())
                {
                    self.sessions.set_active_provider_run(session_id, None)?;
                }
                let _ = ProviderProcessTracker::new(self).remove_run(run.id());
            }
        }
        Ok(())
    }
}

pub(crate) fn is_workflow_prompt_source(attachment_id: &str) -> bool {
    WorkflowProgression::is_workflow_prompt_attachment(attachment_id)
}

pub(crate) fn pump_workflow_watchdogs(app: &mut DaemonApp) {
    let plans = match app
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
        match invoke_watchdog_workflow_launch(app, plan) {
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
    app: &mut DaemonApp,
    plan: WorkflowWatchdogTickPlan,
) -> Result<(), DaemonError> {
    match app.invoke_queued_workflow_launch(
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
            let _ = app.sessions_mut().mark_workflow_watchdog_failed(
                &plan.session_id,
                &plan.watchdog_id,
                error.to_string(),
            );
            Err(error)
        }
    }
}

pub(crate) fn start_workflow_prompt_from_runtime(
    app: &mut DaemonApp,
    session_id: &str,
    prompt: &PromptQueueItem,
) -> Result<(), DaemonError> {
    WorkflowProgression::on_prompt_started(app, session_id, prompt)
}

pub(crate) fn complete_workflow_prompt_from_runtime(
    app: &mut DaemonApp,
    session_id: &str,
    prompt: &PromptQueueItem,
    provider_run_id: Option<&str>,
) -> Result<(), DaemonError> {
    WorkflowProgression::on_prompt_completed(app, session_id, prompt, provider_run_id)
}

pub(crate) fn workflow_prompt_has_completion_output_from_runtime(
    app: &DaemonApp,
    session_id: &str,
    prompt: &PromptQueueItem,
    provider_run_id: Option<&str>,
) -> bool {
    let (Some(workflow_run_id), Some(workflow_node_run_id)) =
        (prompt.workflow_run_id(), prompt.workflow_node_run_id())
    else {
        return true;
    };
    let Ok(session) = app.sessions().get_session(session_id) else {
        return false;
    };
    let Ok(history) = crate::app::KernelSessionReadService::new(app).session_history(session_id)
    else {
        return false;
    };
    crate::scheduler::runtime::build_workflow_completion_snapshot_from_history(
        &session,
        history,
        session_id,
        workflow_run_id,
        workflow_node_run_id,
        provider_run_id.unwrap_or_default(),
    )
    .and_then(|snapshot| snapshot.output().cloned())
    .is_some()
}

pub(crate) fn cancel_workflow_prompt_from_runtime(
    app: &mut DaemonApp,
    session_id: &str,
    prompt: &PromptQueueItem,
) -> Result<(), DaemonError> {
    WorkflowProgression::on_prompt_cancelled(app, session_id, prompt)
}

pub(crate) fn ensure_workflow_provider_run_from_runtime(
    app: &mut DaemonApp,
    session_id: &str,
    agent_id: &str,
) -> Result<String, DaemonError> {
    WorkflowProgression::ensure_provider_run(app, session_id, agent_id)
}

pub(crate) fn retry_blocked_workflow_claims_from_runtime(app: &mut DaemonApp) {
    for session_id in WorkflowProgression::retry_blocked_claims(app) {
        let _ = crate::app::KernelSessionReadService::new(app).session_snapshot(&session_id);
    }
}
