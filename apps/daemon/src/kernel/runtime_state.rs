use std::sync::Arc;

use tokio::sync::Mutex;

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::local::LocalDaemonResponse;

#[derive(Clone)]
pub(crate) struct CompatibilityRuntimeState {
    app: Arc<Mutex<DaemonApp>>,
}

impl CompatibilityRuntimeState {
    pub(crate) fn new(app: Arc<Mutex<DaemonApp>>) -> Self {
        Self { app }
    }

    pub(crate) fn app(&self) -> Arc<Mutex<DaemonApp>> {
        Arc::clone(&self.app)
    }

    pub(crate) async fn with_app_mut<R>(&self, operation: impl FnOnce(&mut DaemonApp) -> R) -> R {
        let mut app = self.app.lock().await;
        operation(&mut app)
    }

    pub(crate) async fn with_session_mut<R>(
        &self,
        operation: impl FnOnce(&mut SessionRuntimeCompatibilityContext<'_>) -> R,
    ) -> R {
        self.with_app_mut(|app| {
            let mut context = SessionRuntimeCompatibilityContext::new(app);
            operation(&mut context)
        })
        .await
    }

    pub(crate) async fn with_agent_mut<R>(
        &self,
        operation: impl FnOnce(&mut AgentRuntimeCompatibilityContext<'_>) -> R,
    ) -> R {
        self.with_app_mut(|app| {
            let mut context = AgentRuntimeCompatibilityContext::new(app);
            operation(&mut context)
        })
        .await
    }

    pub(crate) async fn with_agent_prompt_mut<R>(
        &self,
        operation: impl FnOnce(&mut AgentPromptCompatibilityContext<'_>) -> R,
    ) -> R {
        self.with_app_mut(|app| {
            let mut context = AgentPromptCompatibilityContext::new(app);
            operation(&mut context)
        })
        .await
    }

    pub(crate) async fn with_workflow_mut<R>(
        &self,
        operation: impl FnOnce(&mut WorkflowRuntimeCompatibilityContext<'_>) -> R,
    ) -> R {
        self.with_app_mut(|app| {
            let mut context = WorkflowRuntimeCompatibilityContext::new(app);
            operation(&mut context)
        })
        .await
    }
}

pub(crate) struct SessionRuntimeCompatibilityContext<'a> {
    app: &'a mut DaemonApp,
}

pub(crate) struct AgentRuntimeCompatibilityContext<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> AgentRuntimeCompatibilityContext<'a> {
    fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn active_prompt_agent_id(
        &mut self,
        session_id: &str,
    ) -> Result<Option<String>, DaemonError> {
        self.app.prompt_owner_active_prompt_agent_id(session_id)
    }

    pub(crate) fn focused_agent_id(&self, session_id: &str) -> Result<Option<String>, DaemonError> {
        Ok(self
            .app
            .sessions()
            .get_session(session_id)?
            .focused_agent_id()
            .map(str::to_string))
    }
}

pub(crate) struct AgentPromptCompatibilityContext<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> AgentPromptCompatibilityContext<'a> {
    fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn submit_prepared_prompt(
        &mut self,
        prepared: crate::app::KernelPreparedPromptSubmission,
    ) -> Result<crate::app::KernelPromptSubmission, DaemonError> {
        crate::app::KernelAgentService::new(self.app).submit_prepared_prompt_for_kernel(prepared)
    }

    pub(crate) fn cancel_agent_prompt(
        &mut self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
    ) -> Result<crate::app::KernelPromptCancellation, DaemonError> {
        crate::app::KernelAgentService::new(self.app).cancel_agent_prompt_for_kernel(
            session_id,
            target_agent_id,
            attachment_id,
        )
    }

    pub(crate) fn complete_agent_prompt(
        &mut self,
        session_id: &str,
        target_agent_id: &str,
        next_queued_prompt: Option<&crate::session::PromptQueueItem>,
    ) -> Result<crate::session::PromptCompletion, DaemonError> {
        let provider_run_id = self
            .app
            .providers()
            .get_run_for_agent(session_id, target_agent_id)
            .map(|run| run.id().to_string());
        crate::app::KernelAgentService::new(self.app).complete_active_prompt_for_kernel(
            session_id,
            target_agent_id,
            provider_run_id.as_deref(),
            next_queued_prompt,
        )
    }
}

pub(crate) struct WorkflowRuntimeCompatibilityContext<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> WorkflowRuntimeCompatibilityContext<'a> {
    fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn execute_service_operation(
        &mut self,
        session_id: &str,
        operation: impl FnOnce(
            &mut crate::app::KernelWorkflowService<'_>,
        ) -> Result<LocalDaemonResponse, DaemonError>,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<crate::session::RuntimeSession>,
    ) {
        let result = {
            let mut workflows = crate::app::KernelWorkflowService::new(self.app);
            operation(&mut workflows)
        };
        let projected_session = if let Ok(response) = result.as_ref() {
            workflow_response_session(response).or_else(|| {
                crate::app::KernelSessionReadService::new(self.app)
                    .session_snapshot(session_id)
                    .ok()
            })
        } else {
            None
        };
        (result, projected_session)
    }
}

