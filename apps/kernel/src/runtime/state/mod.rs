use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use tokio::sync::Mutex;

use crate::agent::AgentServiceStore;
use crate::app::{
    DaemonApp, PromptActivityStore, PromptWorkspaceClaimStore, ProviderProcessTrackingStore,
};
use crate::attachment::AttachmentServiceStore;
use crate::error::DaemonError;
use crate::history::{SessionHistoryEntry, SessionHistoryStore};
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::provider::{ProviderProcessServiceStore, ProviderRunOperationLanes};
use crate::session::{SessionStateOwner, SessionStateStore};
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use arroba_relay::protocol::ClientTarget;

mod managed_io;
use managed_io::*;

#[derive(Clone)]
pub(crate) struct KernelRuntimeState {
    app: Arc<Mutex<DaemonApp>>,
    owned: KernelRuntimeOwnedState,
}

#[derive(Clone)]
struct KernelRuntimeOwnedState {
    config_projection: crate::runtime::projection::DaemonConfigProjectionStore,
    session_store: SessionStateStore,
    agent_store: AgentServiceStore,
    attachment_store: AttachmentServiceStore,
    provider_store: ProviderProcessServiceStore,
    provider_process_tracking: ProviderProcessTrackingStore,
    session_projection: crate::runtime::projection::SessionStateProjectionStore,
    provider_run_projection: crate::runtime::projection::ProviderRunProjectionStore,
    history_store: SessionHistoryStore,
    history_projection: crate::runtime::projection::SessionHistoryProjectionStore,
    prompt_state_owner: crate::runtime::prompt_state::PromptStateOwner,
    prompt_activity: PromptActivityStore,
    prompt_idle_timeout: Duration,
    prompt_workspace_claims: PromptWorkspaceClaimStore,
    structured_output_records: crate::app::provider_output::StructuredOutputRecordStore,
    terminal_stream: crate::terminal::TerminalStreamStore,
    workspace_coordinator: crate::runtime::workspace_coordinator::WorkspaceCoordinator,
    managed_io_coordinator: Arc<Mutex<crate::io::ArtifactEditCoordinator>>,
    managed_io_external_changes: crate::io::ArtifactExternalChangeMonitor,
    workspace_identity_monitor:
        crate::runtime::workspace_identity_monitor::WorkspaceIdentityMonitor,
}

#[derive(Default)]
struct WorkflowPromptDispatches {
    local: Vec<crate::app::KernelPromptDispatch>,
    remote: Vec<crate::app::KernelRemotePromptDispatch>,
}

impl WorkflowPromptDispatches {
    fn extend(&mut self, other: Self) {
        self.local.extend(other.local);
        self.remote.extend(other.remote);
    }
}

struct ManagedIoWorkspaceContext {
    root: PathBuf,
    identity: crate::io::WorkspaceIdentity,
    generation: u64,
    identity_changed: bool,
    valid: bool,
}

mod owned;

impl KernelRuntimeState {
    pub(crate) fn new_with_owned_state(
        app: Arc<Mutex<DaemonApp>>,
        config_projection: crate::runtime::projection::DaemonConfigProjectionStore,
        session_store: SessionStateStore,
        agent_store: AgentServiceStore,
        attachment_store: AttachmentServiceStore,
        provider_store: ProviderProcessServiceStore,
        provider_process_tracking: ProviderProcessTrackingStore,
        session_projection: crate::runtime::projection::SessionStateProjectionStore,
        provider_run_projection: crate::runtime::projection::ProviderRunProjectionStore,
        history_store: SessionHistoryStore,
        history_projection: crate::runtime::projection::SessionHistoryProjectionStore,
        prompt_state_owner: crate::runtime::prompt_state::PromptStateOwner,
        prompt_activity: PromptActivityStore,
        prompt_idle_timeout: Duration,
        prompt_workspace_claims: PromptWorkspaceClaimStore,
        structured_output_records: crate::app::provider_output::StructuredOutputRecordStore,
        terminal_stream: crate::terminal::TerminalStreamStore,
        workspace_coordinator: crate::runtime::workspace_coordinator::WorkspaceCoordinator,
    ) -> Self {
        Self {
            app,
            owned: KernelRuntimeOwnedState {
                config_projection,
                session_store,
                agent_store,
                attachment_store,
                provider_store,
                provider_process_tracking,
                session_projection,
                provider_run_projection,
                history_store,
                history_projection,
                prompt_state_owner,
                prompt_activity,
                prompt_idle_timeout,
                prompt_workspace_claims,
                structured_output_records,
                terminal_stream,
                workspace_coordinator,
                managed_io_coordinator: Arc::new(Mutex::new(
                    crate::io::ArtifactEditCoordinator::new(),
                )),
                managed_io_external_changes: crate::io::ArtifactExternalChangeMonitor::default(),
                workspace_identity_monitor:
                    crate::runtime::workspace_identity_monitor::WorkspaceIdentityMonitor::default(),
            },
        }
    }

    pub(crate) async fn config_snapshot(&self) -> crate::config::DaemonConfig {
        self.owned.config_projection.snapshot()
    }

    pub(crate) async fn managed_io_health_snapshot(
        &self,
    ) -> crate::runtime::projection::ManagedIoHealthSnapshot {
        let reservations = self
            .owned
            .managed_io_coordinator
            .lock()
            .await
            .active_reservation_snapshots();
        let active_reservation_artifacts = reservations
            .iter()
            .map(|reservation| reservation.artifact_id.clone())
            .collect::<BTreeSet<_>>()
            .len();
        crate::runtime::projection::ManagedIoHealthSnapshot {
            active_reservations: reservations.len(),
            active_reservation_artifacts,
            workspace_identity: self.owned.workspace_identity_monitor.health_snapshot(),
            external_changes: self.owned.managed_io_external_changes.health_snapshot(),
        }
    }

    async fn with_app_side_effect<R>(&self, operation: impl FnOnce(&mut DaemonApp) -> R) -> R {
        let mut app = self.app.lock().await;
        operation(&mut app)
    }

