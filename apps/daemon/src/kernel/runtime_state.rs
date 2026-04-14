use std::sync::Arc;

use tokio::sync::Mutex;

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::local::LocalDaemonResponse;
use crate::provider::ProviderRunOperationLanes;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use arroba_relay::protocol::ClientTarget;

#[derive(Clone)]
pub(crate) struct CompatibilityRuntimeState {
    app: Arc<Mutex<DaemonApp>>,
    owned: Option<CompatibilityRuntimeOwnedState>,
}

#[derive(Clone)]
pub(crate) struct CompatibilityRuntimeOwnedState {
    terminal_stream: crate::terminal::TerminalStreamStore,
    workspace_coordinator: crate::kernel::workspace_coordinator::WorkspaceCoordinator,
}

impl CompatibilityRuntimeState {
    #[cfg(test)]
    pub(crate) fn new(app: Arc<Mutex<DaemonApp>>) -> Self {
        Self { app, owned: None }
    }

    pub(crate) fn new_with_owned_state(
        app: Arc<Mutex<DaemonApp>>,
        terminal_stream: crate::terminal::TerminalStreamStore,
        workspace_coordinator: crate::kernel::workspace_coordinator::WorkspaceCoordinator,
    ) -> Self {
        Self {
            app,
            owned: Some(CompatibilityRuntimeOwnedState {
                terminal_stream,
                workspace_coordinator,
            }),
        }
    }

    pub(crate) async fn config_snapshot(&self) -> crate::config::DaemonConfig {
        self.with_app_mut(|app| app.config().clone()).await
    }

    async fn with_app_mut<R>(&self, operation: impl FnOnce(&mut DaemonApp) -> R) -> R {
        let mut app = self.app.lock().await;
        operation(&mut app)
    }

