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
    config_projection: crate::kernel::projection::DaemonConfigProjectionStore,
    session_projection: crate::kernel::projection::SessionStateProjectionStore,
    provider_run_projection: crate::kernel::projection::ProviderRunProjectionStore,
    prompt_state_owner: crate::kernel::prompt_state::PromptStateOwner,
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
        config_projection: crate::kernel::projection::DaemonConfigProjectionStore,
        session_projection: crate::kernel::projection::SessionStateProjectionStore,
        provider_run_projection: crate::kernel::projection::ProviderRunProjectionStore,
        prompt_state_owner: crate::kernel::prompt_state::PromptStateOwner,
        terminal_stream: crate::terminal::TerminalStreamStore,
        workspace_coordinator: crate::kernel::workspace_coordinator::WorkspaceCoordinator,
    ) -> Self {
        Self {
            app,
            owned: Some(CompatibilityRuntimeOwnedState {
                config_projection,
                session_projection,
                provider_run_projection,
                prompt_state_owner,
                terminal_stream,
                workspace_coordinator,
            }),
        }
    }

    pub(crate) async fn config_snapshot(&self) -> crate::config::DaemonConfig {
        if let Some(owned) = &self.owned {
            return owned.config_projection.snapshot();
        }
        self.with_app_mut(|app| app.config().clone()).await
    }

    async fn with_app_mut<R>(&self, operation: impl FnOnce(&mut DaemonApp) -> R) -> R {
        let mut app = self.app.lock().await;
        operation(&mut app)
    }

    pub(crate) async fn active_prompt_agent_id(
        &self,
        session_id: &str,
    ) -> Result<Option<String>, DaemonError> {
        if let Some(owned) = &self.owned {
            if let Some(session) = owned.session_projection.get(session_id) {
                return Ok(owned.prompt_state_owner.active_prompt_agent_id(&session));
            }
        }
        self.with_app_mut(|app| app.prompt_owner_active_prompt_agent_id(session_id))
            .await
    }

    pub(crate) async fn focused_agent_id(
        &self,
        session_id: &str,
    ) -> Result<Option<String>, DaemonError> {
        if let Some(owned) = &self.owned {
            if let Some(session) = owned.session_projection.get(session_id) {
                return Ok(session.focused_agent_id().map(str::to_string));
            }
        }
        self.with_app_mut(|app| {
            Ok(app
                .sessions()
                .get_session(session_id)?
                .focused_agent_id()
                .map(str::to_string))
        })
        .await
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

    pub(crate) async fn with_agent_prompt_mut<R>(
        &self,
        operation: impl FnOnce(&mut AgentPromptCompatibilityContext<'_>) -> R,
    ) -> R {
        self.with_app_mut(|app| {
            let mut context = AgentPromptCompatibilityContext::new(
                app,
                self.owned
                    .as_ref()
                    .map(|owned| owned.provider_run_projection.clone()),
            );
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

    pub(crate) async fn execute_workflow_service_operation(
        &self,
        session_id: &str,
        operation: impl FnOnce(
            &mut crate::app::KernelWorkflowService<'_>,
        ) -> Result<LocalDaemonResponse, DaemonError>,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<crate::session::RuntimeSession>,
    ) {
        self.with_app_mut(|app| {
            let result = {
                let mut workflows = crate::app::KernelWorkflowService::new(app);
                operation(&mut workflows)
            };
            let projected_session = if let Ok(response) = result.as_ref() {
                workflow_response_session(response).or_else(|| {
                    crate::app::KernelSessionReadService::new(app)
                        .session_snapshot(session_id)
                        .ok()
                })
            } else {
                None
            };
            (result, projected_session)
        })
        .await
    }

    pub(crate) async fn start_provider_launch(
        &self,
        request: crate::local::LaunchProviderRunRequest,
    ) -> Result<(crate::app::StartedProviderLaunch, u64), DaemonError> {
        let launch_request = self.launch_provider_request_from_owned_state(request);
        self.with_app_mut(|app| {
            Ok((
                app.start_provider_launch(launch_request)?,
                app.config().provider_runtime_init_delay_ms,
            ))
        })
        .await
    }

    fn launch_provider_request_from_owned_state(
        &self,
        request: crate::local::LaunchProviderRunRequest,
    ) -> crate::provider::LaunchProviderRequest {
        let mut launch_request = crate::provider::LaunchProviderRequest::new(
            request.session_id.clone(),
            request.adapter_key,
            request.provider,
            request.account_profile,
            request.model,
        )
        .with_variant(request.variant);
        if let Some(agent_id) = request.agent_id.clone().or_else(|| {
            self.owned
                .as_ref()
                .and_then(|owned| owned.session_projection.get(&request.session_id))
                .and_then(|session| session.focused_agent_id().map(str::to_string))
        }) {
            launch_request = launch_request.with_agent_id(agent_id);
        }
        launch_request
    }

    pub(crate) async fn finish_provider_launch(
        &self,
        started: &crate::app::StartedProviderLaunch,
        binding: Option<crate::provider::ProviderRuntimeBinding>,
    ) {
        self.with_app_mut(|app| {
            if let Err(error) = app.finish_provider_launch(started, binding) {
                app.fail_provider_launch(started, &error);
            }
        })
        .await;
    }

    pub(crate) async fn fail_provider_launch(
        &self,
        started: &crate::app::StartedProviderLaunch,
        error: &DaemonError,
    ) {
        self.with_app_mut(|app| app.fail_provider_launch(started, error))
            .await;
    }

    pub(crate) async fn pump_terminal_output_with_snapshot(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<
        (
            Vec<crate::terminal::TerminalOutputRecord>,
            Option<crate::session::RuntimeSession>,
        ),
        DaemonError,
    > {
        self.with_app_mut(|app| {
            let records = crate::app::provider_output::pump_terminal_output_for_attachment(
                app,
                session_id,
                attachment_id,
            )?;
            let session = crate::app::KernelSessionReadService::new(app)
                .session_snapshot(session_id)
                .ok();
            Ok((records, session))
        })
        .await
    }

    pub(crate) async fn pump_active_provider_output_with_snapshot(
        &self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
    ) -> Result<Option<crate::session::RuntimeSession>, DaemonError> {
        self.with_app_mut(|app| {
            let _ = crate::app::provider_output::ProviderOutputPump::new(app)
                .pump_provider_output(crate::app::provider_output::ProviderOutputPumpRequest {
                    session_id,
                    provider_run_id,
                    recipient_attachment_ids,
                })?;
            Ok(crate::app::KernelSessionReadService::new(app)
                .session_snapshot(session_id)
                .ok())
        })
        .await
    }

    pub(crate) async fn capability_context(
        &self,
        session_id: &str,
        attachment_id: &str,
        capability: &'static str,
    ) -> Result<CapabilityRuntimeSnapshot, DaemonError> {
        let app = self.app.lock().await;
        let context = crate::app::KernelSessionReadService::new(&app).capability_context(
            session_id,
            attachment_id,
            capability,
        )?;
        Ok(CapabilityRuntimeSnapshot {
            workspace_id: context.workspace_id,
            worktree_root: context.worktree_root,
            workspace_coordinator: self
                .owned
                .as_ref()
                .map(|owned| owned.workspace_coordinator.clone())
                .unwrap_or_else(|| app.workspace_coordinator()),
        })
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

pub(crate) struct AgentPromptCompatibilityContext<'a> {
    app: &'a mut DaemonApp,
    provider_run_projection: Option<crate::kernel::projection::ProviderRunProjectionStore>,
}

impl<'a> AgentPromptCompatibilityContext<'a> {
    fn new(
        app: &'a mut DaemonApp,
        provider_run_projection: Option<crate::kernel::projection::ProviderRunProjectionStore>,
    ) -> Self {
        Self {
            app,
            provider_run_projection,
        }
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
            .provider_run_projection
            .as_ref()
            .and_then(|projection| projection.get_for_agent(session_id, target_agent_id))
            .map(|run| run.id().to_string())
            .or_else(|| {
                self.app
                    .providers()
                    .get_run_for_agent(session_id, target_agent_id)
                    .map(|run| run.id().to_string())
            });
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

pub(crate) struct CapabilityRuntimeSnapshot {
    pub(crate) workspace_id: String,
    pub(crate) worktree_root: std::path::PathBuf,
    pub(crate) workspace_coordinator: crate::kernel::workspace_coordinator::WorkspaceCoordinator,
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
