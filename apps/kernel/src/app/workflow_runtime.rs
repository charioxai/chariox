use crate::app::{provider_runtime::ProviderProcessTracker, DaemonApp};
use crate::error::DaemonError;
use crate::session::{
    unix_epoch_ms, PromptQueueItem, WorkflowDefinition, WorkflowEndpointDefinition,
    WorkflowQueuedPrompt, WorkflowQueuedPromptSource, WorkflowRun, WorkflowWatchdogTickPlan,
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
    Enqueued {
        queued_prompt: WorkflowQueuedPrompt,
        workflow: WorkflowDefinition,
        endpoint: WorkflowEndpointDefinition,
    },
}

impl DaemonApp {
    pub fn enqueue_workflow_prompt_and_maybe_start(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        prompt: Option<String>,
        queue_ref: Option<&str>,
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
        let queued_prompt = self.sessions_mut().enqueue_workflow_prompt(
            session_id,
            workflow.id(),
            endpoint.id(),
            prompt,
            queue_ref,
            WorkflowQueuedPromptSource::Manual,
            None,
        )?;
        if self
            .sessions()
            .get_session(session_id)?
            .has_active_workflow_run()
        {
            return Ok(WorkflowLaunchOutcome::Enqueued {
                queued_prompt,
                workflow,
                endpoint,
            });
        }
        self.start_next_queued_workflow_prompt(session_id)?
            .ok_or_else(|| DaemonError::WorkflowLaunchRejected {
                session_id: session_id.to_string(),
                workflow_id: workflow.id().to_string(),
                endpoint_id: endpoint.id().to_string(),
                message: "workflow prompt was enqueued but no dispatchable queue item was found"
                    .to_string(),
            })
    }

    pub fn invoke_workflow_endpoint_and_schedule(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        prompt: Option<String>,
    ) -> Result<(WorkflowRun, WorkflowDefinition, WorkflowEndpointDefinition), DaemonError> {
        match self.enqueue_workflow_prompt_and_maybe_start(
            session_id,
            workflow_ref,
            endpoint_ref,
            prompt,
            None,
        )? {
            WorkflowLaunchOutcome::Started {
                workflow_run,
                workflow,
                endpoint,
            } => Ok((workflow_run, workflow, endpoint)),
            WorkflowLaunchOutcome::Enqueued {
                workflow, endpoint, ..
            } => Err(DaemonError::WorkflowLaunchRejected {
                session_id: session_id.to_string(),
                workflow_id: workflow.id().to_string(),
                endpoint_id: endpoint.id().to_string(),
                message: "workflow launch was queued instead of started".to_string(),
            }),
        }
    }

    pub fn start_next_queued_workflow_prompt(
        &mut self,
        session_id: &str,
    ) -> Result<Option<WorkflowLaunchOutcome>, DaemonError> {
        let Some(queued_prompt) = self
            .sessions_mut()
            .dequeue_next_workflow_prompt(session_id)?
        else {
            return Ok(None);
        };
        if let Some(watchdog_id) = queued_prompt.watchdog_id() {
            let _ = self
                .sessions_mut()
                .mark_workflow_watchdog_pending_started(session_id, watchdog_id);
        }
        let outcome = self.invoke_queued_workflow_prompt(session_id, queued_prompt.clone());
        match outcome {
            Ok(outcome) => Ok(Some(outcome)),
            Err(error) => {
                if let Some(watchdog_id) = queued_prompt.watchdog_id() {
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
                        "Queued workflow prompt `{}` failed: {}",
                        queued_prompt.id(),
                        error
                    ),
                );
                Ok(None)
            }
        }
    }

    fn invoke_queued_workflow_prompt(
        &mut self,
        session_id: &str,
        queued_prompt: WorkflowQueuedPrompt,
    ) -> Result<WorkflowLaunchOutcome, DaemonError> {
        let workflow = self
            .sessions()
            .resolve_workflow_ref(session_id, queued_prompt.workflow_id())?;
        let endpoint = self.sessions().resolve_workflow_endpoint_ref(
            session_id,
            queued_prompt.workflow_id(),
            queued_prompt.endpoint_id(),
        )?;
        WorkflowProgression::validate_agents(self, session_id, &workflow)?;
        self.flush_workflow_agent_context_if_needed(session_id, &workflow)?;
        let workflow_run = self.sessions_mut().invoke_workflow_endpoint(
            session_id,
            workflow.id(),
            endpoint.id(),
            queued_prompt.prompt().map(str::to_string),
        )?;
        if let Err(error) =
            WorkflowProgression::schedule_entry_node(self, session_id, &workflow_run)
        {
            if let Some(node_run) = workflow_run.node_runs().first() {
                let _ = self.sessions_mut().fail_workflow_node_run(
                    session_id,
                    workflow_run.id(),
                    node_run.id(),
                );
            }
            return Err(error);
        }
        let workflow_run = self
            .sessions()
            .resolve_workflow_run_ref(session_id, workflow_run.id())?;
        if let Some(watchdog_id) = queued_prompt.watchdog_id() {
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
        self.clear_workflow_agent_resume_state(session_id, &workflow_agent_ids)?;
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

    fn clear_workflow_agent_resume_state(
        &mut self,
        session_id: &str,
        workflow_agent_ids: &BTreeSet<String>,
    ) -> Result<(), DaemonError> {
        for agent_id in workflow_agent_ids {
            let agent = self.agents().get_agent(agent_id)?;
            if agent.provider_resume_state().is_empty() {
                continue;
            }
            let updated = self.agents.set_agent_runtime_profile(
                agent_id,
                agent.provider(),
                agent.model().map(str::to_string),
                agent.effort().map(str::to_string),
                crate::provider::ProviderResumeState::default(),
            )?;
            let _ = self.durable_state_store().append_event(
                "agent.runtime_profile_updated",
                Some(updated.id().to_string()),
                serde_json::json!({
                    "agent": &updated,
                    "reason": "workflow_context_flushed",
                }),
            );
            self.record_notice(
                session_id,
                None,
                self.attachments().list_session_attachment_ids(session_id),
                format!(
                    "Workflow context flush cleared provider resume state for agent `{}`.",
                    updated.id()
                ),
            );
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
    let queued_prompt = app.sessions_mut().enqueue_workflow_prompt(
        &plan.session_id,
        &plan.workflow_id,
        &plan.endpoint_id,
        Some(plan.invocation_prompt.clone()),
        None,
        WorkflowQueuedPromptSource::Watchdog,
        Some(plan.watchdog_id.clone()),
    )?;
    if app
        .sessions()
        .get_session(&plan.session_id)?
        .has_active_workflow_run()
    {
        return Ok(());
    }
    match app.invoke_queued_workflow_prompt(&plan.session_id, queued_prompt) {
        Ok(WorkflowLaunchOutcome::Started { .. }) => Ok(()),
        Ok(WorkflowLaunchOutcome::Enqueued { .. }) => Ok(()),
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
    if session
        .workflow_run(workflow_run_id)
        .and_then(|workflow_run| {
            workflow_run
                .node_runs()
                .iter()
                .find(|node_run| node_run.id() == workflow_node_run_id)
        })
        .is_some_and(|node_run| node_run.has_valid_pending_final_output())
    {
        return true;
    }
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
