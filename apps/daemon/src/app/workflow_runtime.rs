use std::path::PathBuf;

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::LaunchProviderRequest;
use crate::session::{
    PromptQueueItem, PromptSubmissionOutcome, WorkflowCompletionUpdate, WorkflowDispatch,
    WorkflowDefinition, WorkflowEndpointDefinition, WorkflowMessage, WorkflowRun,
};

const WORKFLOW_PROMPT_SOURCE_PREFIX: &str = "workflow-run:";

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
        let endpoint = self
            .sessions()
            .resolve_workflow_endpoint_ref(session_id, workflow_ref, endpoint_ref)?;
        let workflow_run =
            self.sessions_mut()
                .invoke_workflow_endpoint(session_id, workflow_ref, endpoint_ref, prompt)?;
        if workflow_run
            .invocation_prompt()
            .is_some_and(|value| !value.trim().is_empty())
        {
            self.schedule_workflow_run_entry_node(session_id, &workflow_run)?;
        }
        let workflow_run = self
            .sessions()
            .resolve_workflow_run_ref(session_id, workflow_run.id())?;
        Ok((workflow_run, workflow, endpoint))
    }

    pub(crate) fn is_workflow_prompt_source_attachment_id(attachment_id: &str) -> bool {
        attachment_id.starts_with(WORKFLOW_PROMPT_SOURCE_PREFIX)
    }

    fn workflow_prompt_source_attachment_id(workflow_run_id: &str) -> String {
        format!("{WORKFLOW_PROMPT_SOURCE_PREFIX}{workflow_run_id}")
    }

    fn schedule_workflow_run_entry_node(
        &mut self,
        session_id: &str,
        workflow_run: &WorkflowRun,
    ) -> Result<(), DaemonError> {
        let prompt = workflow_run
            .invocation_prompt()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| DaemonError::InvalidWorkflowRunState {
                workflow_run_id: workflow_run.id().to_string(),
                status: workflow_run.status(),
                operation: "schedule workflow run entry node",
            })?;
        let node_run = workflow_run
            .node_runs()
            .first()
            .ok_or_else(|| DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_run.workflow_id().to_string(),
                reference: workflow_run.id().to_string(),
                message: "workflow run has no entry node run",
            })?;
        self.schedule_workflow_node_prompt(
            session_id,
            workflow_run.id(),
            node_run.id(),
            node_run.agent_id(),
            node_run.node_id(),
            prompt,
        )
    }

    fn schedule_workflow_node_prompt(
        &mut self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
        target_agent_id: &str,
        node_id: &str,
        prompt: &str,
    ) -> Result<(), DaemonError> {
        let (_session, outcome) = self.sessions_mut().submit_workflow_prompt(
            session_id,
            &Self::workflow_prompt_source_attachment_id(workflow_run_id),
            target_agent_id,
            workflow_run_id,
            workflow_node_run_id,
            prompt.to_string(),
        )?;

        match outcome {
            PromptSubmissionOutcome::Started { prompt } => {
                let provider_run_id =
                    self.ensure_workflow_provider_run_for_agent(session_id, target_agent_id)?;
                if let Err(error) = self.dispatch_prompt_to_provider(
                    session_id,
                    &provider_run_id,
                    prompt.source_attachment_id(),
                    prompt.prompt(),
                    prompt.attachments(),
                ) {
                    if let Ok((_, cancelled)) = self.sessions_mut().cancel_active_prompt(session_id) {
                        let _ = self.reconcile_workflow_prompt_cancelled(session_id, &cancelled);
                    }
                    self.clear_prompt_activity(session_id);
                    return Err(error);
                }
                self.reconcile_workflow_prompt_started(session_id, &prompt)?;
                self.note_prompt_started(session_id);
            }
            PromptSubmissionOutcome::Queued { .. } => {
                self.record_notice(
                    session_id,
                    None,
                    self.attachments().list_session_attachment_ids(session_id),
                    format!(
                        "Workflow run `{workflow_run_id}` queued node `{node_id}` behind the current active prompt."
                    ),
                );
            }
        }

        Ok(())
    }

    fn schedule_workflow_dispatches(
        &mut self,
        session_id: &str,
        workflow_run_id: &str,
        dispatches: &[WorkflowDispatch],
    ) {
        for dispatch in dispatches {
            self.record_notice(
                session_id,
                None,
                self.attachments().list_session_attachment_ids(session_id),
                format!(
                    "Workflow run `{workflow_run_id}` routed `{}` to node `{}`.",
                    dispatch.message.id(),
                    dispatch.node_run.node_id()
                ),
            );
            if let Err(error) = self.schedule_workflow_node_prompt(
                session_id,
                workflow_run_id,
                dispatch.node_run.id(),
                dispatch.node_run.agent_id(),
                dispatch.node_run.node_id(),
                &Self::build_workflow_handoff_prompt(&dispatch.message),
            ) {
                self.record_notice(
                    session_id,
                    None,
                    self.attachments().list_session_attachment_ids(session_id),
                    format!(
                        "Workflow run `{workflow_run_id}` could not schedule downstream node `{}`: {}",
                        dispatch.node_run.node_id(),
                        error
                    ),
                );
            }
        }
    }

    fn build_workflow_handoff_prompt(message: &WorkflowMessage) -> String {
        format!(
            "Workflow handoff payload (JSON):\n{}\n\nExecute workflow node `{}` using this payload as the authoritative upstream context.",
            message.handoff_payload(),
            message.target_node_id()
        )
    }

    pub(crate) fn ensure_workflow_provider_run_for_agent(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<String, DaemonError> {
        match self.ensure_active_provider_run_for_agent(session_id, agent_id) {
            Ok(provider_run_id) => Ok(provider_run_id),
            Err(DaemonError::NoActiveProviderRun { .. }) => {
                let agent = self.agents().get_agent(agent_id)?;
                let adapter_key = match agent.provider() {
                    "default" => "opencode",
                    value => value,
                };
                let provider = match agent.provider() {
                    "default" => "opencode",
                    value => value,
                };
                let mut request = LaunchProviderRequest::new(
                    session_id,
                    adapter_key,
                    provider,
                    "default",
                    agent.model().unwrap_or("default"),
                )
                .with_agent_id(agent.id().to_string())
                .with_variant(agent.effort().map(str::to_string));
                if let Some(worktree_id) = agent.worktree_id() {
                    request = request.with_working_directory(PathBuf::from(worktree_id));
                }
                let provider_run = self.launch_provider(request)?;
                Ok(provider_run.id().to_string())
            }
            Err(error) => Err(error),
        }
    }

    pub(crate) fn reconcile_workflow_prompt_started(
        &mut self,
        session_id: &str,
        prompt: &PromptQueueItem,
    ) -> Result<(), DaemonError> {
        let (Some(workflow_run_id), Some(workflow_node_run_id)) =
            (prompt.workflow_run_id(), prompt.workflow_node_run_id())
        else {
            return Ok(());
        };
        let workflow_run = self.sessions_mut().start_workflow_node_run(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        )?;
        self.record_notice(
            session_id,
            self.sessions()
                .get_session(session_id)?
                .active_provider_run_id(),
            self.attachments().list_session_attachment_ids(session_id),
            format!(
                "Workflow run `{}` started on agent `{}`.",
                workflow_run.id(),
                prompt.target_agent_id()
            ),
        );
        Ok(())
    }

    pub(crate) fn reconcile_workflow_prompt_completed(
        &mut self,
        session_id: &str,
        prompt: &PromptQueueItem,
    ) -> Result<(), DaemonError> {
        let (Some(workflow_run_id), Some(workflow_node_run_id)) =
            (prompt.workflow_run_id(), prompt.workflow_node_run_id())
        else {
            return Ok(());
        };
        let WorkflowCompletionUpdate {
            workflow_run,
            dispatches,
        } = self.sessions_mut().complete_workflow_node_run(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        )?;
        self.schedule_workflow_dispatches(session_id, workflow_run.id(), &dispatches);
        let state_suffix = match workflow_run.status() {
            crate::session::WorkflowRunStatus::Waiting => "waiting for downstream handoffs",
            crate::session::WorkflowRunStatus::Completed => "completed",
            _ => "updated",
        };
        self.record_notice(
            session_id,
            None,
            self.attachments().list_session_attachment_ids(session_id),
            format!("Workflow run `{}` {state_suffix}.", workflow_run.id()),
        );
        Ok(())
    }

    pub(crate) fn reconcile_workflow_prompt_cancelled(
        &mut self,
        session_id: &str,
        prompt: &PromptQueueItem,
    ) -> Result<(), DaemonError> {
        let (Some(workflow_run_id), Some(workflow_node_run_id)) =
            (prompt.workflow_run_id(), prompt.workflow_node_run_id())
        else {
            return Ok(());
        };
        let workflow_run = self.sessions_mut().stop_workflow_node_run(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        )?;
        self.record_notice(
            session_id,
            None,
            self.attachments().list_session_attachment_ids(session_id),
            format!("Workflow run `{}` was stopped.", workflow_run.id()),
        );
        Ok(())
    }
}
