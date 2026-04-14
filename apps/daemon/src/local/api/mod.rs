use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::agent::AgentInstance;
use crate::app::{DaemonApp, SessionHistoryCursor, SessionHistoryPageEntry};
use crate::attachment::{AttachRequest, ClientCapabilityLevel, RuntimeAttachment};
use crate::capability::{
    CaptureScreenshotResult, EditFileResult, InspectGitResult, ReadDirectoryTreeResult,
    ReadFileResult, RunShellCommandRequest, RunShellCommandResult, StoredTransferArtifact,
};
use crate::error::DaemonError;
use crate::kernel::projection::DaemonHealthProjection;
use crate::provider::{
    OpenCodeProviderCatalog, ProviderAuthStatus, ProviderCommandCatalog, ProviderLoginStart,
    ProviderProcessInfo, RuntimeProviderRun,
};
use crate::session::{
    CreateSessionRequest, PromptAttachment, PromptCancellation, PromptCompletion,
    PromptSubmissionOutcome, QueuedWorkflowLaunch, RuntimeSession, SessionConfigState,
    WorkflowDefinition, WorkflowEdgeDefinition, WorkflowEndpointDefinition, WorkflowLaunchPolicy,
    WorkflowNodeDefinition, WorkflowRun, WorkflowWatchdogDefinition, WorkflowWatchdogPolicy,
};

#[cfg(test)]
mod tests;
mod types;

pub use types::*;

impl DaemonApp {
    pub(crate) fn create_session_response(
        &mut self,
        request: CreateSessionRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let (mut session, agent) = self.create_session(request)?;
        let agents = self.agents().get_session_agents(session.id());
        session.set_agents(agents);
        crate::logging::info_with_fields(
            "daemon.session",
            "session created with default agent",
            serde_json::json!({
                "session_id": session.id(),
                "session_alias": session.alias(),
                "workspace_id": session.workspace_id(),
                "worktree_id": session.worktree_id(),
                "execution_mode": format!("{:?}", session.execution_mode()),
                "agent_id": agent.id(),
                "agent_ref": agent.agent_ref(),
            }),
        );
        Ok(LocalDaemonResponse::SessionCreated { session, agent })
    }

    pub(crate) fn local_api_session_snapshot(
        &self,
        session_id: &str,
    ) -> Result<RuntimeSession, DaemonError> {
        let mut session = self.sessions().get_session(session_id)?;
        let agents = self.agents().get_session_agents(session_id);
        session.set_agents(agents);
        self.project_session_runtime_view(&mut session);
        self.update_session_projection(session.clone());
        Ok(session)
    }