fn workflow_response_session(
    response: &LocalDaemonResponse,
) -> Option<crate::session::RuntimeSession> {
    match response {
        LocalDaemonResponse::WorkflowCreated { session, .. }
        | LocalDaemonResponse::WorkflowAliased { session, .. }
        | LocalDaemonResponse::WorkflowEndpointCreated { session, .. }
        | LocalDaemonResponse::WorkflowEndpointAliased { session, .. }
        | LocalDaemonResponse::WorkflowEndpointBound { session, .. }
        | LocalDaemonResponse::WorkflowNodeAdded { session, .. }
        | LocalDaemonResponse::WorkflowNodeRemoved { session, .. }
        | LocalDaemonResponse::WorkflowNodeInstructionsUpdated { session, .. }
        | LocalDaemonResponse::WorkflowNodeCanCompleteRunUpdated { session, .. }
        | LocalDaemonResponse::WorkflowNodeCanEmitIntermediateOutputUpdated { session, .. }
        | LocalDaemonResponse::WorkflowNodeIntermediateOutputSchemaUpdated { session, .. }
        | LocalDaemonResponse::WorkflowNodeMaxTurnsUpdated { session, .. }
        | LocalDaemonResponse::WorkflowEdgeAdded { session, .. }
        | LocalDaemonResponse::WorkflowEdgeRemoved { session, .. }
        | LocalDaemonResponse::WorkflowRunInvoked { session, .. }
        | LocalDaemonResponse::WorkflowRunQueued { session, .. }
        | LocalDaemonResponse::WorkflowRunCancelled { session, .. }
        | LocalDaemonResponse::WorkflowRunResumed { session, .. }
        | LocalDaemonResponse::WorkflowWatchdogCreated { session, .. }
        | LocalDaemonResponse::WorkflowWatchdogUpdated { session, .. }
        | LocalDaemonResponse::WorkflowWatchdogRemoved { session, .. }
        | LocalDaemonResponse::WorkflowFlushContextUpdated { session, .. }
        | LocalDaemonResponse::WorkflowRunOutputSchemaUpdated { session, .. }
        | LocalDaemonResponse::WorkflowIntermediateOutputSchemaUpdated { session, .. }
        | LocalDaemonResponse::WorkflowLaunchPolicyUpdated { session, .. }
        | LocalDaemonResponse::QueuedWorkflowLaunchRemoved { session, .. }
        | LocalDaemonResponse::QueuedWorkflowLaunchesCleared { session, .. }
        | LocalDaemonResponse::WorkflowTurnAcknowledged { session, .. } => Some(session.clone()),
        _ => None,
    }
}

