use std::sync::Arc;

use tokio::sync::Mutex;

use crate::agent::AgentServiceStore;
use crate::app::DaemonApp;
use crate::attachment::AttachmentServiceStore;
use crate::error::DaemonError;
use crate::local::LocalDaemonResponse;
use crate::provider::{ProviderProcessServiceStore, ProviderRunOperationLanes};
use crate::session::SessionStateStore;
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
    session_store: SessionStateStore,
    agent_store: AgentServiceStore,
    attachment_store: AttachmentServiceStore,
    provider_store: ProviderProcessServiceStore,
    session_projection: crate::kernel::projection::SessionStateProjectionStore,
    provider_run_projection: crate::kernel::projection::ProviderRunProjectionStore,
    prompt_state_owner: crate::kernel::prompt_state::PromptStateOwner,
    terminal_stream: crate::terminal::TerminalStreamStore,
    workspace_coordinator: crate::kernel::workspace_coordinator::WorkspaceCoordinator,
}

impl CompatibilityRuntimeOwnedState {
    fn session_snapshot(
        &self,
        session_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        let mut session = self.session_store.get_session(session_id)?;
        let agents = self.agent_store.get_session_agents(session_id);
        session.set_agents(agents);
        self.project_session_runtime_view(&mut session);
        self.session_projection.update(session.clone());
        Ok(session)
    }

    fn project_session_runtime_view(&self, session: &mut crate::session::RuntimeSession) {
        if let Some(active_provider_run_id) = session.active_provider_run_id() {
            if let Ok(active_run) = self.provider_store.get_run(active_provider_run_id) {
                let active_run_agent_id = active_run.agent_instance_id();
                let active_prompt_is_running = active_run_agent_id
                    .and_then(|agent_id| {
                        self.prompt_state_owner
                            .active_prompt_for_agent_snapshot(session, agent_id)
                    })
                    .is_some();
                if active_run.state() == crate::provider::ProviderRunState::Running
                    && active_prompt_is_running
                {
                    return;
                }
            }
        }

        let projected_run_id = session.focused_agent_id().and_then(|agent_id| {
            self.provider_store
                .get_run_for_agent(session.id(), agent_id)
                .map(|run| run.id().to_string())
        });
        session.set_active_provider_run(projected_run_id);
    }

    fn ensure_attachment_in_session(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<crate::attachment::RuntimeAttachment, DaemonError> {
        let attachment = self.attachment_store.get_attachment(attachment_id)?;
        if attachment.session_id() != session_id {
            return Err(DaemonError::AttachmentNotInSession {
                session_id: session_id.to_string(),
                attachment_id: attachment_id.to_string(),
            });
        }
        Ok(attachment)
    }

    fn capability_context(
        &self,
        session_id: &str,
        attachment_id: &str,
        capability: &'static str,
    ) -> Result<CapabilityRuntimeSnapshot, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let attachment = self.ensure_attachment_in_session(session_id, attachment_id)?;
        if !matches!(
            attachment.capability_level(),
            crate::attachment::ClientCapabilityLevel::FullTerminal
                | crate::attachment::ClientCapabilityLevel::InteractiveStructured
        ) {
            return Err(DaemonError::AttachmentCapabilityDenied {
                session_id: session_id.to_string(),
                attachment_id: attachment.id().to_string(),
                capability,
            });
        }
        Ok(CapabilityRuntimeSnapshot {
            workspace_id: session.workspace_id().to_string(),
            worktree_root: std::path::PathBuf::from(session.worktree_id()),
            workspace_coordinator: self.workspace_coordinator.clone(),
        })
    }
}

impl CompatibilityRuntimeState {
    #[cfg(test)]
    pub(crate) fn new(app: Arc<Mutex<DaemonApp>>) -> Self {
        Self { app, owned: None }
    }