    pub(crate) async fn with_session_mut<R>(
        &self,
        operation: impl FnOnce(&mut SessionRuntimeCompatibilityContext<'_>) -> R,
    ) -> R {
        self.with_app_mut(|app| {
            let mut context = SessionRuntimeCompatibilityContext::new(
                app,
                self.owned
                    .as_ref()
                    .map(|owned| owned.terminal_stream.clone()),
            );
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

    pub(crate) fn spawn_prompt_dispatch(
        &self,
        dispatch: crate::app::KernelPromptDispatch,
        provider_runtime_lanes: ProviderRunOperationLanes,
    ) {
        let state = self.clone();
        tokio::spawn(async move {
            let _permit = provider_runtime_lanes
                .acquire(&dispatch.provider_run_id)
                .await;
            state
                .with_agent_prompt_mut(|prompt| {
                    if let Err(error) = prompt.enqueue_prompt_dispatch(&dispatch) {
                        let _ = prompt.fail_prompt_dispatch(dispatch, error);
                    }
                })
                .await;
        });
    }

    pub(crate) fn spawn_remote_prompt_dispatch(
        &self,
        dispatch: crate::app::KernelRemotePromptDispatch,
    ) {
        let state = self.clone();
        tokio::spawn(async move {
            let config = state.config_snapshot().await;
            let attachments = dispatch.attachments.clone();
            let serialized_attachments = match tokio::task::spawn_blocking(move || {
                crate::app::serialize_remote_prompt_attachments(&attachments)
            })
            .await
            {
                Ok(result) => result,
                Err(error) => Err(DaemonError::LocalTransport {
                    operation: "serialize remote prompt attachments",
                    message: error.to_string(),
                }),
            };
            let result = match serialized_attachments {
                Ok(attachments) => {
                    match crate::transport::relay_client::send_peer_request_via_temporary_connection(
                        &config,
                        ClientTarget {
                            daemon_id: Some(dispatch.worker_kernel_id.clone()),
                            daemon_alias: None,
                        },
                        RelayPeerRequest::SubmitLeasedPrompt {
                            leased_agent_id: dispatch.leased_agent_id.clone(),
                            prompt: dispatch.prompt.clone(),
                            attachments,
                            workflow_context: dispatch.workflow_context.clone(),
                        },
                    )
                    .await
                    {
                        Ok(RelayPeerResponse::LeasedPromptSubmitted {
                            provider_run_id, ..
                        }) => Ok(provider_run_id),
                        Ok(other) => Err(DaemonError::LocalTransport {
                            operation: "submit remote prepared prompt",
                            message: format!("unexpected remote prompt response: {other:?}"),
                        }),
                        Err(error) => Err(error),
                    }
                }
                Err(error) => Err(error),
            };
            state
                .with_agent_prompt_mut(|prompt| {
                    let _ = prompt.finish_remote_prompt_dispatch(dispatch, result);
                })
                .await;
        });
    }

    pub(crate) fn spawn_prompt_abort(
        &self,
        dispatch: crate::app::KernelPromptAbortDispatch,
        provider_runtime_lanes: ProviderRunOperationLanes,
    ) {
        let state = self.clone();
        tokio::spawn(async move {
            let _permit = provider_runtime_lanes
                .acquire(&dispatch.provider_run_id)
                .await;
            loop {
                let outcome = state
                    .with_agent_prompt_mut(|prompt| match prompt.enqueue_prompt_abort(&dispatch) {
                        Ok(()) => PromptAbortDispatchOutcome::Done,
                        Err(_)
                            if prompt.structured_prompt_io_in_flight(&dispatch.provider_run_id) =>
                        {
                            PromptAbortDispatchOutcome::Retry
                        }
                        Err(error) => {
                            let _ = prompt.fail_prompt_abort(dispatch.clone(), error);
                            PromptAbortDispatchOutcome::Done
                        }
                    })
                    .await;
                match outcome {
                    PromptAbortDispatchOutcome::Done => break,
                    PromptAbortDispatchOutcome::Retry => {
                        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                    }
                }
            }
        });
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

    pub(crate) async fn with_provider_launch_mut<R>(
        &self,
        operation: impl FnOnce(&mut ProviderLaunchCompatibilityContext<'_>) -> R,
    ) -> R {
        self.with_app_mut(|app| {
            let mut context = ProviderLaunchCompatibilityContext::new(app);
            operation(&mut context)
        })
        .await
    }

    pub(crate) async fn with_terminal_output_mut<R>(
        &self,
        operation: impl FnOnce(&mut TerminalOutputCompatibilityContext<'_>) -> R,
    ) -> R {
        self.with_app_mut(|app| {
            let mut context = TerminalOutputCompatibilityContext::new(app);
            operation(&mut context)
        })
        .await
    }

    pub(crate) async fn with_capability_runtime<R>(
        &self,
        operation: impl FnOnce(&CapabilityRuntimeCompatibilityContext<'_>) -> R,
    ) -> R {
        let app = self.app.lock().await;
        let context = CapabilityRuntimeCompatibilityContext::new(
            &app,
            self.owned
                .as_ref()
                .map(|owned| owned.workspace_coordinator.clone()),
        );
        operation(&context)
    }

    pub(crate) async fn dispatch_authenticated_runtime_tool_call(
        &self,
        auth_token: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        self.with_app_mut(|app| {
            crate::transport::runtime_tools::dispatch_authenticated_runtime_tool_call(
                app, auth_token, tool_name, arguments,
            )
        })
        .await
    }

    pub(crate) async fn dispatch_forwarded_workflow_runtime_tool_call(
        &self,
        context: crate::execution_lease::RemoteWorkflowTurnContext,
        tool_name: String,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        self.with_app_mut(|app| {
            crate::transport::runtime_tools::dispatch_forwarded_workflow_runtime_tool_call(
                app, context, tool_name, arguments,
            )
        })
        .await
    }
}

enum PromptAbortDispatchOutcome {
    Done,
    Retry,
}

pub(crate) struct SessionRuntimeCompatibilityContext<'a> {
    app: &'a mut DaemonApp,
    terminal_stream: Option<crate::terminal::TerminalStreamStore>,
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

    pub(crate) fn enqueue_prompt_dispatch(
        &mut self,
        dispatch: &crate::app::KernelPromptDispatch,
    ) -> Result<(), DaemonError> {
        self.app.enqueue_kernel_prompt_dispatch(dispatch)
    }

    pub(crate) fn fail_prompt_dispatch(
        &mut self,
        dispatch: crate::app::KernelPromptDispatch,
        error: DaemonError,
    ) -> Result<(), DaemonError> {
        self.app.fail_kernel_prompt_dispatch(dispatch, error)
    }

    pub(crate) fn finish_remote_prompt_dispatch(
        &mut self,
        dispatch: crate::app::KernelRemotePromptDispatch,
        result: Result<String, DaemonError>,
    ) -> Result<(), DaemonError> {
        self.app
            .finish_kernel_remote_prompt_dispatch(dispatch, result)
    }

    pub(crate) fn enqueue_prompt_abort(
        &mut self,
        dispatch: &crate::app::KernelPromptAbortDispatch,
    ) -> Result<(), DaemonError> {
        self.app.enqueue_kernel_prompt_abort(dispatch)
    }

    pub(crate) fn structured_prompt_io_in_flight(&self, provider_run_id: &str) -> bool {
        self.app.structured_prompt_io_in_flight(provider_run_id)
    }

    pub(crate) fn fail_prompt_abort(
        &mut self,
        dispatch: crate::app::KernelPromptAbortDispatch,
        error: DaemonError,
    ) -> Result<(), DaemonError> {
        self.app.fail_kernel_prompt_abort(dispatch, error)
    }
}

pub(crate) struct WorkflowRuntimeCompatibilityContext<'a> {
    app: &'a mut DaemonApp,
}

pub(crate) struct ProviderLaunchCompatibilityContext<'a> {
    app: &'a mut DaemonApp,
}

pub(crate) struct TerminalOutputCompatibilityContext<'a> {
    app: &'a mut DaemonApp,
}

pub(crate) struct CapabilityRuntimeCompatibilityContext<'a> {
    app: &'a DaemonApp,
    workspace_coordinator: Option<crate::kernel::workspace_coordinator::WorkspaceCoordinator>,
}

pub(crate) struct CapabilityRuntimeSnapshot {
    pub(crate) workspace_id: String,
    pub(crate) worktree_root: std::path::PathBuf,
    pub(crate) workspace_coordinator: crate::kernel::workspace_coordinator::WorkspaceCoordinator,
}

impl<'a> ProviderLaunchCompatibilityContext<'a> {
    fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn start_launch(
        &mut self,
        request: crate::local::LaunchProviderRunRequest,
    ) -> Result<(crate::app::StartedProviderLaunch, u64), DaemonError> {
        let launch_request =
            crate::local::provider_requests::launch_provider_request_from_local(self.app, request);
        Ok((
            self.app.start_provider_launch(launch_request)?,
            self.app.config().provider_runtime_init_delay_ms,
        ))
    }

    pub(crate) fn finish_launch(
        &mut self,
        started: &crate::app::StartedProviderLaunch,
        binding: Option<crate::provider::ProviderRuntimeBinding>,
    ) {
        if let Err(error) = self.app.finish_provider_launch(started, binding) {
            self.app.fail_provider_launch(started, &error);
        }
    }

    pub(crate) fn fail_launch(
        &mut self,
        started: &crate::app::StartedProviderLaunch,
        error: &DaemonError,
    ) {
        self.app.fail_provider_launch(started, error);
    }
}

impl<'a> CapabilityRuntimeCompatibilityContext<'a> {
    fn new(
        app: &'a DaemonApp,
        workspace_coordinator: Option<crate::kernel::workspace_coordinator::WorkspaceCoordinator>,
    ) -> Self {
        Self {
            app,
            workspace_coordinator,
        }
    }

    pub(crate) fn capability_context(
        &self,
        session_id: &str,
        attachment_id: &str,
        capability: &'static str,
    ) -> Result<CapabilityRuntimeSnapshot, DaemonError> {
        let context = crate::app::KernelSessionReadService::new(self.app).capability_context(
            session_id,
            attachment_id,
            capability,
        )?;
        Ok(CapabilityRuntimeSnapshot {
            workspace_id: context.workspace_id,
            worktree_root: context.worktree_root,
            workspace_coordinator: self
                .workspace_coordinator
                .clone()
                .unwrap_or_else(|| self.app.workspace_coordinator()),
        })
    }
}

impl<'a> TerminalOutputCompatibilityContext<'a> {
    fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn pump_terminal_output(
        &mut self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<Vec<crate::terminal::TerminalOutputRecord>, DaemonError> {
        crate::app::provider_output::pump_terminal_output_for_attachment(
            self.app,
            session_id,
            attachment_id,
        )
    }

    pub(crate) fn pump_active_provider_output(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
    ) -> Result<(), DaemonError> {
        let _ = crate::app::provider_output::ProviderOutputPump::new(self.app)
            .pump_provider_output(crate::app::provider_output::ProviderOutputPumpRequest {
                session_id,
                provider_run_id,
                recipient_attachment_ids,
            })?;
        Ok(())
    }

    pub(crate) fn session_snapshot(
        &self,
        session_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        crate::app::KernelSessionReadService::new(self.app).session_snapshot(session_id)
    }
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
    fn new(
        app: &'a mut DaemonApp,
        terminal_stream: Option<crate::terminal::TerminalStreamStore>,
    ) -> Self {
        Self {
            app,
            terminal_stream,
        }
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
        if let Some(terminal_stream) = &self.terminal_stream {
            return terminal_stream.drain_notice_records(session_id, attachment_id);
        }
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