impl<'a> SessionRuntimeCompatibilityContext<'a> {
    fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn resolve_session_ref_id(
        &mut self,
        session_ref: &str,
        workspace_id: Option<&str>,
    ) -> Result<String, DaemonError> {
        crate::app::KernelSessionService::new(self.app)
            .resolve_session_ref_id(session_ref, workspace_id)
    }

    pub(crate) fn attachment_session_id(
        &mut self,
        attachment_id: &str,
    ) -> Result<String, DaemonError> {
        crate::app::KernelSessionService::new(self.app).attachment_session_id(attachment_id)
    }

    pub(crate) fn session_snapshot(
        &mut self,
        session_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        crate::app::KernelSessionService::new(self.app).session_snapshot(session_id)
    }

    pub(crate) fn create_session_response(
        &mut self,
        request: crate::session::CreateSessionRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        crate::app::KernelSessionService::new(self.app).create_session_response(request)
    }

    pub(crate) fn attach(
        &mut self,
        request: crate::attachment::AttachRequest,
    ) -> Result<crate::attachment::RuntimeAttachment, DaemonError> {
        crate::app::KernelSessionService::new(self.app).attach(request)
    }

    pub(crate) fn detach(
        &mut self,
        attachment_id: &str,
    ) -> Result<crate::attachment::RuntimeAttachment, DaemonError> {
        crate::app::KernelSessionService::new(self.app).detach(attachment_id)
    }

    pub(crate) fn focus_agent(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        crate::app::KernelSessionService::new(self.app).focus_agent(session_id, agent_id)
    }

    pub(crate) fn cycle_agent_focus(
        &mut self,
        session_id: &str,
    ) -> Result<Option<crate::agent::AgentInstance>, DaemonError> {
        crate::app::KernelSessionService::new(self.app).cycle_agent_focus(session_id)
    }

    pub(crate) fn resize_terminal(
        &mut self,
        session_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), DaemonError> {
        crate::app::KernelSessionService::new(self.app).resize_terminal(session_id, cols, rows)
    }

    pub(crate) fn ensure_attachment_in_session(
        &mut self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<(), DaemonError> {
        let _ = crate::app::KernelSessionService::new(self.app)
            .ensure_attachment_in_session(session_id, attachment_id)?;
        Ok(())
    }

    pub(crate) fn drain_notice_records(
        &mut self,
        session_id: &str,
        attachment_id: &str,
    ) -> Vec<crate::terminal::RuntimeNoticeRecord> {
        self.app
            .terminal()
            .drain_notice_records(session_id, attachment_id)
    }

    pub(crate) fn update_session_config(
        &mut self,
        session_id: &str,
        attachment_id: &str,
        values: std::collections::BTreeMap<String, String>,
        requires_idle: bool,
    ) -> Result<crate::session::SessionConfigState, DaemonError> {
        crate::app::KernelSessionService::new(self.app).update_session_config(
            session_id,
            attachment_id,
            values,
            requires_idle,
        )
    }

    pub(crate) fn alias_session(
        &mut self,
        session_id: &str,
        alias: String,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        crate::app::KernelSessionService::new(self.app).alias_session(session_id, alias)
    }

    pub(crate) fn spawn_agent(
        &mut self,
        request: crate::agent::CreateAgentRequest,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        crate::app::KernelSessionService::new(self.app).spawn_agent(request)
    }

    pub(crate) fn destroy_agent(
        &mut self,
        agent_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        crate::app::KernelSessionService::new(self.app).destroy_agent(agent_id)
    }

    pub(crate) fn end_session(
        &mut self,
        session_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        crate::app::KernelSessionService::new(self.app).end_session(session_id)
    }

    pub(crate) fn delete_session_ref(
        &mut self,
        session_ref: &str,
        workspace_id: Option<&str>,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        crate::app::KernelSessionService::new(self.app)
            .delete_session_ref(session_ref, workspace_id)
    }
}