    pub(crate) fn new_with_owned_state(
        app: Arc<Mutex<DaemonApp>>,
        config_projection: crate::kernel::projection::DaemonConfigProjectionStore,
        session_store: SessionStateStore,
        agent_store: AgentServiceStore,
        attachment_store: AttachmentServiceStore,
        provider_store: ProviderProcessServiceStore,
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
                session_store,
                agent_store,
                attachment_store,
                provider_store,
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
            let session = owned.session_store.get_session(session_id)?;
            return Ok(owned.prompt_state_owner.active_prompt_agent_id(&session));
        }
        self.with_app_mut(|app| app.prompt_owner_active_prompt_agent_id(session_id))
            .await
    }

    pub(crate) async fn focused_agent_id(
        &self,
        session_id: &str,
    ) -> Result<Option<String>, DaemonError> {
        if let Some(owned) = &self.owned {
            let session = owned.session_store.get_session(session_id)?;
            return Ok(session.focused_agent_id().map(str::to_string));
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

    pub(crate) async fn resolve_session_ref_id(
        &self,
        session_ref: &str,
        workspace_id: Option<&str>,
    ) -> Result<String, DaemonError> {
        if let Some(owned) = &self.owned {
            return Ok(owned
                .session_store
                .read()
                .resolve_session_ref(session_ref, workspace_id)?
                .id()
                .to_string());
        }
        self.with_app_mut(|app| {
            crate::app::KernelSessionService::new(app)
                .resolve_session_ref_id(session_ref, workspace_id)
        })
        .await
    }

    pub(crate) async fn attachment_session_id(
        &self,
        attachment_id: &str,
    ) -> Result<String, DaemonError> {
        if let Some(owned) = &self.owned {
            return Ok(owned
                .attachment_store
                .get_attachment(attachment_id)?
                .session_id()
                .to_string());
        }
        self.with_app_mut(|app| {
            crate::app::KernelSessionService::new(app).attachment_session_id(attachment_id)
        })
        .await
    }

    pub(crate) async fn session_snapshot(
        &self,
        session_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        if let Some(owned) = &self.owned {
            return owned.session_snapshot(session_id);
        }
        self.with_app_mut(|app| {
            crate::app::KernelSessionService::new(app).session_snapshot(session_id)
        })
        .await
    }

    pub(crate) async fn create_session_response(
        &self,
        request: crate::session::CreateSessionRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.with_app_mut(|app| {
            crate::app::KernelSessionService::new(app).create_session_response(request)
        })
        .await
    }

    pub(crate) async fn attach(
        &self,
        request: crate::attachment::AttachRequest,
    ) -> Result<crate::attachment::RuntimeAttachment, DaemonError> {
        self.with_app_mut(|app| crate::app::KernelSessionService::new(app).attach(request))
            .await
    }

    pub(crate) async fn detach(
        &self,
        attachment_id: &str,
    ) -> Result<crate::attachment::RuntimeAttachment, DaemonError> {
        self.with_app_mut(|app| crate::app::KernelSessionService::new(app).detach(attachment_id))
            .await
    }

    pub(crate) async fn focus_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.with_app_mut(|app| {
            crate::app::KernelSessionService::new(app).focus_agent(session_id, agent_id)
        })
        .await
    }

    pub(crate) async fn cycle_agent_focus(
        &self,
        session_id: &str,
    ) -> Result<Option<crate::agent::AgentInstance>, DaemonError> {
        self.with_app_mut(|app| {
            crate::app::KernelSessionService::new(app).cycle_agent_focus(session_id)
        })
        .await
    }

    pub(crate) async fn resize_terminal(
        &self,
        session_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), DaemonError> {
        self.with_app_mut(|app| {
            crate::app::KernelSessionService::new(app).resize_terminal(session_id, cols, rows)
        })
        .await
    }

    pub(crate) async fn ensure_attachment_in_session(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<(), DaemonError> {
        if let Some(owned) = &self.owned {
            let _ = owned.ensure_attachment_in_session(session_id, attachment_id)?;
            return Ok(());
        }
        self.with_app_mut(|app| {
            let _ = crate::app::KernelSessionService::new(app)
                .ensure_attachment_in_session(session_id, attachment_id)?;
            Ok(())
        })
        .await
    }

    pub(crate) async fn drain_notice_records(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Vec<crate::terminal::RuntimeNoticeRecord> {
        if let Some(owned) = &self.owned {
            return owned
                .terminal_stream
                .drain_notice_records(session_id, attachment_id);
        }
        self.with_app_mut(|app| {
            app.terminal()
                .drain_notice_records(session_id, attachment_id)
        })
        .await
    }

    pub(crate) async fn update_session_config(
        &self,
        session_id: &str,
        attachment_id: &str,
        values: std::collections::BTreeMap<String, String>,
        requires_idle: bool,
    ) -> Result<crate::session::SessionConfigState, DaemonError> {
        self.with_app_mut(|app| {
            crate::app::KernelSessionService::new(app).update_session_config(
                session_id,
                attachment_id,
                values,
                requires_idle,
            )
        })
        .await
    }

    pub(crate) async fn alias_session(
        &self,
        session_id: &str,
        alias: String,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        self.with_app_mut(|app| {
            crate::app::KernelSessionService::new(app).alias_session(session_id, alias)
        })
        .await
    }

    pub(crate) async fn spawn_agent(
        &self,
        request: crate::agent::CreateAgentRequest,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.with_app_mut(|app| crate::app::KernelSessionService::new(app).spawn_agent(request))
            .await
    }

    pub(crate) async fn destroy_agent(
        &self,
        agent_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.with_app_mut(|app| crate::app::KernelSessionService::new(app).destroy_agent(agent_id))
            .await
    }

    pub(crate) async fn end_session(
        &self,
        session_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        self.with_app_mut(|app| crate::app::KernelSessionService::new(app).end_session(session_id))
            .await
    }

    pub(crate) async fn delete_session_ref(
        &self,
        session_ref: &str,
        workspace_id: Option<&str>,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        self.with_app_mut(|app| {
            crate::app::KernelSessionService::new(app).delete_session_ref(session_ref, workspace_id)
        })
        .await
    }

    pub(crate) async fn submit_prepared_prompt(
        &self,
        prepared: crate::app::KernelPreparedPromptSubmission,
    ) -> Result<crate::app::KernelPromptSubmission, DaemonError> {
        self.with_app_mut(|app| {
            crate::app::KernelAgentService::new(app).submit_prepared_prompt_for_kernel(prepared)
        })
        .await
    }

    pub(crate) async fn cancel_agent_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
    ) -> Result<crate::app::KernelPromptCancellation, DaemonError> {
        self.with_app_mut(|app| {
            crate::app::KernelAgentService::new(app).cancel_agent_prompt_for_kernel(
                session_id,
                target_agent_id,
                attachment_id,
            )
        })
        .await
    }

    pub(crate) async fn complete_agent_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        next_queued_prompt: Option<&crate::session::PromptQueueItem>,
    ) -> Result<crate::session::PromptCompletion, DaemonError> {
        let projected_provider_run_id = self
            .owned
            .as_ref()
            .and_then(|owned| {
                owned
                    .provider_run_projection
                    .get_for_agent(session_id, target_agent_id)
            })
            .map(|run| run.id().to_string());
        self.with_app_mut(|app| {
            let provider_run_id = projected_provider_run_id.or_else(|| {
                app.providers()
                    .get_run_for_agent(session_id, target_agent_id)
                    .map(|run| run.id().to_string())
            });
            crate::app::KernelAgentService::new(app).complete_active_prompt_for_kernel(
                session_id,
                target_agent_id,
                provider_run_id.as_deref(),
                next_queued_prompt,
            )
        })
        .await
    }

    async fn enqueue_prompt_dispatch(
        &self,
        dispatch: &crate::app::KernelPromptDispatch,
    ) -> Result<(), DaemonError> {
        self.with_app_mut(|app| app.enqueue_kernel_prompt_dispatch(dispatch))
            .await
    }

    async fn fail_prompt_dispatch(
        &self,
        dispatch: crate::app::KernelPromptDispatch,
        error: DaemonError,
    ) -> Result<(), DaemonError> {
        self.with_app_mut(|app| app.fail_kernel_prompt_dispatch(dispatch, error))
            .await
    }

    async fn finish_remote_prompt_dispatch(
        &self,
        dispatch: crate::app::KernelRemotePromptDispatch,
        result: Result<String, DaemonError>,
    ) -> Result<(), DaemonError> {
        self.with_app_mut(|app| app.finish_kernel_remote_prompt_dispatch(dispatch, result))
            .await
    }

    async fn enqueue_prompt_abort(
        &self,
        dispatch: &crate::app::KernelPromptAbortDispatch,
    ) -> Result<(), DaemonError> {
        self.with_app_mut(|app| app.enqueue_kernel_prompt_abort(dispatch))
            .await
    }

    async fn structured_prompt_io_in_flight(&self, provider_run_id: &str) -> bool {
        if let Some(owned) = &self.owned {
            return owned
                .provider_store
                .structured_prompt_io_in_flight(provider_run_id);
        }
        self.with_app_mut(|app| app.structured_prompt_io_in_flight(provider_run_id))
            .await
    }

    async fn fail_prompt_abort(
        &self,
        dispatch: crate::app::KernelPromptAbortDispatch,
        error: DaemonError,
    ) -> Result<(), DaemonError> {
        self.with_app_mut(|app| app.fail_kernel_prompt_abort(dispatch, error))
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
            if let Err(error) = state.enqueue_prompt_dispatch(&dispatch).await {
                let _ = state.fail_prompt_dispatch(dispatch, error).await;
            }
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
            let _ = state.finish_remote_prompt_dispatch(dispatch, result).await;
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
                let outcome = match state.enqueue_prompt_abort(&dispatch).await {
                    Ok(()) => PromptAbortDispatchOutcome::Done,
                    Err(_)
                        if state
                            .structured_prompt_io_in_flight(&dispatch.provider_run_id)
                            .await =>
                    {
                        PromptAbortDispatchOutcome::Retry
                    }
                    Err(error) => {
                        let _ = state.fail_prompt_abort(dispatch.clone(), error).await;
                        PromptAbortDispatchOutcome::Done
                    }
                };
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
        let (result, response_session) = self
            .with_app_mut(|app| {
                let result = {
                    let mut workflows = crate::app::KernelWorkflowService::new(app);
                    operation(&mut workflows)
                };
                let response_session = result.as_ref().ok().and_then(workflow_response_session);
                (result, response_session)
            })
            .await;
        let projected_session = if response_session.is_some() || result.is_err() {
            response_session
        } else if let Some(owned) = &self.owned {
            owned.session_snapshot(session_id).ok()
        } else {
            self.with_app_mut(|app| {
                crate::app::KernelSessionReadService::new(app)
                    .session_snapshot(session_id)
                    .ok()
            })
            .await
        };
        (result, projected_session)
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
            self.owned.as_ref().and_then(|owned| {
                owned
                    .session_store
                    .get_session(&request.session_id)
                    .ok()
                    .and_then(|session| session.focused_agent_id().map(str::to_string))
                    .or_else(|| {
                        owned
                            .agent_store
                            .get_focused_agent(&request.session_id)
                            .map(|agent| agent.id().to_string())
                    })
            })
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
        let records = self
            .with_app_mut(|app| {
                crate::app::provider_output::pump_terminal_output_for_attachment(
                    app,
                    session_id,
                    attachment_id,
                )
            })
            .await?;
        let session = if let Some(owned) = &self.owned {
            owned.session_snapshot(session_id).ok()
        } else {
            self.with_app_mut(|app| {
                crate::app::KernelSessionReadService::new(app)
                    .session_snapshot(session_id)
                    .ok()
            })
            .await
        };
        Ok((records, session))
    }

    pub(crate) async fn pump_active_provider_output_with_snapshot(
        &self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
    ) -> Result<Option<crate::session::RuntimeSession>, DaemonError> {
        self.with_app_mut(|app| {
            crate::app::provider_output::ProviderOutputPump::new(app).pump_provider_output(
                crate::app::provider_output::ProviderOutputPumpRequest {
                    session_id,
                    provider_run_id,
                    recipient_attachment_ids,
                },
            )
        })
        .await?;
        let session = if let Some(owned) = &self.owned {
            owned.session_snapshot(session_id).ok()
        } else {
            self.with_app_mut(|app| {
                crate::app::KernelSessionReadService::new(app)
                    .session_snapshot(session_id)
                    .ok()
            })
            .await
        };
        Ok(session)
    }

    pub(crate) async fn capability_context(
        &self,
        session_id: &str,
        attachment_id: &str,
        capability: &'static str,
    ) -> Result<CapabilityRuntimeSnapshot, DaemonError> {
        if let Some(owned) = &self.owned {
            return owned.capability_context(session_id, attachment_id, capability);
        }
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