    pub(crate) async fn active_prompt_agent_id(
        &self,
        session_id: &str,
    ) -> Result<Option<String>, DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        Ok(self
            .owned
            .prompt_state_owner
            .active_prompt_agent_id(&session))
    }

    pub(crate) async fn focused_agent_id(
        &self,
        session_id: &str,
    ) -> Result<Option<String>, DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        Ok(session.focused_agent_id().map(str::to_string))
    }

    pub(crate) async fn resolve_session_ref_id(
        &self,
        session_ref: &str,
        workspace_id: Option<&str>,
    ) -> Result<String, DaemonError> {
        Ok(self
            .owned
            .session_store
            .read()
            .resolve_session_ref(session_ref, workspace_id)?
            .id()
            .to_string())
    }

    pub(crate) async fn attachment_session_id(
        &self,
        attachment_id: &str,
    ) -> Result<String, DaemonError> {
        Ok(self
            .owned
            .attachment_store
            .get_attachment(attachment_id)?
            .session_id()
            .to_string())
    }

    pub(crate) async fn session_snapshot(
        &self,
        session_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        self.owned.session_snapshot(session_id)
    }

    pub(crate) async fn create_session_response(
        &self,
        request: crate::session::CreateSessionRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.owned.create_session_response(request)
    }

    pub(crate) async fn attach(
        &self,
        request: crate::attachment::AttachRequest,
    ) -> Result<crate::attachment::RuntimeAttachment, DaemonError> {
        self.owned.attach(request)
    }

    pub(crate) async fn detach(
        &self,
        attachment_id: &str,
    ) -> Result<crate::attachment::RuntimeAttachment, DaemonError> {
        self.owned.detach(attachment_id)
    }

    pub(crate) async fn focus_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.owned.focus_agent(session_id, agent_id)
    }

    pub(crate) async fn cycle_agent_focus(
        &self,
        session_id: &str,
    ) -> Result<Option<crate::agent::AgentInstance>, DaemonError> {
        self.owned.cycle_agent_focus(session_id)
    }

    pub(crate) async fn resize_terminal(
        &self,
        session_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), DaemonError> {
        if let Some(provider_run_id) = self.owned.resize_terminal(session_id)? {
            self.with_app_side_effect(|app| app.pty_mut().resize(&provider_run_id, cols, rows))
                .await?;
        }
        Ok(())
    }

    pub(crate) async fn ensure_attachment_in_session(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<(), DaemonError> {
        let _ = self
            .owned
            .ensure_attachment_in_session(session_id, attachment_id)?;
        Ok(())
    }

    pub(crate) async fn drain_notice_records(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Vec<crate::terminal::RuntimeNoticeRecord> {
        self.owned
            .terminal_stream
            .drain_notice_records(session_id, attachment_id)
    }

    pub(crate) async fn update_session_config(
        &self,
        session_id: &str,
        attachment_id: &str,
        values: std::collections::BTreeMap<String, String>,
        requires_idle: bool,
    ) -> Result<crate::session::SessionConfigState, DaemonError> {
        self.owned
            .update_session_config(session_id, attachment_id, values, requires_idle)
    }

    pub(crate) async fn alias_session(
        &self,
        session_id: &str,
        alias: String,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        self.owned.alias_session(session_id, alias)
    }

    pub(crate) async fn spawn_agent(
        &self,
        request: crate::agent::CreateAgentRequest,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        if request.machine_ref.is_none() {
            return self.owned.spawn_agent(request);
        }
        self.with_app_side_effect(|app| {
            crate::app::KernelSessionService::new(app).spawn_agent(request)
        })
        .await
    }

    pub(crate) async fn destroy_agent(
        &self,
        agent_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if agent.remote_execution().is_none() {
            return self.owned.destroy_agent(agent_id);
        }
        self.with_app_side_effect(|app| {
            crate::app::KernelSessionService::new(app).destroy_agent(agent_id)
        })
        .await
    }

    pub(crate) async fn end_session(
        &self,
        session_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        let owned = &self.owned;
        let (session, terminated_run_ids) = owned.end_session(session_id)?;
        for provider_run_id in terminated_run_ids {
            let (_, process_key) = self
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(&provider_run_id)
                })
                .await
                .unwrap_or((false, None));
            owned.remove_provider_process_tracking_for_run(&provider_run_id, process_key);
        }
        Ok(session)
    }

    pub(crate) async fn delete_session_ref(
        &self,
        session_ref: &str,
        workspace_id: Option<&str>,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        let owned = &self.owned;
        let (session, terminated_run_ids) = owned.delete_session_ref(session_ref, workspace_id)?;
        for provider_run_id in terminated_run_ids {
            let (_, process_key) = self
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(&provider_run_id)
                })
                .await
                .unwrap_or((false, None));
            owned.remove_provider_process_tracking_for_run(&provider_run_id, process_key);
        }
        Ok(session)
    }

    pub(crate) async fn submit_prepared_prompt(
        &self,
        prepared: crate::app::KernelPreparedPromptSubmission,
    ) -> Result<crate::app::KernelPromptSubmission, DaemonError> {
        {
            let owned = &self.owned;
            if let Some(mut submission) = owned.submit_local_prepared_prompt(&prepared)? {
                self.finish_owned_prompt_submission_workflow_start(&mut submission)
                    .await?;
                return Ok(submission);
            }
            if let Some(mut submission) = owned.submit_remote_prepared_prompt(&prepared)? {
                self.finish_owned_prompt_submission_workflow_start(&mut submission)
                    .await?;
                return Ok(submission);
            }
            let session_id = prepared.session_id.clone();
            let target_agent_id = prepared.prompt.target_agent_id().to_string();
            let attachment_id = prepared.prompt.source_attachment_id().to_string();
            let has_active = owned
                .prompt_state_owner
                .active_prompt_for_agent(
                    &owned.session_store.get_session(&session_id)?,
                    &target_agent_id,
                )
                .is_some();
            let has_run = owned
                .provider_store
                .get_run_for_agent(&session_id, &target_agent_id)
                .is_some();
            if !has_active && !has_run {
                if crate::scheduler::runtime::is_workflow_prompt_attachment(&attachment_id) {
                    owned.workflow_ensure_provider_run(&session_id, &target_agent_id)?;
                } else {
                    self.with_app_side_effect(|app| {
                        app.ensure_prompt_provider_run_for_agent(&session_id, &target_agent_id)
                    })
                    .await?;
                };
                if let Some(mut submission) = owned.submit_local_prepared_prompt(&prepared)? {
                    self.finish_owned_prompt_submission_workflow_start(&mut submission)
                        .await?;
                    return Ok(submission);
                }
            }
            Err(DaemonError::LocalTransport {
                operation: "submit prepared prompt",
                message:
                    "owned prompt runtime could not admit prompt without side-effect completion"
                        .to_string(),
            })
        }
    }

    async fn finish_owned_prompt_submission_workflow_start(
        &self,
        submission: &mut crate::app::KernelPromptSubmission,
    ) -> Result<(), DaemonError> {
        let crate::session::PromptSubmissionOutcome::Started { prompt } = &submission.outcome
        else {
            return Ok(());
        };
        if !crate::scheduler::runtime::is_workflow_prompt_attachment(prompt.source_attachment_id())
        {
            return Ok(());
        }
        let session_id = submission.session.id().to_string();
        let prompt = prompt.clone();
        if let Some(remote_dispatch) = submission.remote_dispatch.as_mut() {
            remote_dispatch.workflow_context = Some(
                self.with_app_side_effect(|app| {
                    crate::app::RemoteWorkflowTurnContextResolver::new(app)
                        .remote_workflow_turn_context_for_prompt(
                            &session_id,
                            prompt.target_agent_id(),
                            &prompt,
                        )
                })
                .await?,
            );
        }
        self.owned.workflow_start_prompt(&session_id, &prompt)
    }

    pub(crate) async fn cancel_agent_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
    ) -> Result<crate::app::KernelPromptCancellation, DaemonError> {
        {
            let owned = &self.owned;
            if owned
                .agent_store
                .get_agent(target_agent_id)?
                .remote_execution()
                .is_some()
            {
                let remote_execution = owned
                    .agent_store
                    .get_agent(target_agent_id)?
                    .remote_execution()
                    .cloned()
                    .expect("remote execution checked above");
                match self
                    .with_app_side_effect(|app| {
                        app.block_on_relay_future(
                            crate::transport::relay_client::send_peer_request_via_temporary_connection(
                                app.config(),
                                ClientTarget {
                                    daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                                    daemon_alias: None,
                                },
                                RelayPeerRequest::CancelLeasedPrompt {
                                    leased_agent_id: remote_execution.leased_agent_id.clone(),
                                },
                            ),
                        )
                    })
                    .await?
                {
                    RelayPeerResponse::LeasedPromptCancelled { .. } => {
                        return owned.begin_remote_prompt_cancellation(
                            session_id,
                            target_agent_id,
                            attachment_id,
                        );
                    }
                    other => {
                        return Err(DaemonError::LocalTransport {
                            operation: "cancel remote prompt",
                            message: format!(
                                "unexpected remote prompt cancellation response: {other:?}"
                            ),
                        });
                    }
                }
            }
            if let Some(cancellation) =
                owned.cancel_local_prompt(session_id, target_agent_id, attachment_id)?
            {
                return Ok(cancellation);
            }
            Err(DaemonError::LocalTransport {
                operation: "cancel prompt",
                message:
                    "owned prompt runtime could not cancel prompt without side-effect completion"
                        .to_string(),
            })
        }
    }

    pub(crate) async fn complete_agent_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        next_queued_prompt: Option<&crate::session::PromptQueueItem>,
    ) -> Result<crate::session::PromptCompletion, DaemonError> {
        let owned = &self.owned;
        let owned_provider_run_id = owned
            .provider_run_projection
            .get_for_agent(session_id, target_agent_id)
            .or_else(|| {
                owned
                    .provider_store
                    .get_run_for_agent(session_id, target_agent_id)
            })
            .map(|run| run.id().to_string());
        {
            if let Some(remote_execution) = owned
                .agent_store
                .get_agent(target_agent_id)?
                .remote_execution()
                .cloned()
            {
                let remote_provider_run_id = match self
                    .with_app_side_effect(|app| {
                        app.block_on_relay_future(
                            crate::transport::relay_client::send_peer_request_via_temporary_connection(
                                app.config(),
                                ClientTarget {
                                    daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                                    daemon_alias: None,
                                },
                                RelayPeerRequest::CompleteLeasedPrompt {
                                    leased_agent_id: remote_execution.leased_agent_id.clone(),
                                },
                            ),
                        )
                    })
                    .await?
                {
                    RelayPeerResponse::LeasedPromptCompleted {
                        provider_run_id, ..
                    } => provider_run_id
                        .unwrap_or_else(|| "remote-provider-run-completed".to_string()),
                    other => {
                        return Err(DaemonError::LocalTransport {
                            operation: "complete remote prompt",
                            message: format!(
                                "unexpected remote prompt completion response: {other:?}"
                            ),
                        });
                    }
                };
                let completion = owned.complete_remote_prompt_owner(
                    session_id,
                    target_agent_id,
                    &remote_provider_run_id,
                    next_queued_prompt,
                )?;
                if completion.completed.workflow_run_id().is_some() {
                    let dispatches = owned.workflow_complete_prompt(
                        session_id,
                        &completion.completed,
                        Some(&remote_provider_run_id),
                    )?;
                    self.spawn_workflow_prompt_dispatches(dispatches);
                }
                if let Some(started_next) = completion.started_next.as_ref() {
                    let attachments = self
                        .with_app_side_effect(|app| {
                            app.serialize_remote_prompt_attachments(started_next.attachments())
                        })
                        .await?;
                    let workflow_context =
                        if crate::scheduler::runtime::is_workflow_prompt_attachment(
                            started_next.source_attachment_id(),
                        ) {
                            Some(
                                self.with_app_side_effect(|app| {
                                    crate::app::RemoteWorkflowTurnContextResolver::new(app)
                                        .remote_workflow_turn_context_for_prompt(
                                            session_id,
                                            target_agent_id,
                                            started_next,
                                        )
                                })
                                .await?,
                            )
                        } else {
                            None
                        };
                    let submit_result = self
                        .with_app_side_effect(|app| {
                            app.block_on_relay_future(
                                crate::transport::relay_client::send_peer_request_via_temporary_connection(
                                    app.config(),
                                    ClientTarget {
                                        daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                                        daemon_alias: None,
                                    },
                                    RelayPeerRequest::SubmitLeasedPrompt {
                                        leased_agent_id: remote_execution.leased_agent_id.clone(),
                                        prompt: started_next.prompt().to_string(),
                                        attachments,
                                        workflow_context,
                                    },
                                ),
                            )
                        })
                        .await?;
                    if let RelayPeerResponse::LeasedPromptSubmitted {
                        provider_run_id, ..
                    } = submit_result
                    {
                        owned.echo_prompt_to_other_attachments(
                            session_id,
                            &provider_run_id,
                            started_next.source_attachment_id(),
                            started_next.prompt(),
                            started_next.attachments(),
                        );
                    }
                }
                return Ok(completion);
            }
        }
        if next_queued_prompt.is_none() {
            {
                let owned = &self.owned;
                if let Some(completion) = owned.complete_local_prompt_without_advance(
                    session_id,
                    target_agent_id,
                    owned_provider_run_id.as_deref(),
                )? {
                    if completion.completion.completed.workflow_run_id().is_some() {
                        let dispatches = owned.workflow_complete_prompt(
                            session_id,
                            &completion.completion.completed,
                            owned_provider_run_id.as_deref(),
                        )?;
                        self.spawn_workflow_prompt_dispatches(dispatches);
                    }
                    if completion.released_claim
                        && completion.completion.completed.workflow_run_id().is_none()
                    {
                        self.spawn_workflow_prompt_dispatches(
                            owned.workflow_retry_blocked_claims(),
                        );
                    }
                    return Ok(completion.completion);
                }
            }
        } else if let Some(next_queued_prompt) = next_queued_prompt {
            if let Some(completion) = owned.complete_local_prompt_with_queued_advance(
                session_id,
                target_agent_id,
                owned_provider_run_id.as_deref(),
                next_queued_prompt,
            )? {
                let completion_result = completion.completion;
                if completion_result.completed.workflow_run_id().is_some() {
                    let dispatches = owned.workflow_complete_prompt(
                        session_id,
                        &completion_result.completed,
                        owned_provider_run_id.as_deref(),
                    )?;
                    self.spawn_workflow_prompt_dispatches(dispatches);
                }
                if let Some(started_next) = completion_result.started_next.as_ref() {
                    if crate::scheduler::runtime::is_workflow_prompt_attachment(
                        started_next.source_attachment_id(),
                    ) {
                        owned.workflow_start_prompt(session_id, started_next)?;
                    }
                }
                if let Some(dispatch) = completion.dispatch {
                    if let Err(error) = self.enqueue_prompt_dispatch(&dispatch).await {
                        let _ = self.fail_prompt_dispatch(dispatch, error).await;
                    }
                }
                return Ok(completion_result);
            }
        }
        Err(DaemonError::LocalTransport {
            operation: "complete prompt",
            message:
                "owned prompt runtime could not complete prompt without side-effect completion"
                    .to_string(),
        })
    }

    async fn reconcile_provider_run_exit(
        &self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        let owned = &self.owned;

        if let Some(exit) = owned.reconcile_provider_run_liveness_provider_phase(
            session_id,
            provider_run_id,
            None,
        )? {
            let (_, process_key) = self
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(provider_run_id)
                })
                .await
                .unwrap_or((false, None));
            owned.remove_provider_process_tracking_for_run(provider_run_id, process_key);
            return Ok(exit.already_ended);
        }

        let process_running = self
            .with_app_side_effect(|app| {
                crate::app::ProviderLaunchProcessRuntime::new(app).poll_running(provider_run_id)
            })
            .await?;
        let Some(exit) = owned.reconcile_provider_run_liveness_provider_phase(
            session_id,
            provider_run_id,
            Some(process_running),
        )?
        else {
            return Ok(false);
        };
        let (_, process_key) = self
            .with_app_side_effect(|app| {
                crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(provider_run_id)
            })
            .await
            .unwrap_or((false, None));
        owned.remove_provider_process_tracking_for_run(provider_run_id, process_key);
        if exit.already_ended {
            return Ok(true);
        }

        let session_outcome = self
            .settle_owned_provider_prompt(session_id, provider_run_id, false, true)
            .await?;
        let recipients = owned
            .attachment_store
            .list_session_attachment_ids(session_id);
        owned.record_notice(
            session_id,
            Some(provider_run_id),
            recipients,
            format!(
                "Provider run `{}` for `{}` ended unexpectedly. {}",
                provider_run_id,
                exit.ended_run.provider(),
                if session_outcome.had_active_prompt {
                    if session_outcome.started_next_prompt {
                        "The active prompt was closed and Arroba advanced the queued backlog onto the next available provider run."
                    } else {
                        "The active prompt was closed without starting the queued backlog."
                    }
                } else {
                    "No active prompt was running."
                }
            ),
        );
        Ok(true)
    }

    async fn enqueue_prompt_dispatch(
        &self,
        dispatch: &crate::app::KernelPromptDispatch,
    ) -> Result<(), DaemonError> {
        {
            let owned = &self.owned;
            let has_managed_process = owned
                .provider_process_tracking
                .read()
                .run_processes
                .contains_key(&dispatch.provider_run_id);
            if has_managed_process {
                let _ = self
                    .reconcile_provider_run_exit(&dispatch.session_id, &dispatch.provider_run_id)
                    .await?;
            }
            self.enqueue_prompt_dispatch_after_liveness(dispatch, owned)
                .await
        }
    }

    async fn enqueue_prompt_dispatch_after_liveness(
        &self,
        dispatch: &crate::app::KernelPromptDispatch,
        owned: &KernelRuntimeOwnedState,
    ) -> Result<(), DaemonError> {
        owned.echo_prompt_to_other_attachments(
            &dispatch.session_id,
            &dispatch.provider_run_id,
            &dispatch.source_attachment_id,
            &dispatch.prompt,
            &dispatch.attachments,
        );
        let provider_run = owned
            .ensure_provider_run_in_session(&dispatch.session_id, &dispatch.provider_run_id)?;
        if provider_run.state() != crate::provider::ProviderRunState::Running {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: dispatch.provider_run_id.clone(),
                state: provider_run.state(),
                operation: "submit prompt",
            });
        }
        if owned
            .provider_store
            .run_uses_structured_prompt_io(&provider_run)
        {
            owned.note_prompt_started(&dispatch.provider_run_id);
            return owned.provider_store.enqueue_structured_prompt_submit(
                dispatch.session_id.clone(),
                dispatch.provider_run_id.clone(),
                dispatch.agent_id.clone(),
                &provider_run,
                &dispatch.prompt,
                &dispatch.attachments,
            );
        }
        if !crate::scheduler::runtime::is_workflow_prompt_attachment(&dispatch.source_attachment_id)
        {
            let attachment = owned
                .attachment_store
                .get_attachment(&dispatch.source_attachment_id)?;
            if attachment.session_id() != dispatch.session_id {
                return Err(DaemonError::AttachmentNotInSession {
                    session_id: dispatch.session_id.clone(),
                    attachment_id: dispatch.source_attachment_id.clone(),
                });
            }
        }
        owned.terminal_stream.record_input(
            &dispatch.session_id,
            &dispatch.provider_run_id,
            &dispatch.source_attachment_id,
            dispatch.prompt.as_bytes(),
        );
        let has_managed_process = owned
            .provider_process_tracking
            .read()
            .run_processes
            .contains_key(&dispatch.provider_run_id);
        if !has_managed_process {
            owned.note_prompt_started(&dispatch.provider_run_id);
            return Ok(());
        }
        self.with_app_side_effect(|app| {
            app.write_provider_pty_input_for_runtime(
                &dispatch.provider_run_id,
                dispatch.prompt.as_bytes(),
            )
        })
        .await?;
        owned.note_prompt_started(&dispatch.provider_run_id);
        return Ok(());
    }

    async fn fail_prompt_dispatch(
        &self,
        dispatch: crate::app::KernelPromptDispatch,
        error: DaemonError,
    ) -> Result<(), DaemonError> {
        {
            let owned = &self.owned;
            let _ = owned.cancel_active_prompt_only(&dispatch.session_id, &dispatch.agent_id);
            let released_claim = owned.clear_prompt_activity(&dispatch.provider_run_id);
            let _ = owned.session_snapshot(&dispatch.session_id);
            let recipients = owned
                .attachment_store
                .list_session_attachment_ids(&dispatch.session_id);
            owned.record_notice(
                &dispatch.session_id,
                Some(&dispatch.provider_run_id),
                recipients,
                format!("Prompt dispatch failed after acknowledgement: {error}"),
            );
            if released_claim {
                self.spawn_workflow_prompt_dispatches(owned.workflow_retry_blocked_claims());
            }
            Err(error)
        }
    }

    fn spawn_workflow_prompt_dispatches(&self, dispatches: WorkflowPromptDispatches) {
        for dispatch in dispatches.local {
            let state = self.clone();
            tokio::spawn(async move {
                if let Err(error) = state.enqueue_prompt_dispatch(&dispatch).await {
                    let _ = state.fail_prompt_dispatch(dispatch, error).await;
                }
            });
        }
        for dispatch in dispatches.remote {
            self.spawn_remote_prompt_dispatch(dispatch);
        }
    }

    async fn finish_remote_prompt_dispatch(
        &self,
        dispatch: crate::app::KernelRemotePromptDispatch,
        result: Result<String, DaemonError>,
    ) -> Result<(), DaemonError> {
        {
            let owned = &self.owned;
            match result {
                Ok(remote_provider_run_id) => {
                    owned.echo_prompt_to_other_attachments(
                        &dispatch.session_id,
                        &remote_provider_run_id,
                        &dispatch.source_attachment_id,
                        &dispatch.prompt,
                        &dispatch.attachments,
                    );
                    Ok(())
                }
                Err(error) => {
                    let _ =
                        owned.cancel_active_prompt_only(&dispatch.session_id, &dispatch.agent_id);
                    let _ = owned.session_snapshot(&dispatch.session_id);
                    let recipients = owned
                        .attachment_store
                        .list_session_attachment_ids(&dispatch.session_id);
                    owned.record_notice(
                        &dispatch.session_id,
                        None,
                        recipients,
                        format!("Remote prompt dispatch failed after acknowledgement: {error}"),
                    );
                    Err(error)
                }
            }
        }
    }

    async fn enqueue_prompt_abort(
        &self,
        dispatch: &crate::app::KernelPromptAbortDispatch,
    ) -> Result<(), DaemonError> {
        {
            let owned = &self.owned;
            owned.reap_structured_prompt_jobs();
            self.reconcile_provider_run_exit(&dispatch.session_id, &dispatch.provider_run_id)
                .await?;
            let provider_run = owned
                .ensure_provider_run_in_session(&dispatch.session_id, &dispatch.provider_run_id)?;
            if provider_run.state() != crate::provider::ProviderRunState::Running {
                return Err(DaemonError::InvalidProviderRunState {
                    provider_run_id: dispatch.provider_run_id.clone(),
                    state: provider_run.state(),
                    operation: "submit prompt",
                });
            }
            if owned
                .provider_store
                .run_uses_structured_prompt_io(&provider_run)
            {
                return owned.provider_store.enqueue_structured_prompt_abort(
                    dispatch.session_id.clone(),
                    dispatch.provider_run_id.clone(),
                );
            }
            owned.terminal_stream.record_input(
                &dispatch.session_id,
                &dispatch.provider_run_id,
                &dispatch.source_attachment_id,
                b"\x03",
            );
            self.with_app_side_effect(|app| {
                app.write_provider_pty_input_for_runtime(&dispatch.provider_run_id, b"\x03")
            })
            .await?;
            Ok(())
        }
    }

    async fn structured_prompt_io_in_flight(&self, provider_run_id: &str) -> bool {
        {
            let owned = &self.owned;
            owned
                .provider_store
                .structured_prompt_io_in_flight(provider_run_id)
        }
    }

    async fn fail_prompt_abort(
        &self,
        dispatch: crate::app::KernelPromptAbortDispatch,
        error: DaemonError,
    ) -> Result<(), DaemonError> {
        {
            let owned = &self.owned;
            let recipients = owned
                .attachment_store
                .list_session_attachment_ids(&dispatch.session_id);
            owned.record_notice(
                &dispatch.session_id,
                Some(&dispatch.provider_run_id),
                recipients,
                format!("Prompt cancellation dispatch failed after acknowledgement: {error}"),
            );
            Err(error)
        }
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

    pub(crate) async fn execute_workflow_request(
        &self,
        request: LocalDaemonRequest,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<crate::session::RuntimeSession>,
    ) {
        let owned = &self.owned;

        match request {
            LocalDaemonRequest::CreateWorkflow(request) => {
                let result = owned.workflow_create_workflow(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::AliasWorkflow(request) => {
                let result = owned.workflow_alias_workflow(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::ListWorkflows(request) => {
                (owned.workflow_list_workflows(request), None)
            }
            LocalDaemonRequest::ResolveWorkflow(request) => {
                (owned.workflow_resolve_workflow(request), None)
            }
            LocalDaemonRequest::CreateWorkflowEndpoint(request) => {
                let result = owned.workflow_create_endpoint(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::AliasWorkflowEndpoint(request) => {
                let result = owned.workflow_alias_endpoint(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::BindWorkflowEndpoint(request) => {
                let result = owned.workflow_bind_endpoint(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::AddWorkflowNode(request) => {
                let result = owned.workflow_add_node(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::RemoveWorkflowNode(request) => {
                let result = owned.workflow_remove_node(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::UpdateWorkflowNodeInstructions(request) => {
                let result = owned.workflow_update_node_instructions(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::SetWorkflowNodeCanCompleteRun(request) => {
                let result = owned.workflow_set_node_can_complete_run(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::SetWorkflowNodeCanEmitIntermediateOutput(request) => {
                let result = owned.workflow_set_node_can_emit_intermediate_output(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::SetWorkflowNodeIntermediateOutputSchema(request) => {
                let result = owned.workflow_set_node_intermediate_output_schema(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::SetWorkflowNodeMaxTurns(request) => {
                let result = owned.workflow_set_node_max_turns(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::AddWorkflowEdge(request) => {
                let result = owned.workflow_add_edge(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::RemoveWorkflowEdge(request) => {
                let result = owned.workflow_remove_edge(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::SetWorkflowFlushContext(request) => {
                let result = owned.workflow_set_flush_context(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::SetWorkflowRunOutputSchema(request) => {
                let result = owned.workflow_set_run_output_schema(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::SetWorkflowIntermediateOutputSchema(request) => {
                let result = owned.workflow_set_intermediate_output_schema(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::SetWorkflowLaunchPolicy(request) => {
                let result = owned.workflow_set_launch_policy(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::ListWorkflowRuns(request) => {
                (owned.workflow_list_runs(request), None)
            }
            LocalDaemonRequest::GetWorkflowRun(request) => (owned.workflow_get_run(request), None),
            LocalDaemonRequest::CreateWorkflowWatchdog(request) => {
                let result = owned.workflow_create_watchdog(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::ListWorkflowWatchdogs(request) => {
                (owned.workflow_list_watchdogs(request), None)
            }
            LocalDaemonRequest::SetWorkflowWatchdogEnabled(request) => {
                let result = owned.workflow_set_watchdog_enabled(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::RemoveWorkflowWatchdog(request) => {
                let result = owned.workflow_remove_watchdog(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::ListQueuedWorkflowLaunches(request) => {
                (owned.workflow_list_queued_launches(request), None)
            }
            LocalDaemonRequest::RemoveQueuedWorkflowLaunch(request) => {
                let result = owned.workflow_remove_queued_launch(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::ClearQueuedWorkflowLaunches(request) => {
                let result = owned.workflow_clear_queued_launches(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::InvokeWorkflowEndpoint(request) => {
                let session_id = request.session_id.clone();
                let result = match owned.workflow_invoke_endpoint_with_admission(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.endpoint_ref,
                    request.prompt,
                ) {
                    Ok((outcome, dispatches)) => {
                        self.spawn_workflow_prompt_dispatches(dispatches);
                        let session = match owned.session_snapshot(&request.session_id) {
                            Ok(session) => session,
                            Err(error) => return (Err(error), None),
                        };
                        match outcome {
                            crate::app::workflow_runtime::WorkflowLaunchOutcome::Started {
                                workflow_run,
                                workflow,
                                endpoint,
                            } => Ok(LocalDaemonResponse::WorkflowRunInvoked {
                                workflow_run,
                                workflow,
                                endpoint,
                                session,
                            }),
                            crate::app::workflow_runtime::WorkflowLaunchOutcome::Queued {
                                queued_launch,
                                workflow,
                                endpoint,
                            } => Ok(LocalDaemonResponse::WorkflowRunQueued {
                                queued_launch,
                                workflow,
                                endpoint,
                                session,
                            }),
                        }
                    }
                    Err(error) => Err(error),
                };
                let session = result
                    .as_ref()
                    .ok()
                    .and_then(workflow_response_session)
                    .or_else(|| owned.session_snapshot(&session_id).ok());
                (result, session)
            }
            LocalDaemonRequest::CancelWorkflowRun(request) => {
                let session_id = request.session_id.clone();
                let result = (|| {
                    let workflow_run_id = owned
                        .session_store
                        .read()
                        .resolve_workflow_run_ref(&request.session_id, &request.workflow_run_ref)?
                        .id()
                        .to_string();
                    let session = owned.session_store.get_session(&request.session_id)?;
                    for agent in owned.agent_store.get_session_agents(&request.session_id) {
                        if owned
                            .prompt_state_owner
                            .active_prompt_for_agent(&session, agent.id())
                            .and_then(|prompt| prompt.workflow_run_id().map(str::to_string))
                            .as_deref()
                            == Some(workflow_run_id.as_str())
                        {
                            let _ = owned
                                .prompt_state_owner
                                .begin_cancelling_active_prompt(&session, agent.id())
                                .ok_or_else(|| DaemonError::NoActivePrompt {
                                    session_id: request.session_id.clone(),
                                })?;
                            let (active_prompt, queued_prompts) =
                                owned.prompt_state_owner.state_parts(&session, agent.id());
                            owned.session_store.mirror_agent_prompt_state(
                                &request.session_id,
                                agent.id(),
                                active_prompt,
                                queued_prompts,
                            )?;
                        }
                    }
                    let workflow_run = owned
                        .session_store
                        .write()
                        .cancel_workflow_run(&request.session_id, &request.workflow_run_ref)?;
                    let _ = owned.prompt_workspace_claims.remove_matching(|claim| {
                        claim.session_id == request.session_id
                            && claim.operation == "workflow_node_dispatch"
                    });
                    let workflow = owned
                        .session_store
                        .read()
                        .resolve_workflow_ref(&request.session_id, workflow_run.workflow_id())?;
                    for node in workflow.nodes() {
                        if let Some(run) = owned
                            .provider_store
                            .get_run_for_agent(&request.session_id, node.agent_id())
                        {
                            let _ = owned.clear_prompt_activity(run.id());
                        }
                    }
                    let session = owned.session_store.get_session(&request.session_id)?;
                    let _ = owned
                        .prompt_state_owner
                        .remove_queued_prompts_by_workflow_run(&session, &workflow_run_id);
                    for agent in owned.agent_store.get_session_agents(&request.session_id) {
                        let (active_prompt, queued_prompts) =
                            owned.prompt_state_owner.state_parts(&session, agent.id());
                        let _ = owned.session_store.mirror_agent_prompt_state(
                            &request.session_id,
                            agent.id(),
                            active_prompt,
                            queued_prompts,
                        );
                    }
                    owned.workflow_maybe_start_next_queued_launch(&request.session_id);
                    let session = owned.session_snapshot(&request.session_id)?;
                    Ok(LocalDaemonResponse::WorkflowRunCancelled {
                        workflow_run,
                        session,
                    })
                })();
                let session = result
                    .as_ref()
                    .ok()
                    .and_then(workflow_response_session)
                    .or_else(|| owned.session_snapshot(&session_id).ok());
                (result, session)
            }
            LocalDaemonRequest::ResumeWorkflowRun(request) => {
                let session_id = request.session_id.clone();
                let result = match owned
                    .workflow_resume_run(&request.session_id, &request.workflow_run_ref)
                {
                    Ok((workflow_run, dispatches)) => {
                        self.spawn_workflow_prompt_dispatches(dispatches);
                        owned.workflow_session(&request.session_id).map(|session| {
                            LocalDaemonResponse::WorkflowRunResumed {
                                workflow_run,
                                session,
                            }
                        })
                    }
                    Err(error) => Err(error),
                };
                let session = result
                    .as_ref()
                    .ok()
                    .and_then(workflow_response_session)
                    .or_else(|| owned.session_snapshot(&session_id).ok());
                (result, session)
            }
            LocalDaemonRequest::ValidateWorkflowOutput(request) => {
                let result = owned.workflow_validate_output(request);
                (result, None)
            }
            LocalDaemonRequest::AckWorkflowTurn(request) => {
                let result = owned.workflow_ack_turn(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            _ => (
                Err(DaemonError::LocalTransport {
                    operation: "execute workflow request",
                    message: "request is not handled by the workflow runtime".to_string(),
                }),
                None,
            ),
        }
    }

    pub(crate) async fn start_provider_launch(
        &self,
        request: crate::local::LaunchProviderRunRequest,
    ) -> Result<(crate::app::StartedProviderLaunch, u64), DaemonError> {
        let launch_request = self.launch_provider_request_from_owned_state(request);
        {
            let owned = &self.owned;
            let config = owned.config_projection.snapshot();
            let launch_request =
                owned.prepare_provider_launch_request(launch_request, config.runtime_mcp_url())?;
            crate::logging::info_with_fields(
                "daemon.app",
                "launching provider run",
                serde_json::json!({
                    "adapter_key": launch_request.adapter_key.clone(),
                    "agent_id": launch_request.agent_id.clone(),
                    "provider": launch_request.provider.clone(),
                    "session_id": launch_request.session_id.clone(),
                }),
            );
            let started = owned.start_provider_launch(launch_request)?;
            let run = started.run.clone();
            if let Some(previous_active_run_id) = started.previous_active_run_id.as_deref() {
                if let Ok(previous_run) = owned.provider_store.get_run(previous_active_run_id) {
                    owned.provider_run_projection.update(previous_run);
                }
            }
            crate::logging::info_with_fields(
                "daemon.app",
                "prepared provider run endpoint metadata",
                serde_json::json!({
                    "provider_run_id": run.id(),
                    "endpoint_mode": run.endpoint_mode().to_string(),
                    "session_id": run.session_id(),
                    "provider": run.provider(),
                }),
            );
            if let Err(error) = self
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app).spawn_for_launch(&run)
                })
                .await
            {
                crate::logging::error_with_fields(
                    "daemon.app",
                    "PTY spawn failed for provider run",
                    serde_json::json!({
                        "provider_run_id": run.id(),
                        "session_id": run.session_id(),
                        "error": error.to_string(),
                    }),
                );
                if let Ok(outcome) = owned
                    .provider_store
                    .terminate_run_provider_only(run.session_id(), run.id())
                {
                    let _ = owned.clear_active_provider_run_session_pointer(
                        run.session_id(),
                        outcome.run().id(),
                    );
                    owned.provider_run_projection.update(outcome.into_run());
                }
                if let Some(previous_active_run_id) = started.previous_active_run_id.as_deref() {
                    let recipients = owned
                        .attachment_store
                        .list_session_attachment_ids(run.session_id());
                    match owned
                        .resume_provider_run_for_session(run.session_id(), previous_active_run_id)
                    {
                        Ok(resumed_run) => {
                            owned.record_notice(
                                run.session_id(),
                                Some(resumed_run.id()),
                                recipients,
                                format!(
                                    "Provider switch failed for session `{}`. Arroba resumed the previous provider run `{}` automatically.",
                                    run.session_id(),
                                    resumed_run.id()
                                ),
                            );
                        }
                        Err(resume_error) => {
                            owned.record_notice(
                                run.session_id(),
                                None,
                                recipients,
                                format!(
                                    "Provider switch failed for session `{}` and Arroba could not resume the previous provider run: {}",
                                    run.session_id(),
                                    resume_error
                                ),
                            );
                        }
                    }
                }
                return Err(error);
            }
            owned.provider_run_projection.update(run);
            Ok((started, config.provider_runtime_init_delay_ms))
        }
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
        if crate::provider::provider_requires_managed_io_by_default(&launch_request.provider) {
            launch_request = launch_request.with_managed_io_required();
        }
        if let Some(agent_id) = request.agent_id.clone().or_else(|| {
            self.owned
                .session_store
                .get_session(&request.session_id)
                .ok()
                .and_then(|session| session.focused_agent_id().map(str::to_string))
                .or_else(|| {
                    self.owned
                        .agent_store
                        .get_focused_agent(&request.session_id)
                        .map(|agent| agent.id().to_string())
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
        {
            let owned = &self.owned;
            let result = owned.finish_provider_launch_success(started, binding);
            match result {
                Ok(run) => {
                    if let Some(agent_id) = run.agent_instance_id() {
                        match owned.advance_next_queued_prompt_dispatch(
                            run.session_id(),
                            agent_id,
                            run.id(),
                        ) {
                            Ok(Some(dispatch)) => {
                                if let Err(error) = self.enqueue_prompt_dispatch(&dispatch).await {
                                    let _ = self.fail_prompt_dispatch(dispatch, error).await;
                                }
                            }
                            Ok(None) => {}
                            Err(error) => {
                                self.fail_provider_launch(started, &error).await;
                                return;
                            }
                        }
                        let _ = owned.session_snapshot(run.session_id());
                    }
                }
                Err(error) => {
                    self.fail_provider_launch(started, &error).await;
                }
            }
        }
    }

    pub(crate) async fn fail_provider_launch(
        &self,
        started: &crate::app::StartedProviderLaunch,
        error: &DaemonError,
    ) {
        {
            let owned = &self.owned;
            crate::logging::error_with_fields(
                "daemon.app",
                "provider runtime initialization failed",
                serde_json::json!({
                    "provider_run_id": started.run.id(),
                    "session_id": started.run.session_id(),
                    "error": error.to_string(),
                }),
            );
            let recipients = owned
                .attachment_store
                .list_session_attachment_ids(started.run.session_id());
            owned.record_notice(
                started.run.session_id(),
                Some(started.run.id()),
                recipients,
                format!(
                    "Provider launch `{}` failed before it became ready: {}",
                    started.run.id(),
                    error
                ),
            );
            let (_, process_key) = self
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(started.run.id())
                })
                .await
                .unwrap_or((false, None));
            owned.remove_provider_process_tracking_for_run(started.run.id(), process_key);
            owned.provider_store.clear_runtime(started.run.id());
            if let Ok(outcome) = owned
                .provider_store
                .terminate_run_provider_only(started.run.session_id(), started.run.id())
            {
                let _ = owned.clear_active_provider_run_session_pointer(
                    started.run.session_id(),
                    outcome.run().id(),
                );
                owned.provider_run_projection.update(outcome.into_run());
            }
            if let Some(previous_active_run_id) = started.previous_active_run_id.as_deref() {
                let _ = owned.resume_provider_run_for_session(
                    started.run.session_id(),
                    previous_active_run_id,
                );
            }
            let _ = owned.session_snapshot(started.run.session_id());
        }
    }

    async fn settle_owned_provider_prompt(
        &self,
        session_id: &str,
        provider_run_id: &str,
        prompt_completed: bool,
        force: bool,
    ) -> Result<crate::app::ProviderRunExitSessionSummary, DaemonError> {
        let owned = &self.owned;
        let provider_run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        let agent_id = provider_run
            .agent_instance_id()
            .map(str::to_string)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: "provider run has no agent".to_string(),
            })?;
        let active_prompt = owned
            .prompt_state_owner
            .active_prompt_for_agent(&owned.session_store.get_session(session_id)?, &agent_id);
        let Some(active_prompt) = active_prompt else {
            if owned.clear_prompt_activity(provider_run_id) {
                self.spawn_workflow_prompt_dispatches(owned.workflow_retry_blocked_claims());
            }
            let _ = owned.sync_focused_provider_run_if_idle(session_id);
            let _ = owned.session_snapshot(session_id);
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: false,
                started_next_prompt: false,
            });
        };

        if active_prompt.status() == crate::session::PromptStatus::Cancelling {
            if !force && !prompt_completed && !owned.prompt_should_settle(provider_run_id) {
                return Ok(crate::app::ProviderRunExitSessionSummary {
                    had_active_prompt: true,
                    started_next_prompt: false,
                });
            }
            let cancellation = owned.finalize_local_prompt_cancellation_with_queued_advance(
                session_id,
                &agent_id,
                Some(provider_run_id),
            )?;
            owned.workflow_cancel_prompt(session_id, &cancellation.cancellation.prompt)?;
            if cancellation.released_claim {
                self.spawn_workflow_prompt_dispatches(owned.workflow_retry_blocked_claims());
            }
            if let Some(dispatch) = cancellation.dispatch {
                if let Err(error) = self
                    .enqueue_prompt_dispatch_after_liveness(&dispatch, owned)
                    .await
                {
                    let _ = self.fail_prompt_dispatch(dispatch, error).await;
                }
            }
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: true,
                started_next_prompt: cancellation.cancellation.started_next.is_some(),
            });
        }

        if !force && !prompt_completed && !owned.prompt_should_settle(provider_run_id) {
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: true,
                started_next_prompt: false,
            });
        }
        if !force {
            if let (Some(workflow_run_id), Some(workflow_node_run_id)) = (
                active_prompt.workflow_run_id(),
                active_prompt.workflow_node_run_id(),
            ) {
                if !owned.workflow_prompt_has_completion_output(
                    session_id,
                    workflow_run_id,
                    workflow_node_run_id,
                    provider_run_id,
                ) {
                    return Ok(crate::app::ProviderRunExitSessionSummary {
                        had_active_prompt: true,
                        started_next_prompt: false,
                    });
                }
            }
        }
        let provider_run_state = provider_run.state();
        let next_queued_prompt = if provider_run_state == crate::provider::ProviderRunState::Running
        {
            owned
                .prompt_state_owner
                .peek_next_queued_prompt(&owned.session_store.get_session(session_id)?, &agent_id)
        } else {
            None
        };
        let completion = if let Some(next_queued_prompt) = next_queued_prompt.as_ref() {
            owned.complete_local_prompt_with_queued_advance(
                session_id,
                &agent_id,
                Some(provider_run_id),
                next_queued_prompt,
            )?
        } else {
            owned.complete_local_prompt_without_advance(
                session_id,
                &agent_id,
                Some(provider_run_id),
            )?
        }
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "settle provider prompt",
            message: "owned prompt runtime could not settle provider prompt".to_string(),
        })?;
        if completion.completion.completed.workflow_run_id().is_some() {
            let dispatches = owned.workflow_complete_prompt(
                session_id,
                &completion.completion.completed,
                Some(provider_run_id),
            )?;
            self.spawn_workflow_prompt_dispatches(dispatches);
        }
        if let Some(started_next) = completion.completion.started_next.as_ref() {
            if crate::scheduler::runtime::is_workflow_prompt_attachment(
                started_next.source_attachment_id(),
            ) {
                owned.workflow_start_prompt(session_id, started_next)?;
            }
        }
        if completion.released_claim && completion.completion.completed.workflow_run_id().is_none()
        {
            self.spawn_workflow_prompt_dispatches(owned.workflow_retry_blocked_claims());
        }
        if let Some(dispatch) = completion.dispatch {
            if let Err(error) = self
                .enqueue_prompt_dispatch_after_liveness(&dispatch, owned)
                .await
            {
                let _ = self.fail_prompt_dispatch(dispatch, error).await;
            }
        }
        Ok(crate::app::ProviderRunExitSessionSummary {
            had_active_prompt: true,
            started_next_prompt: completion.completion.started_next.is_some(),
        })
    }

    async fn pump_owned_provider_output(
        &self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
        initial_liveness_already_checked: bool,
    ) -> Result<Vec<crate::terminal::TerminalOutputRecord>, DaemonError> {
        let owned = &self.owned;
        owned.reap_structured_prompt_jobs();
        if !initial_liveness_already_checked
            && self
                .reconcile_provider_run_exit(session_id, provider_run_id)
                .await?
        {
            return Ok(Vec::new());
        }
        let provider_run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        if matches!(
            provider_run.state(),
            crate::provider::ProviderRunState::Ended | crate::provider::ProviderRunState::Parked
        ) {
            return Ok(Vec::new());
        }

        if owned
            .provider_store
            .run_uses_structured_prompt_io(&provider_run)
        {
            return self
                .pump_owned_structured_provider_output(
                    session_id,
                    provider_run_id,
                    recipient_attachment_ids,
                )
                .await;
        }

        let chunks = match self
            .with_app_side_effect(|app| app.drain_provider_pty_output_for_runtime(provider_run_id))
            .await
        {
            Ok(chunks) => chunks,
            Err(error) => {
                if self
                    .reconcile_provider_run_exit(session_id, provider_run_id)
                    .await?
                {
                    return Ok(Vec::new());
                }
                return Err(error);
            }
        };
        if !chunks.is_empty() {
            owned.note_prompt_response_content(provider_run_id);
        }
        if !self
            .reconcile_provider_run_exit(session_id, provider_run_id)
            .await?
        {
            let _ = self
                .settle_owned_provider_prompt(session_id, provider_run_id, false, false)
                .await?;
        }
        Ok(chunks
            .into_iter()
            .map(|chunk| {
                owned.fan_out_terminal_output(
                    session_id,
                    provider_run_id,
                    crate::terminal::TerminalOutputKind::ProviderOutput,
                    None,
                    recipient_attachment_ids.clone(),
                    &chunk.bytes,
                )
            })
            .collect())
    }

    async fn pump_owned_structured_provider_output(
        &self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
    ) -> Result<Vec<crate::terminal::TerminalOutputRecord>, DaemonError> {
        let owned = &self.owned;
        let provider_run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        if provider_run.state() == crate::provider::ProviderRunState::Parked {
            return Ok(Vec::new());
        }
        if provider_run.endpoint_mode() != crate::provider::AgentEndpointMode::External {
            if let Err(error) = self
                .with_app_side_effect(|app| {
                    app.drain_provider_pty_output_for_runtime(provider_run_id)
                })
                .await
            {
                if self
                    .reconcile_provider_run_exit(session_id, provider_run_id)
                    .await?
                {
                    return Ok(Vec::new());
                }
                if !matches!(error, DaemonError::PtyProcessNotFound { .. }) {
                    return Err(error);
                }
            }
        }
        let mut records = owned.structured_output_records.take(provider_run_id);
        for finished in owned
            .provider_store
            .drain_finished_structured_output_poll_jobs()
        {
            let finished_run_id = finished.provider_run_id.clone();
            let is_requested_run = finished_run_id == provider_run_id;
            let poll_result = match finished.result {
                Ok(Some(poll_result)) => poll_result,
                Ok(None) => continue,
                Err(error) => {
                    let reconcile_result = if is_requested_run {
                        self.reconcile_provider_run_exit(session_id, provider_run_id)
                            .await
                    } else {
                        match owned.provider_store.get_run(&finished_run_id) {
                            Ok(run) => {
                                self.reconcile_provider_run_exit(run.session_id(), &finished_run_id)
                                    .await
                            }
                            Err(run_error) => Err(run_error),
                        }
                    };
                    match reconcile_result {
                        Ok(true) => continue,
                        Ok(false) if is_requested_run => return Err(error),
                        Ok(false) => {
                            crate::logging::error_with_fields(
                                "daemon.app",
                                "background structured output poll failed",
                                serde_json::json!({
                                    "provider_run_id": finished_run_id,
                                    "error": error.to_string(),
                                }),
                            );
                            continue;
                        }
                        Err(reconcile_error) if is_requested_run => return Err(reconcile_error),
                        Err(reconcile_error) => {
                            crate::logging::error_with_fields(
                                "daemon.app",
                                "background structured output poll reconciliation failed",
                                serde_json::json!({
                                    "provider_run_id": finished_run_id,
                                    "error": reconcile_error.to_string(),
                                }),
                            );
                            continue;
                        }
                    }
                }
            };
            let run = match owned.provider_store.get_run(&finished_run_id) {
                Ok(run) => run,
                Err(_) => continue,
            };
            let run_session_id = run.session_id().to_string();
            let recipients = if is_requested_run {
                recipient_attachment_ids.clone()
            } else {
                owned
                    .attachment_store
                    .list_session_attachment_ids(&run_session_id)
            };
            let applied = self
                .apply_owned_structured_output_batch(
                    &run_session_id,
                    &finished_run_id,
                    recipients,
                    poll_result,
                )
                .await?;
            if is_requested_run {
                records.extend(applied);
            } else {
                owned
                    .structured_output_records
                    .append(finished_run_id, applied);
            }
        }
        owned
            .provider_store
            .enqueue_structured_output_poll(provider_run_id)?;
        Ok(records)
    }

    async fn apply_owned_structured_output_batch(
        &self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
        poll_result: crate::provider::ProviderPromptSignalBatch,
    ) -> Result<Vec<crate::terminal::TerminalOutputRecord>, DaemonError> {
        let owned = &self.owned;
        owned
            .provider_store
            .apply_structured_output_metadata(provider_run_id, &poll_result)?;
        let provider_run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        owned.provider_run_projection.update(provider_run);
        for notice in &poll_result.notices {
            owned.record_notice(
                session_id,
                Some(provider_run_id),
                recipient_attachment_ids.clone(),
                notice.to_string(),
            );
        }
        let saw_response_content = poll_result.chunks.iter().any(|chunk| {
            matches!(
                chunk.kind,
                crate::terminal::TerminalOutputKind::ProviderOutput
                    | crate::terminal::TerminalOutputKind::ProviderReasoning
            )
        });
        let saw_runtime_activity = poll_result.chunks.iter().any(|chunk| {
            matches!(
                chunk.kind,
                crate::terminal::TerminalOutputKind::ProviderOutput
                    | crate::terminal::TerminalOutputKind::ProviderReasoning
                    | crate::terminal::TerminalOutputKind::ProviderTool
                    | crate::terminal::TerminalOutputKind::ProviderStatus
            )
        });
        if saw_response_content {
            owned.note_prompt_response_content(provider_run_id);
        } else if saw_runtime_activity {
            owned.note_prompt_output(provider_run_id);
        }
        for completion in &poll_result.completions {
            owned.record_assistant_message_completion(
                session_id,
                provider_run_id,
                recipient_attachment_ids.clone(),
                &completion.message_id,
                completion.completed_at_ms,
            );
            owned.mark_prompt_completion_recorded(provider_run_id);
        }
        let prompt_completed = poll_result.prompt_completed;
        let records = poll_result
            .chunks
            .into_iter()
            .map(|chunk| {
                owned.fan_out_terminal_output(
                    session_id,
                    provider_run_id,
                    chunk.kind,
                    chunk.merge_key,
                    recipient_attachment_ids.clone(),
                    &chunk.bytes,
                )
            })
            .collect::<Vec<_>>();
        if !self
            .reconcile_provider_run_exit(session_id, provider_run_id)
            .await?
        {
            let _ = self
                .settle_owned_provider_prompt(session_id, provider_run_id, prompt_completed, false)
                .await?;
        }
        Ok(records)
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
        let owned = &self.owned;
        owned.reap_structured_prompt_jobs();
        owned.ensure_attachment_in_session(session_id, attachment_id)?;
        let active_provider_run_id = owned
            .session_store
            .get_session(session_id)?
            .active_provider_run_id()
            .map(str::to_string);
        let mut provider_run_ids = BTreeSet::new();
        if let Some(provider_run_id) = active_provider_run_id {
            provider_run_ids.insert(provider_run_id);
        }
        provider_run_ids.extend(
            owned
                .provider_store
                .list_runs()
                .into_iter()
                .filter(|run| run.session_id() == session_id)
                .filter(|run| {
                    matches!(
                        run.state(),
                        crate::provider::ProviderRunState::Starting
                            | crate::provider::ProviderRunState::Running
                    )
                })
                .map(|run| run.id().to_string()),
        );
        let recipient_attachment_ids = owned
            .attachment_store
            .list_session_attachment_ids(session_id);
        for provider_run_id in provider_run_ids {
            let _ = self
                .pump_owned_provider_output(
                    session_id,
                    &provider_run_id,
                    recipient_attachment_ids.clone(),
                    false,
                )
                .await?;
        }
        let records = owned
            .terminal_stream
            .drain_output_records(session_id, attachment_id);
        let session = owned.session_snapshot(session_id).ok();
        Ok((records, session))
    }

    pub(crate) async fn capability_context(
        &self,
        session_id: &str,
        attachment_id: &str,
        capability: &'static str,
    ) -> Result<CapabilityRuntimeSnapshot, DaemonError> {
        self.owned
            .capability_context(session_id, attachment_id, capability)
    }

    pub(crate) async fn dispatch_authenticated_runtime_tool_call(
        &self,
        auth_token: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        {
            let owned = &self.owned;
            let canonical_tool_name = tool_name.strip_prefix("arroba_").unwrap_or(tool_name);
            let provider_runs = owned
                .provider_store
                .get_runs_by_runtime_mcp_auth_token(auth_token);
            if provider_runs.is_empty() {
                return Err(DaemonError::LocalTransport {
                    operation: "dispatch_authenticated_runtime_tool_call",
                    message: "invalid runtime MCP auth token".to_string(),
                });
            }
            if matches!(
                canonical_tool_name,
                crate::transport::runtime_tools::READ_ARTIFACT_TOOL
                    | crate::transport::runtime_tools::EDIT_ARTIFACT_TOOL
                    | crate::transport::runtime_tools::APPLY_PATCH_TOOL
                    | crate::transport::runtime_tools::DELETE_ARTIFACT_TOOL
                    | crate::transport::runtime_tools::MOVE_ARTIFACT_TOOL
                    | crate::transport::runtime_tools::WRITE_ARTIFACT_TOOL
            ) {
                if let Some(result) = self
                    .try_dispatch_remote_managed_io_runtime_tool_call(
                        &provider_runs[0],
                        canonical_tool_name,
                        arguments.clone(),
                    )
                    .await?
                {
                    return Ok(result);
                }
                return self
                    .dispatch_managed_io_runtime_tool_call(
                        &provider_runs[0],
                        canonical_tool_name,
                        arguments,
                    )
                    .await;
            }
            let provider_run_ids = provider_runs
                .iter()
                .map(|run| run.id().to_string())
                .collect::<Vec<_>>();
            let leased_workflow_context = self
                .with_app_side_effect(|app| {
                    let runtime = crate::app::RemoteLeaseRuntime::new(app);
                    provider_run_ids.iter().find_map(|provider_run_id| {
                        runtime.leased_workflow_turn_context_for_provider_run(provider_run_id)
                    })
                })
                .await;
            if let Some(context) = leased_workflow_context {
                let response = self
                    .with_app_side_effect(|app| {
                        app.block_on_relay_future(
                            crate::transport::relay_client::send_peer_request_via_temporary_connection(
                                app.config(),
                                ClientTarget {
                                    daemon_id: Some(context.home_kernel_id.clone()),
                                    daemon_alias: None,
                                },
                                RelayPeerRequest::ForwardWorkflowRuntimeTool {
                                    context: context.clone(),
                                    tool_name: canonical_tool_name.to_string(),
                                    arguments: arguments.clone(),
                                },
                            ),
                        )
                    })
                    .await?;
                return match response {
                    RelayPeerResponse::WorkflowRuntimeToolHandled { result } => {
                        if leased_workflow_tool_result_should_complete_turn(
                            canonical_tool_name,
                            &result,
                        ) {
                            self.with_app_side_effect(|app| {
                                let mut runtime = crate::app::RemoteLeaseRuntime::new(app);
                                for provider_run_id in &provider_run_ids {
                                    if runtime
                                        .leased_workflow_turn_context_for_provider_run(
                                            provider_run_id,
                                        )
                                        .is_some()
                                    {
                                        let _ = runtime
                                            .complete_leased_workflow_prompt_for_provider_run(
                                                provider_run_id,
                                            )?;
                                        break;
                                    }
                                }
                                Ok(())
                            })
                            .await?;
                        }
                        Ok(result)
                    }
                    other => Err(DaemonError::LocalTransport {
                        operation: "forward leased workflow runtime tool",
                        message: format!("unexpected forwarded workflow tool response: {other:?}"),
                    }),
                };
            }
            let requested_delivery_token = match canonical_tool_name {
                crate::transport::runtime_tools::ACK_WORKFLOW_TURN_TOOL => {
                    serde_json::from_value::<crate::transport::runtime_tools::AckWorkflowTurnArgs>(
                        arguments.clone(),
                    )
                    .ok()
                    .map(|args| args.delivery_token)
                }
                crate::transport::runtime_tools::VALIDATE_WORKFLOW_OUTPUT_TOOL => {
                    serde_json::from_value::<
                        crate::transport::runtime_tools::ValidateWorkflowOutputArgs,
                    >(arguments.clone())
                    .ok()
                    .and_then(|args| args.delivery_token)
                }
                crate::transport::runtime_tools::VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL
                | crate::transport::runtime_tools::VALIDATE_AND_SUBMIT_INTERMEDIATE_WORKFLOW_RUN_OUTPUT_TOOL => {
                    serde_json::from_value::<
                        crate::transport::runtime_tools::ValidateAndSubmitWorkflowRunOutputArgs,
                    >(arguments.clone())
                    .ok()
                    .and_then(|args| args.delivery_token)
                }
                _ => None,
            };
            let session_id = provider_runs[0].session_id().to_string();
            let candidate_agent_ids = provider_runs
                .iter()
                .filter_map(|run| run.agent_instance_id().map(str::to_string))
                .collect::<Vec<_>>();
            let (workflow_run_ref, workflow_node_run_id) = owned
                .resolve_owned_authenticated_workflow_turn(
                    &session_id,
                    &candidate_agent_ids,
                    requested_delivery_token.as_deref(),
                )?;
            let context = owned.workflow_tool_context(
                session_id,
                workflow_run_ref,
                workflow_node_run_id,
                None,
            )?;
            let (result, dispatches) = owned.dispatch_workflow_runtime_tool_call(
                canonical_tool_name.to_string(),
                arguments,
                context,
            )?;
            self.spawn_workflow_prompt_dispatches(dispatches);
            Ok(result)
        }
    }

    async fn dispatch_managed_io_runtime_tool_call(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let workspace_context = self
            .managed_io_workspace_for_provider_run(provider_run)
            .await?;
        if !workspace_context.valid {
            return Ok(managed_io_workspace_identity_rejected(&workspace_context));
        }
        let workspace_root = workspace_context.root.clone();
        let workspace_identity = workspace_context.identity.clone();
        let mut coordinator = self.owned.managed_io_coordinator.lock().await;
        match tool_name {
            crate::transport::runtime_tools::READ_ARTIFACT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManagedReadArtifactArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_read_artifact",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain =
                    KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
                let read = crate::io::ManagedFileIo::read_artifact(
                    &mut coordinator,
                    crate::io::ManagedFileReadRequest {
                        workspace_identity: workspace_identity.clone(),
                        workspace_root: workspace_root.clone(),
                        path: PathBuf::from(args.path),
                        domain,
                    },
                )
                .map_err(managed_io_daemon_error)?;
                self.owned.managed_io_external_changes.observe_managed_read(
                    provider_run.id(),
                    &workspace_identity,
                    &workspace_root,
                    &read.path,
                );
                let mut payload = managed_io_read_payload(read);
                add_managed_io_workspace_payload(&mut payload, &workspace_context);
                Ok(crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload })
            }
            crate::transport::runtime_tools::EDIT_ARTIFACT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManagedEditArtifactArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_edit_artifact",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain =
                    KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
                if domain != crate::io::ArtifactDomainKind::TextDocument {
                    return Err(DaemonError::LocalTransport {
                        operation: "runtime_tool_edit_artifact",
                        message: "managed edit currently supports only text artifacts".to_string(),
                    });
                }
                let operation = match (args.range, args.old_text) {
                    (Some(range), Some(old_text)) => crate::io::AgentEditOperation::ReplaceRange {
                        range: crate::io::TextRange::new(range.start, range.end),
                        old_text,
                        new_text: args.new_text,
                    },
                    (None, Some(old_text)) => crate::io::AgentEditOperation::ReplaceText {
                        old_text,
                        new_text: args.new_text,
                    },
                    (Some(_), None) => {
                        return Err(DaemonError::LocalTransport {
                            operation: "runtime_tool_edit_artifact",
                            message: "range edits require old_text".to_string(),
                        });
                    }
                    (None, None) => {
                        return Err(DaemonError::LocalTransport {
                            operation: "runtime_tool_edit_artifact",
                            message: "managed text edits require old_text or range+old_text"
                                .to_string(),
                        });
                    }
                };
                let path = PathBuf::from(args.path.clone());
                let before = managed_io_text_for_diff(&workspace_root, &path, false);
                let reservation_ranges = managed_io_reservation_ranges_for_operation(
                    &operation,
                    before.as_ref(),
                    crate::io::TextRange::new(0, usize::MAX),
                );
                let reservation = match managed_io_try_reserve_ranges(
                    &mut coordinator,
                    &workspace_identity,
                    &path,
                    reservation_ranges,
                    managed_io_reservation_owner(provider_run, tool_name),
                ) {
                    Ok(reservation) => reservation,
                    Err(mut output) => {
                        add_managed_io_workspace_payload(&mut output.payload, &workspace_context);
                        return Ok(output);
                    }
                };
                let external_change_notice = self
                    .owned
                    .managed_io_external_changes
                    .external_change_notice(&workspace_identity, &path);
                let result = crate::io::ManagedFileIo::apply_edit(
                    &mut coordinator,
                    crate::io::ManagedFileWriteRequest {
                        workspace_identity: workspace_identity.clone(),
                        workspace_root: workspace_root.clone(),
                        domain,
                        intent: crate::io::AgentEditIntent {
                            path: path.clone(),
                            snapshot_id: managed_io_snapshot_id_from_arg(args.snapshot_id),
                            operation,
                        },
                    },
                );
                coordinator.release_reservation(reservation);
                record_managed_io_external_change_if_rejected(
                    &self.owned.managed_io_external_changes,
                    &workspace_identity,
                    &path,
                    &result,
                );
                record_managed_io_write_if_applied(
                    &self.owned.managed_io_external_changes,
                    provider_run.id(),
                    &workspace_identity,
                    &workspace_root,
                    &path,
                    &result,
                );
                let after = managed_io_result_applied(&result)
                    .then(|| managed_io_text_for_diff(&workspace_root, &path, true))
                    .flatten();
                let mut output = managed_io_edit_result(
                    result,
                    ManagedIoChangeContext {
                        path,
                        before,
                        after,
                    },
                    external_change_notice,
                );
                add_managed_io_workspace_payload(&mut output.payload, &workspace_context);
                Ok(output)
            }
            crate::transport::runtime_tools::APPLY_PATCH_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManagedApplyPatchArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_apply_patch",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain =
                    KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
                if domain != crate::io::ArtifactDomainKind::TextDocument {
                    return Err(DaemonError::LocalTransport {
                        operation: "runtime_tool_apply_patch",
                        message: "managed apply_patch currently supports only text artifacts"
                            .to_string(),
                    });
                }
                let operations = parse_managed_apply_patch(&args.patch_text)?;
                let mut output = apply_managed_patch_operations(
                    &mut coordinator,
                    workspace_identity,
                    workspace_root.clone(),
                    domain,
                    operations,
                    managed_io_reservation_owner(provider_run, tool_name),
                    &self.owned.managed_io_external_changes,
                )?;
                add_managed_io_workspace_payload(&mut output.payload, &workspace_context);
                Ok(output)
            }
            crate::transport::runtime_tools::DELETE_ARTIFACT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManagedDeleteArtifactArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_delete_artifact",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain =
                    KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
                let mut output = if domain == crate::io::ArtifactDomainKind::TextDocument {
                    apply_managed_patch_operations(
                        &mut coordinator,
                        workspace_identity,
                        workspace_root.clone(),
                        domain,
                        vec![ManagedPatchOperation::Delete {
                            path: PathBuf::from(args.path),
                        }],
                        managed_io_reservation_owner(provider_run, tool_name),
                        &self.owned.managed_io_external_changes,
                    )?
                } else {
                    apply_managed_whole_file_operations(
                        &mut coordinator,
                        workspace_identity,
                        workspace_root.clone(),
                        domain,
                        vec![ManagedWholeFileOperation::Delete {
                            path: PathBuf::from(args.path),
                        }],
                        managed_io_reservation_owner(provider_run, tool_name),
                        &self.owned.managed_io_external_changes,
                    )?
                };
                add_managed_io_workspace_payload(&mut output.payload, &workspace_context);
                Ok(output)
            }
            crate::transport::runtime_tools::MOVE_ARTIFACT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManagedMoveArtifactArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_move_artifact",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain =
                    KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
                let mut output = if domain == crate::io::ArtifactDomainKind::TextDocument {
                    apply_managed_patch_operations(
                        &mut coordinator,
                        workspace_identity,
                        workspace_root.clone(),
                        domain,
                        vec![ManagedPatchOperation::Move {
                            from_path: PathBuf::from(args.from_path),
                            to_path: PathBuf::from(args.to_path),
                            old_text: args.old_text,
                            new_text: args.new_text,
                        }],
                        managed_io_reservation_owner(provider_run, tool_name),
                        &self.owned.managed_io_external_changes,
                    )?
                } else {
                    if args.has_non_text_transform_fields() {
                        return Err(DaemonError::LocalTransport {
                            operation: "runtime_tool_move_artifact",
                            message: "non-text managed moves cannot transform content; omit old_text and new_text".to_string(),
                        });
                    }
                    apply_managed_whole_file_operations(
                        &mut coordinator,
                        workspace_identity,
                        workspace_root.clone(),
                        domain,
                        vec![ManagedWholeFileOperation::Move {
                            from_path: PathBuf::from(args.from_path),
                            to_path: PathBuf::from(args.to_path),
                        }],
                        managed_io_reservation_owner(provider_run, tool_name),
                        &self.owned.managed_io_external_changes,
                    )?
                };
                add_managed_io_workspace_payload(&mut output.payload, &workspace_context);
                Ok(output)
            }
            crate::transport::runtime_tools::WRITE_ARTIFACT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManagedWriteArtifactArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_write_artifact",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain =
                    KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
                let path = PathBuf::from(args.path.clone());
                let before = managed_io_text_for_diff(&workspace_root, &path, true);
                let content = managed_io_write_content_from_args(
                    "runtime_tool_write_artifact",
                    domain,
                    &args,
                )?;
                let reservation = match managed_io_try_reserve_ranges(
                    &mut coordinator,
                    &workspace_identity,
                    &path,
                    vec![crate::io::TextRange::new(0, usize::MAX)],
                    managed_io_reservation_owner(provider_run, tool_name),
                ) {
                    Ok(reservation) => reservation,
                    Err(mut output) => {
                        add_managed_io_workspace_payload(&mut output.payload, &workspace_context);
                        return Ok(output);
                    }
                };
                let external_change_notice = self
                    .owned
                    .managed_io_external_changes
                    .external_change_notice(&workspace_identity, &path);
                let result = crate::io::ManagedFileIo::apply_edit(
                    &mut coordinator,
                    crate::io::ManagedFileWriteRequest {
                        workspace_identity: workspace_identity.clone(),
                        workspace_root: workspace_root.clone(),
                        domain,
                        intent: crate::io::AgentEditIntent {
                            path: path.clone(),
                            snapshot_id: managed_io_write_snapshot_id_from_arg(
                                args.snapshot_id,
                                &path,
                            ),
                            operation: crate::io::AgentEditOperation::WriteArtifact { content },
                        },
                    },
                );
                coordinator.release_reservation(reservation);
                record_managed_io_external_change_if_rejected(
                    &self.owned.managed_io_external_changes,
                    &workspace_identity,
                    &path,
                    &result,
                );
                record_managed_io_write_if_applied(
                    &self.owned.managed_io_external_changes,
                    provider_run.id(),
                    &workspace_identity,
                    &workspace_root,
                    &path,
                    &result,
                );
                let after = managed_io_result_applied(&result)
                    .then(|| managed_io_text_for_diff(&workspace_root, &path, true))
                    .flatten();
                let mut output = managed_io_edit_result(
                    result,
                    ManagedIoChangeContext {
                        path,
                        before,
                        after,
                    },
                    external_change_notice,
                );
                add_managed_io_workspace_payload(&mut output.payload, &workspace_context);
                Ok(output)
            }
            other => Err(DaemonError::LocalTransport {
                operation: "dispatch_managed_io_runtime_tool_call",
                message: format!("unsupported managed I/O tool `{other}`"),
            }),
        }
    }

    async fn try_dispatch_remote_managed_io_runtime_tool_call(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<Option<crate::transport::runtime_tools::RuntimeToolResult>, DaemonError> {
        let workspace_context = self
            .managed_io_workspace_for_provider_run(provider_run)
            .await?;
        let remote_context = self
            .with_app_side_effect(|app| {
                crate::app::RemoteLeaseRuntime::new(app).leased_managed_io_context_for_provider_run(
                    provider_run.id(),
                    workspace_context.identity.clone(),
                )
            })
            .await;
        let Some(remote_context) = remote_context else {
            return Ok(None);
        };
        if !workspace_context.valid {
            return Ok(Some(managed_io_workspace_identity_rejected(
                &workspace_context,
            )));
        }
        let artifact_states = remote_managed_io_artifact_states_for_tool(
            &workspace_context.root,
            tool_name,
            &arguments,
        )?;
        let response = self
            .with_app_side_effect(|app| {
                app.block_on_relay_future(
                    crate::transport::relay_client::send_peer_request_via_temporary_connection(
                        app.config(),
                        ClientTarget {
                            daemon_id: Some(remote_context.home_kernel_id.clone()),
                            daemon_alias: None,
                        },
                        RelayPeerRequest::ForwardManagedIoRuntimeTool {
                            context: remote_context.clone(),
                            tool_name: tool_name.to_string(),
                            arguments: arguments.clone(),
                            artifact_states: artifact_states.clone(),
                        },
                    ),
                )
            })
            .await?;
        let (mut result, final_states) = match response {
            RelayPeerResponse::ManagedIoRuntimeToolHandled {
                result,
                final_artifact_states,
            } => (result, final_artifact_states),
            other => {
                return Err(DaemonError::LocalTransport {
                    operation: "forward leased managed I/O runtime tool",
                    message: format!("unexpected forwarded managed I/O response: {other:?}"),
                });
            }
        };
        if result.ok && !final_states.is_empty() {
            if let Some(rejection) = apply_remote_managed_io_final_states(
                &workspace_context.root,
                &artifact_states,
                &final_states,
            )? {
                result = rejection;
            }
        }
        add_managed_io_workspace_payload(&mut result.payload, &workspace_context);
        Ok(Some(result))
    }

    async fn managed_io_workspace_for_provider_run(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
    ) -> Result<ManagedIoWorkspaceContext, DaemonError> {
        let session = self
            .owned
            .session_store
            .get_session(provider_run.session_id())?;
        let workspace_root = provider_run
            .working_directory()
            .cloned()
            .unwrap_or_else(|| PathBuf::from(session.worktree_id()));
        let identity = workspace_identity_for_root_off_thread(workspace_root.clone()).await?;
        let snapshot = self.owned.workspace_identity_monitor.observe_provider_run(
            provider_run.id(),
            workspace_root.clone(),
            identity,
        );
        Ok(ManagedIoWorkspaceContext {
            root: workspace_root,
            identity: snapshot.current_identity,
            generation: snapshot.generation,
            identity_changed: snapshot.identity_changed,
            valid: snapshot.valid,
        })
    }

    pub(crate) async fn dispatch_forwarded_workflow_runtime_tool_call(
        &self,
        context: crate::execution_lease::RemoteWorkflowTurnContext,
        tool_name: String,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        {
            let owned = &self.owned;
            let home_session_id = context.home_session_id.clone();
            let home_agent_id = context.home_agent_id.clone();
            let canonical_tool_name = tool_name
                .strip_prefix("arroba_")
                .unwrap_or(&tool_name)
                .to_string();
            let context = owned.workflow_tool_context(
                context.home_session_id,
                context.workflow_run_id,
                context.workflow_node_run_id,
                Some(context.delivery_token),
            )?;
            let (result, dispatches) =
                owned.dispatch_workflow_runtime_tool_call(tool_name, arguments, context)?;
            self.spawn_workflow_prompt_dispatches(dispatches);
            if forwarded_workflow_tool_result_should_complete_home_prompt(
                &canonical_tool_name,
                &result,
            ) {
                if let Some(active_prompt) = owned.prompt_state_owner.active_prompt_for_agent(
                    &owned.session_store.get_session(&home_session_id)?,
                    &home_agent_id,
                ) {
                    let completion = owned.complete_remote_prompt_owner(
                        &home_session_id,
                        &home_agent_id,
                        "remote-provider-run-completed",
                        None,
                    )?;
                    if active_prompt.workflow_run_id().is_some() {
                        let dispatches = owned.workflow_complete_prompt(
                            &home_session_id,
                            &completion.completed,
                            Some("remote-provider-run-completed"),
                        )?;
                        self.spawn_workflow_prompt_dispatches(dispatches);
                    }
                }
            }
            Ok(result)
        }
    }

    pub(crate) async fn dispatch_forwarded_managed_io_runtime_tool_call(
        &self,
        context: crate::transport::relay_peer::RemoteManagedIoContext,
        tool_name: String,
        arguments: serde_json::Value,
        artifact_states: Vec<crate::transport::relay_peer::RemoteManagedIoArtifactState>,
    ) -> Result<
        (
            crate::transport::runtime_tools::RuntimeToolResult,
            Vec<crate::transport::relay_peer::RemoteManagedIoArtifactState>,
        ),
        DaemonError,
    > {
        let session = self
            .owned
            .session_store
            .get_session(&context.home_session_id)?;
        let home_root = PathBuf::from(session.worktree_id());
        let home_identity = workspace_identity_for_root_off_thread(home_root.clone()).await?;
        if !managed_io_workspace_identities_match(
            &home_identity,
            &context.worker_workspace_identity,
        ) {
            let result = crate::transport::runtime_tools::RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "applied": false,
                    "reason": {
                        "kind": "remote_workspace_not_coordinated",
                        "message": "The remote agent workspace does not match the home session repo/branch, so Arroba will not coordinate this managed I/O operation through the home kernel."
                    },
                    "next_action": "Move the remote agent to the same repo and branch as the home session, then retry through Arroba managed I/O.",
                }),
            };
            return Ok((result, Vec::new()));
        }
        let workspace_context = ManagedIoWorkspaceContext {
            root: home_root,
            identity: context.worker_workspace_identity.clone(),
            generation: 0,
            identity_changed: false,
            valid: true,
        };
        let mut coordinator = self.owned.managed_io_coordinator.lock().await;
        match tool_name.as_str() {
            crate::transport::runtime_tools::READ_ARTIFACT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManagedReadArtifactArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "forwarded_managed_io_read_artifact",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain =
                    KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
                let state =
                    remote_managed_io_state_for_path(&artifact_states, &PathBuf::from(&args.path))
                        .ok_or_else(|| DaemonError::LocalTransport {
                            operation: "forwarded_managed_io_read_artifact",
                            message: "missing forwarded artifact state".to_string(),
                        })?;
                let content = remote_managed_io_content_from_state(state, domain)?;
                let read = coordinator.read_artifact(crate::io::ArtifactReadRequest {
                    workspace_identity: context.worker_workspace_identity,
                    path: PathBuf::from(args.path),
                    domain,
                    content,
                });
                let mut payload = managed_io_read_payload(read);
                add_managed_io_workspace_payload(&mut payload, &workspace_context);
                Ok((
                    crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload },
                    Vec::new(),
                ))
            }
            crate::transport::runtime_tools::EDIT_ARTIFACT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManagedEditArtifactArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "forwarded_managed_io_edit_artifact",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain =
                    KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
                if domain != crate::io::ArtifactDomainKind::TextDocument {
                    return Err(DaemonError::LocalTransport {
                        operation: "forwarded_managed_io_edit_artifact",
                        message: "remote managed edit currently supports only text artifacts"
                            .to_string(),
                    });
                }
                let operation = managed_io_edit_operation_from_args(args.clone())?;
                let path = PathBuf::from(args.path.clone());
                let state =
                    remote_managed_io_state_for_path(&artifact_states, &path).ok_or_else(|| {
                        DaemonError::LocalTransport {
                            operation: "forwarded_managed_io_edit_artifact",
                            message: "missing forwarded artifact state".to_string(),
                        }
                    })?;
                let before = remote_managed_io_text_snapshot_from_state(state);
                coordinator.read_artifact(crate::io::ArtifactReadRequest {
                    workspace_identity: context.worker_workspace_identity.clone(),
                    path: path.clone(),
                    domain,
                    content: remote_managed_io_content_from_state(state, domain)?,
                });
                let reservation = match managed_io_try_reserve_ranges(
                    &mut coordinator,
                    &context.worker_workspace_identity,
                    &path,
                    managed_io_reservation_ranges_for_operation(
                        &operation,
                        before.as_ref(),
                        crate::io::TextRange::new(0, usize::MAX),
                    ),
                    crate::io::ArtifactReservationOwner::new(
                        format!("remote:{}", context.worker_provider_run_id),
                        Some(context.home_agent_id.clone()),
                        tool_name.clone(),
                    ),
                ) {
                    Ok(reservation) => reservation,
                    Err(mut result) => {
                        add_managed_io_workspace_payload(&mut result.payload, &workspace_context);
                        return Ok((result, Vec::new()));
                    }
                };
                let result = coordinator.apply_edit(crate::io::ArtifactWriteRequest {
                    workspace_identity: context.worker_workspace_identity,
                    intent: crate::io::AgentEditIntent {
                        path: path.clone(),
                        snapshot_id: managed_io_snapshot_id_from_arg(args.snapshot_id.clone()),
                        operation,
                    },
                });
                coordinator.release_reservation(reservation);
                let after = managed_io_result_applied(&result)
                    .then(|| {
                        let artifact_id =
                            coordinator.resolve_artifact_id(&workspace_context.identity, &path);
                        coordinator
                            .current_content(&artifact_id)
                            .and_then(|content| content.as_text().map(str::to_string))
                            .map(|text| ManagedIoTextSnapshot {
                                existed: true,
                                text,
                            })
                    })
                    .flatten();
                let final_states = after
                    .as_ref()
                    .map(|after| vec![remote_managed_io_state(&path, Some(after.text.clone()))])
                    .unwrap_or_default();
                let mut output = managed_io_edit_result(
                    result,
                    ManagedIoChangeContext {
                        path,
                        before,
                        after,
                    },
                    None,
                );
                add_managed_io_workspace_payload(&mut output.payload, &workspace_context);
                Ok((output, final_states))
            }
            crate::transport::runtime_tools::WRITE_ARTIFACT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManagedWriteArtifactArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "forwarded_managed_io_write_artifact",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain =
                    KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
                let path = PathBuf::from(args.path.clone());
                let state =
                    remote_managed_io_state_for_path(&artifact_states, &path).ok_or_else(|| {
                        DaemonError::LocalTransport {
                            operation: "forwarded_managed_io_write_artifact",
                            message: "missing forwarded artifact state".to_string(),
                        }
                    })?;
                let before = remote_managed_io_text_snapshot_from_state(state);
                coordinator.read_artifact(crate::io::ArtifactReadRequest {
                    workspace_identity: context.worker_workspace_identity.clone(),
                    path: path.clone(),
                    domain,
                    content: remote_managed_io_content_from_state(state, domain)?,
                });
                let reservation = match managed_io_try_reserve_ranges(
                    &mut coordinator,
                    &context.worker_workspace_identity,
                    &path,
                    vec![crate::io::TextRange::new(0, usize::MAX)],
                    crate::io::ArtifactReservationOwner::new(
                        format!("remote:{}", context.worker_provider_run_id),
                        Some(context.home_agent_id.clone()),
                        tool_name.clone(),
                    ),
                ) {
                    Ok(reservation) => reservation,
                    Err(mut result) => {
                        add_managed_io_workspace_payload(&mut result.payload, &workspace_context);
                        return Ok((result, Vec::new()));
                    }
                };
                let result = coordinator.apply_edit(crate::io::ArtifactWriteRequest {
                    workspace_identity: context.worker_workspace_identity,
                    intent: crate::io::AgentEditIntent {
                        path: path.clone(),
                        snapshot_id: managed_io_write_snapshot_id_from_arg(
                            args.snapshot_id.clone(),
                            &path,
                        ),
                        operation: crate::io::AgentEditOperation::WriteArtifact {
                            content: managed_io_write_content_from_args(
                                "forwarded_managed_io_write_artifact",
                                domain,
                                &args,
                            )?,
                        },
                    },
                });
                coordinator.release_reservation(reservation);
                let (after, final_states) = if managed_io_result_applied(&result) {
                    let artifact_id =
                        coordinator.resolve_artifact_id(&workspace_context.identity, &path);
                    let content = coordinator.current_content(&artifact_id).cloned();
                    let after = content.as_ref().and_then(|content| match content {
                        crate::io::ArtifactContent::Text(text) => Some(ManagedIoTextSnapshot {
                            existed: true,
                            text: text.clone(),
                        }),
                        crate::io::ArtifactContent::Bytes(_) => None,
                    });
                    let final_states = content
                        .map(|content| {
                            vec![remote_managed_io_state_from_content_with_domain(
                                &path,
                                Some(content),
                                domain,
                            )]
                        })
                        .unwrap_or_default();
                    (after, final_states)
                } else {
                    (None, Vec::new())
                };
                let mut output = managed_io_edit_result(
                    result,
                    ManagedIoChangeContext {
                        path,
                        before,
                        after,
                    },
                    None,
                );
                add_managed_io_workspace_payload(&mut output.payload, &workspace_context);
                Ok((output, final_states))
            }
            crate::transport::runtime_tools::APPLY_PATCH_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManagedApplyPatchArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "forwarded_managed_io_apply_patch",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain =
                    KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
                if domain != crate::io::ArtifactDomainKind::TextDocument {
                    return Err(DaemonError::LocalTransport {
                        operation: "forwarded_managed_io_apply_patch",
                        message:
                            "remote managed apply_patch currently supports only text artifacts"
                                .to_string(),
                    });
                }
                let operations = parse_managed_apply_patch(&args.patch_text)?;
                apply_remote_managed_patch_operations(
                    &mut coordinator,
                    context.worker_workspace_identity,
                    domain,
                    operations,
                    artifact_states,
                    crate::io::ArtifactReservationOwner::new(
                        format!("remote:{}", context.worker_provider_run_id),
                        Some(context.home_agent_id),
                        tool_name,
                    ),
                    &workspace_context,
                )
            }
            crate::transport::runtime_tools::DELETE_ARTIFACT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManagedDeleteArtifactArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "forwarded_managed_io_delete_artifact",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain =
                    KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
                if domain == crate::io::ArtifactDomainKind::TextDocument {
                    apply_remote_managed_patch_operations(
                        &mut coordinator,
                        context.worker_workspace_identity,
                        domain,
                        vec![ManagedPatchOperation::Delete {
                            path: PathBuf::from(args.path),
                        }],
                        artifact_states,
                        crate::io::ArtifactReservationOwner::new(
                            format!("remote:{}", context.worker_provider_run_id),
                            Some(context.home_agent_id),
                            tool_name,
                        ),
                        &workspace_context,
                    )
                } else {
                    apply_remote_managed_whole_file_operations(
                        &mut coordinator,
                        context.worker_workspace_identity,
                        domain,
                        vec![ManagedWholeFileOperation::Delete {
                            path: PathBuf::from(args.path),
                        }],
                        artifact_states,
                        crate::io::ArtifactReservationOwner::new(
                            format!("remote:{}", context.worker_provider_run_id),
                            Some(context.home_agent_id),
                            tool_name,
                        ),
                        &workspace_context,
                    )
                }
            }
            crate::transport::runtime_tools::MOVE_ARTIFACT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManagedMoveArtifactArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "forwarded_managed_io_move_artifact",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain =
                    KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
                if domain == crate::io::ArtifactDomainKind::TextDocument {
                    apply_remote_managed_patch_operations(
                        &mut coordinator,
                        context.worker_workspace_identity,
                        domain,
                        vec![ManagedPatchOperation::Move {
                            from_path: PathBuf::from(args.from_path),
                            to_path: PathBuf::from(args.to_path),
                            old_text: args.old_text,
                            new_text: args.new_text,
                        }],
                        artifact_states,
                        crate::io::ArtifactReservationOwner::new(
                            format!("remote:{}", context.worker_provider_run_id),
                            Some(context.home_agent_id),
                            tool_name,
                        ),
                        &workspace_context,
                    )
                } else {
                    if args.has_non_text_transform_fields() {
                        return Err(DaemonError::LocalTransport {
                            operation: "forwarded_managed_io_move_artifact",
                            message: "non-text managed moves cannot transform content; omit old_text and new_text".to_string(),
                        });
                    }
                    apply_remote_managed_whole_file_operations(
                        &mut coordinator,
                        context.worker_workspace_identity,
                        domain,
                        vec![ManagedWholeFileOperation::Move {
                            from_path: PathBuf::from(args.from_path),
                            to_path: PathBuf::from(args.to_path),
                        }],
                        artifact_states,
                        crate::io::ArtifactReservationOwner::new(
                            format!("remote:{}", context.worker_provider_run_id),
                            Some(context.home_agent_id),
                            tool_name,
                        ),
                        &workspace_context,
                    )
                }
            }
            _ => Ok((
                crate::transport::runtime_tools::RuntimeToolResult {
                    ok: false,
                    payload: serde_json::json!({
                        "applied": false,
                        "reason": {
                            "kind": "unsupported_remote_managed_io_tool",
                            "message": format!("remote coordinated managed I/O does not yet support `{tool_name}`")
                        },
                        "next_action": "Use arroba.read_artifact, arroba.edit_artifact, or arroba.write_artifact for remote coordinated text edits until patch/move/delete remote routing lands.",
                    }),
                },
                Vec::new(),
            )),
        }
    }
}

enum PromptAbortDispatchOutcome {
    Done,
    Retry,
}

pub(crate) struct CapabilityRuntimeSnapshot {
    pub(crate) workspace_id: String,
    pub(crate) worktree_root: std::path::PathBuf,
    pub(crate) workspace_coordinator: crate::runtime::workspace_coordinator::WorkspaceCoordinator,
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

#[cfg(test)]
mod managed_io_external_change_notice_tests;