    pub fn handle_local_request(
        &mut self,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        match request {
            LocalDaemonRequest::CreateSession(request) => self.create_session_response(request),
            LocalDaemonRequest::AttachToSession(request) => {
                Ok(LocalDaemonResponse::SessionAttached {
                    attachment: self.attach(AttachRequest::new(
                        request.session_id,
                        request.client_id,
                        request.capability_level,
                    ))?,
                })
            }
            LocalDaemonRequest::DetachFromSession(request) => {
                Ok(LocalDaemonResponse::SessionDetached {
                    attachment: self.detach(&request.attachment_id)?,
                })
            }
            LocalDaemonRequest::LaunchProviderRun(request) => {
                self.handle_launch_provider_run_request(request)
            }
            LocalDaemonRequest::ListSessions(_) => {
                let sessions = self.sessions().list_sessions();
                // Populate agents for each session
                let sessions_with_agents: Vec<_> = sessions
                    .into_iter()
                    .map(|mut session| {
                        let agents = self.agents().get_session_agents(session.id());
                        session.set_agents(agents);
                        session
                    })
                    .collect();
                Ok(LocalDaemonResponse::SessionsListed {
                    sessions: sessions_with_agents,
                })
            }
            LocalDaemonRequest::ResolveSession(request) => {
                let mut session = self
                    .resolve_session_ref(&request.session_ref, request.workspace_id.as_deref())?;
                // Populate agents list
                let agents = self.agents().get_session_agents(session.id());
                session.set_agents(agents);
                Ok(LocalDaemonResponse::SessionResolved { session })
            }
            LocalDaemonRequest::GetSessionState(request) => {
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::SessionState { session })
            }
            LocalDaemonRequest::GetDaemonHealth(_) => Ok(LocalDaemonResponse::DaemonHealth {
                projection: DaemonHealthProjection::new(
                    0,
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    self.provider_run_operation_lanes().queue_snapshots(),
                    self.provider_run_operation_lanes().health_snapshot(),
                    Default::default(),
                    self.session_state_projection_store().health_snapshot(),
                    self.agent_runtime_projection_store().health_snapshot(),
                    self.provider_catalog_projection_store().health_snapshot(
                        crate::local::provider_requests::PROVIDER_CATALOG_CACHE_TTL,
                    ),
                    self.transport_health_store().snapshot(
                        crate::kernel_transport::RECENT_EVENT_LIMIT,
                        crate::kernel_transport::COMMAND_RESULT_CACHE_LIMIT,
                        crate::kernel_transport::INBOUND_REQUEST_LIMIT,
                    ),
                    self.terminal().health_snapshot(),
                    self.session_state_projection_store()
                        .workspace_coordination_snapshot(
                            self.workspace_coordinator().active_claims(),
                        ),
                    self.session_state_projection_store()
                        .invariant_snapshot(&self.agent_runtime_projection_store()),
                ),
            }),
            LocalDaemonRequest::GetProviderRun(request) => {
                self.handle_get_provider_run_request(request)
            }
            LocalDaemonRequest::GetProviderCatalog(_) => self.handle_get_provider_catalog_request(),
            LocalDaemonRequest::GetProviderCommandCatalogs(_) => {
                self.handle_get_provider_command_catalogs_request()
            }
            LocalDaemonRequest::RelayStatus(_) => self.handle_relay_status_request(),
            LocalDaemonRequest::ConfigureRelay(request) => {
                self.handle_configure_relay_request(request)
            }
            LocalDaemonRequest::ListRemoteMachines(_) => self.handle_list_remote_machines_request(),
            LocalDaemonRequest::ListRemoteMachineKernels(request) => {
                self.handle_list_remote_machine_kernels_request(request)
            }
            LocalDaemonRequest::ApproveRemoteMachine(request) => {
                self.handle_approve_remote_machine_request(request)
            }
            LocalDaemonRequest::ForgetRemoteMachine(request) => {
                self.handle_forget_remote_machine_request(request)
            }
            LocalDaemonRequest::RenameRemoteMachine(request) => {
                self.handle_rename_remote_machine_request(request)
            }
            LocalDaemonRequest::GetProviderAuthStatus(request) => {
                self.handle_get_provider_auth_status_request(request)
            }
            LocalDaemonRequest::StartProviderLogin(request) => {
                self.handle_start_provider_login_request(request)
            }
            LocalDaemonRequest::LogoutProvider(request) => {
                self.handle_logout_provider_request(request)
            }
            LocalDaemonRequest::ListProviderProcesses(request) => {
                Ok(LocalDaemonResponse::ProviderProcessesListed {
                    processes: self.list_provider_processes(request.provider.as_deref())?,
                })
            }
            LocalDaemonRequest::TeardownProviderProcesses(request) => {
                Ok(LocalDaemonResponse::ProviderProcessesTornDown {
                    processes: self.teardown_provider_processes(request.provider.as_deref())?,
                })
            }
            LocalDaemonRequest::GetSessionHistory(request) => {
                let page = self.session_history_page(
                    &request.session_id,
                    request.agent_id.as_deref(),
                    request.round_count,
                    request.max_chars,
                    request.before_entry_index,
                    request.before_entry_char_offset,
                )?;
                Ok(LocalDaemonResponse::SessionHistory {
                    entries: page.entries,
                    next_cursor: page.next_cursor,
                })
            }
            LocalDaemonRequest::PollRuntimeNotices(request) => {
                let _ =
                    self.ensure_attachment_in_session(&request.session_id, &request.attachment_id)?;
                Ok(LocalDaemonResponse::RuntimeNotices {
                    notices: self
                        .terminal_mut()
                        .drain_notice_records(&request.session_id, &request.attachment_id),
                })
            }
            LocalDaemonRequest::SubmitPrompt(request) => {
                let outcome = if request.target_agent_id.is_some() {
                    crate::transport::TransportService::schedule_direct_prompt_to_agent(
                        self,
                        &request.session_id,
                        &request.attachment_id,
                        request.target_agent_id.as_deref(),
                        &request.prompt,
                        request.attachments,
                    )?
                } else {
                    crate::transport::TransportService::schedule_direct_prompt(
                        self,
                        &request.session_id,
                        &request.attachment_id,
                        &request.prompt,
                        request.attachments,
                    )?
                };
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::PromptSubmitted { outcome, session })
            }
            LocalDaemonRequest::CompletePrompt(request) => {
                Ok(LocalDaemonResponse::PromptCompleted {
                    completion: crate::transport::TransportService::complete_active_prompt(
                        self,
                        &request.session_id,
                    )?,
                })
            }
            LocalDaemonRequest::CancelActivePrompt(request) => {
                Ok(LocalDaemonResponse::PromptCancelled {
                    cancellation: crate::transport::TransportService::cancel_active_prompt(
                        self,
                        &request.session_id,
                        &request.attachment_id,
                    )?,
                })
            }
            LocalDaemonRequest::UpdateSessionConfig(request) => {
                let session_id = request.session_id.clone();
                let config = self.update_session_config(
                    &request.session_id,
                    &request.attachment_id,
                    request.values,
                    request.requires_idle,
                )?;
                let session = self.local_api_session_snapshot(&session_id)?;
                Ok(LocalDaemonResponse::SessionConfigUpdated { config, session })
            }
            LocalDaemonRequest::ResizeTerminal(request) => {
                self.resize_terminal(&request.session_id, request.cols, request.rows)?;
                Ok(LocalDaemonResponse::TerminalResized {
                    session_id: request.session_id,
                    cols: request.cols,
                    rows: request.rows,
                })
            }
            LocalDaemonRequest::PumpTerminalOutput(request) => {
                Ok(LocalDaemonResponse::TerminalOutput {
                    records: self
                        .pump_terminal_output(&request.session_id, &request.attachment_id)?,
                })
            }
            LocalDaemonRequest::RunShellCommand(request) => {
                Ok(LocalDaemonResponse::ShellCommandCompleted {
                    result: self.run_shell_command(
                        RunShellCommandRequest::new(
                            request.session_id,
                            request.attachment_id,
                            request.command,
                            request.args,
                            PathBuf::new(),
                            request.working_directory,
                        )
                        .with_timeout_ms(request.timeout_ms.unwrap_or(5_000)),
                    )?,
                })
            }
            LocalDaemonRequest::ReadDirectoryTree(request) => {
                Ok(LocalDaemonResponse::DirectoryTreeRead {
                    result: self.read_directory_tree(
                        &request.session_id,
                        &request.attachment_id,
                        request.path,
                        request.max_depth,
                    )?,
                })
            }
            LocalDaemonRequest::ReadFile(request) => Ok(LocalDaemonResponse::FileRead {
                result: self.read_file(
                    &request.session_id,
                    &request.attachment_id,
                    request.path,
                )?,
            }),
            LocalDaemonRequest::EditFile(request) => Ok(LocalDaemonResponse::FileEdited {
                result: self.edit_file(
                    &request.session_id,
                    &request.attachment_id,
                    request.path,
                    request.contents,
                )?,
            }),
            LocalDaemonRequest::InspectGit(request) => Ok(LocalDaemonResponse::GitInspected {
                result: self.inspect_git(
                    &request.session_id,
                    &request.attachment_id,
                    request.working_directory,
                )?,
            }),
            LocalDaemonRequest::CaptureScreenshot(request) => {
                Ok(LocalDaemonResponse::ScreenshotCaptured {
                    result: self.capture_screenshot(&request.session_id, &request.attachment_id)?,
                })
            }
            LocalDaemonRequest::StoreTransferredFile(request) => {
                Ok(LocalDaemonResponse::FileTransferred {
                    result: self.store_transferred_file(
                        &request.session_id,
                        &request.attachment_id,
                        request.source_path,
                        request.display_name,
                    )?,
                })
            }
            LocalDaemonRequest::EndSession(request) => Ok(LocalDaemonResponse::SessionEnded {
                session: self.end_session(&request.session_id)?,
            }),
            LocalDaemonRequest::DeleteSession(request) => Ok(LocalDaemonResponse::SessionDeleted {
                session: self
                    .delete_session_ref(&request.session_ref, request.workspace_id.as_deref())?,
            }),
            LocalDaemonRequest::AliasSession(request) => {
                let _session = self
                    .sessions_mut()
                    .assign_session_alias(&request.session_id, request.alias)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::SessionAliased { session })
            }
            LocalDaemonRequest::SpawnAgent(request) => {
                let create_request =
                    crate::agent::CreateAgentRequest::new(&request.session_id, &request.provider);
                let create_request = if let Some(alias) = request.alias {
                    create_request.with_alias(alias)
                } else {
                    create_request
                };
                let create_request = if let Some(model) = request.model {
                    create_request.with_model(model)
                } else {
                    create_request
                };
                let create_request = if let Some(effort) = request.effort {
                    create_request.with_effort(effort)
                } else {
                    create_request
                };
                let create_request = if let Some(worktree_id) = request.worktree_id {
                    create_request.with_worktree(worktree_id)
                } else {
                    create_request
                };
                let create_request = if let Some(machine_ref) = request.machine_ref {
                    create_request.with_machine(machine_ref)
                } else {
                    create_request
                };
                let agent = self.spawn_agent(create_request)?;
                let _ = self.local_api_session_snapshot(agent.session_id())?;
                Ok(LocalDaemonResponse::AgentSpawned { agent })
            }
            LocalDaemonRequest::DestroyAgent(request) => {
                let agent = self.destroy_agent(&request.agent_id)?;
                let _ = self.local_api_session_snapshot(agent.session_id())?;
                Ok(LocalDaemonResponse::AgentDestroyed { agent })
            }
            LocalDaemonRequest::FocusAgent(request) => {
                let agent = self.focus_agent(&request.session_id, &request.agent_id)?;
                Ok(LocalDaemonResponse::AgentFocused { agent })
            }
            LocalDaemonRequest::CycleAgentFocus(request) => {
                let agent = self.cycle_agent_focus(&request.session_id)?;
                Ok(LocalDaemonResponse::AgentFocusCycled { agent })
            }
            LocalDaemonRequest::ListAgents(request) => {
                let agents = self.list_session_agents(&request.session_id);
                Ok(LocalDaemonResponse::AgentsListed { agents })
            }
            request @ (LocalDaemonRequest::CreateWorkflow(_)
            | LocalDaemonRequest::AliasWorkflow(_)
            | LocalDaemonRequest::ListWorkflows(_)
            | LocalDaemonRequest::ResolveWorkflow(_)
            | LocalDaemonRequest::CreateWorkflowEndpoint(_)
            | LocalDaemonRequest::AliasWorkflowEndpoint(_)
            | LocalDaemonRequest::BindWorkflowEndpoint(_)
            | LocalDaemonRequest::AddWorkflowNode(_)
            | LocalDaemonRequest::RemoveWorkflowNode(_)
            | LocalDaemonRequest::UpdateWorkflowNodeInstructions(_)
            | LocalDaemonRequest::SetWorkflowNodeCanCompleteRun(_)
            | LocalDaemonRequest::SetWorkflowNodeCanEmitIntermediateOutput(_)
            | LocalDaemonRequest::SetWorkflowNodeIntermediateOutputSchema(_)
            | LocalDaemonRequest::SetWorkflowNodeMaxTurns(_)
            | LocalDaemonRequest::AddWorkflowEdge(_)
            | LocalDaemonRequest::RemoveWorkflowEdge(_)
            | LocalDaemonRequest::SetWorkflowRunOutputSchema(_)
            | LocalDaemonRequest::SetWorkflowIntermediateOutputSchema(_)
            | LocalDaemonRequest::SetWorkflowFlushContext(_)
            | LocalDaemonRequest::SetWorkflowLaunchPolicy(_)
            | LocalDaemonRequest::InvokeWorkflowEndpoint(_)
            | LocalDaemonRequest::ListWorkflowRuns(_)
            | LocalDaemonRequest::GetWorkflowRun(_)
            | LocalDaemonRequest::AckWorkflowTurn(_)
            | LocalDaemonRequest::ValidateWorkflowOutput(_)
            | LocalDaemonRequest::CancelWorkflowRun(_)
            | LocalDaemonRequest::ResumeWorkflowRun(_)
            | LocalDaemonRequest::ClearQueuedWorkflowLaunches(_)
            | LocalDaemonRequest::RemoveQueuedWorkflowLaunch(_)
            | LocalDaemonRequest::ListQueuedWorkflowLaunches(_)
            | LocalDaemonRequest::CreateWorkflowWatchdog(_)
            | LocalDaemonRequest::RemoveWorkflowWatchdog(_)
            | LocalDaemonRequest::SetWorkflowWatchdogEnabled(_)
            | LocalDaemonRequest::ListWorkflowWatchdogs(_)) => {
                self.handle_workflow_request(request)
            }
        }
    }
}
