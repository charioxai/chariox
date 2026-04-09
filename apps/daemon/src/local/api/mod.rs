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
    fn local_api_session_snapshot(&self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        let mut session = self.sessions().get_session(session_id)?;
        let agents = self.agents().get_session_agents(session_id);
        session.set_agents(agents);
        Ok(session)
    }

    pub fn handle_local_request(
        &mut self,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        match request {
            LocalDaemonRequest::CreateSession(request) => {
                let (mut session, agent) = self.create_session(request)?;
                // Populate agents list
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
                let mut session = self.sessions().get_session(&request.session_id)?;
                // Populate agents list from agent service
                let agents = self.agents().get_session_agents(&request.session_id);
                session.set_agents(agents);
                Ok(LocalDaemonResponse::SessionState { session })
            }
            LocalDaemonRequest::GetProviderRun(request) => {
                self.handle_get_provider_run_request(request)
            }
            LocalDaemonRequest::GetProviderCatalog(_) => self.handle_get_provider_catalog_request(),
            LocalDaemonRequest::GetProviderCommandCatalogs(_) => {
                self.handle_get_provider_command_catalogs_request()
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
                let mut session = self.sessions().get_session(&request.session_id)?;
                session.set_agents(self.agents().get_session_agents(&request.session_id));
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
                let mut session = self.sessions().get_session(&session_id)?;
                session.set_agents(self.agents().get_session_agents(&session_id));
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
                let agent = self.spawn_agent(create_request)?;
                Ok(LocalDaemonResponse::AgentSpawned { agent })
            }
            LocalDaemonRequest::DestroyAgent(request) => {
                let agent = self.destroy_agent(&request.agent_id)?;
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
            LocalDaemonRequest::CreateWorkflow(request) => {
                let workflow = self
                    .sessions_mut()
                    .create_workflow(&request.session_id, request.alias)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowCreated { workflow, session })
            }
            LocalDaemonRequest::AliasWorkflow(request) => {
                let workflow = self.sessions_mut().assign_workflow_alias(
                    &request.session_id,
                    &request.workflow_ref,
                    request.alias,
                )?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowAliased { workflow, session })
            }
            LocalDaemonRequest::ListWorkflows(request) => {
                Ok(LocalDaemonResponse::WorkflowsListed {
                    workflows: self.sessions().list_workflows(&request.session_id)?,
                })
            }
            LocalDaemonRequest::ResolveWorkflow(request) => {
                Ok(LocalDaemonResponse::WorkflowResolved {
                    workflow: self
                        .sessions()
                        .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?,
                })
            }
            LocalDaemonRequest::CreateWorkflowEndpoint(request) => {
                let endpoint = self.sessions_mut().create_workflow_endpoint(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.entry_node_id,
                    request.alias,
                )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowEndpointCreated {
                    endpoint,
                    workflow,
                    session,
                })
            }
            LocalDaemonRequest::AliasWorkflowEndpoint(request) => {
                let endpoint = self.sessions_mut().assign_workflow_endpoint_alias(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.endpoint_ref,
                    request.alias,
                )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowEndpointAliased {
                    endpoint,
                    workflow,
                    session,
                })
            }
            LocalDaemonRequest::BindWorkflowEndpoint(request) => {
                let endpoint = self.sessions_mut().bind_workflow_endpoint(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.endpoint_ref,
                    &request.entry_node_id,
                )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowEndpointBound {
                    endpoint,
                    workflow,
                    session,
                })
            }
            LocalDaemonRequest::AddWorkflowNode(request) => {
                let agent_exists = self
                    .agents()
                    .get_session_agents(&request.session_id)
                    .into_iter()
                    .any(|agent| agent.id() == request.agent_id);
                if !agent_exists {
                    return Err(DaemonError::AgentNotFound {
                        agent_id: request.agent_id,
                    });
                }
                let node = self.sessions_mut().add_workflow_node(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.agent_id,
                )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowNodeAdded {
                    node,
                    workflow,
                    session,
                })
            }
            LocalDaemonRequest::RemoveWorkflowNode(request) => {
                let node = self.sessions_mut().remove_workflow_node(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.node_id,
                )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowNodeRemoved {
                    node,
                    workflow,
                    session,
                })
            }
            LocalDaemonRequest::UpdateWorkflowNodeInstructions(request) => {
                let node = self.sessions_mut().update_workflow_node_instructions(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.node_id,
                    request.instructions.clone(),
                )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowNodeInstructionsUpdated {
                    node,
                    workflow,
                    session,
                })
            }
            LocalDaemonRequest::SetWorkflowNodeCanCompleteRun(request) => {
                let node = self.sessions_mut().set_workflow_node_can_complete_run(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.node_id,
                    request.can_complete_workflow_run,
                )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowNodeCanCompleteRunUpdated {
                    node,
                    workflow,
                    session,
                })
            }
            LocalDaemonRequest::SetWorkflowNodeCanEmitIntermediateOutput(request) => {
                let node = self
                    .sessions_mut()
                    .set_workflow_node_can_emit_intermediate_output(
                        &request.session_id,
                        &request.workflow_ref,
                        &request.node_id,
                        request.can_emit_intermediate_workflow_run_output,
                    )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(
                    LocalDaemonResponse::WorkflowNodeCanEmitIntermediateOutputUpdated {
                        node,
                        workflow,
                        session,
                    },
                )
            }
            LocalDaemonRequest::SetWorkflowNodeIntermediateOutputSchema(request) => {
                let node = self
                    .sessions_mut()
                    .set_workflow_node_intermediate_output_schema_ref(
                        &request.session_id,
                        &request.workflow_ref,
                        &request.node_id,
                        request.intermediate_output_schema_ref.clone(),
                    )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(
                    LocalDaemonResponse::WorkflowNodeIntermediateOutputSchemaUpdated {
                        node,
                        workflow,
                        session,
                    },
                )
            }
            LocalDaemonRequest::SetWorkflowNodeMaxTurns(request) => {
                let node = self.sessions_mut().set_workflow_node_max_turns(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.node_id,
                    request.max_turns,
                )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowNodeMaxTurnsUpdated {
                    node,
                    workflow,
                    session,
                })
            }
            LocalDaemonRequest::AddWorkflowEdge(request) => {
                let edge = self.sessions_mut().add_workflow_edge(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.from_node_id,
                    &request.to_node_id,
                    request.output_schema_ref.clone(),
                    request.validation_policy,
                )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowEdgeAdded {
                    edge,
                    workflow,
                    session,
                })
            }
            LocalDaemonRequest::RemoveWorkflowEdge(request) => {
                let edge = self.sessions_mut().remove_workflow_edge(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.edge_id,
                )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowEdgeRemoved {
                    edge,
                    workflow,
                    session,
                })
            }
            LocalDaemonRequest::InvokeWorkflowEndpoint(request) => {
                let outcome = self.invoke_workflow_endpoint_with_admission(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.endpoint_ref,
                    request.prompt,
                )?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
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
            LocalDaemonRequest::ListWorkflowRuns(request) => {
                Ok(LocalDaemonResponse::WorkflowRunsListed {
                    workflow_runs: self
                        .sessions()
                        .list_workflow_runs(&request.session_id, request.workflow_ref.as_deref())?,
                })
            }
            LocalDaemonRequest::GetWorkflowRun(request) => Ok(LocalDaemonResponse::WorkflowRun {
                workflow_run: self
                    .sessions()
                    .resolve_workflow_run_ref(&request.session_id, &request.workflow_run_ref)?,
            }),
            LocalDaemonRequest::CancelWorkflowRun(request) => {
                let workflow_run_id = self
                    .sessions()
                    .resolve_workflow_run_ref(&request.session_id, &request.workflow_run_ref)?
                    .id()
                    .to_string();
                let should_cancel_active_prompt = self
                    .sessions()
                    .get_session(&request.session_id)?
                    .active_prompt()
                    .and_then(|prompt| prompt.workflow_run_id())
                    == Some(workflow_run_id.as_str());
                if should_cancel_active_prompt {
                    let _ = crate::transport::TransportService::cancel_active_prompt_for_runtime(
                        self,
                        &request.session_id,
                    )?;
                }
                let workflow_run = self
                    .sessions_mut()
                    .cancel_workflow_run(&request.session_id, &request.workflow_run_ref)?;
                let _ = self.drain_session_workflow_launch_queue(&request.session_id)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowRunCancelled {
                    workflow_run,
                    session,
                })
            }
            LocalDaemonRequest::ResumeWorkflowRun(request) => {
                let workflow_run = crate::scheduler::runtime::resume_workflow_run(
                    self,
                    &request.session_id,
                    &request.workflow_run_ref,
                )?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowRunResumed {
                    workflow_run,
                    session,
                })
            }
            LocalDaemonRequest::CreateWorkflowWatchdog(request) => {
                let watchdog = self.sessions_mut().create_workflow_watchdog(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.endpoint_ref,
                    request.interval_seconds,
                    request.invocation_prompt,
                    request.policy,
                    if request.max_wakeups_configured {
                        Some(request.max_wakeups)
                    } else {
                        None
                    },
                )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let endpoint = self.sessions().resolve_workflow_endpoint_ref(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.endpoint_ref,
                )?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowWatchdogCreated {
                    watchdog,
                    workflow,
                    endpoint,
                    session,
                })
            }
            LocalDaemonRequest::ListWorkflowWatchdogs(request) => {
                Ok(LocalDaemonResponse::WorkflowWatchdogsListed {
                    watchdogs: self.sessions().list_workflow_watchdogs(
                        &request.session_id,
                        request.workflow_ref.as_deref(),
                    )?,
                })
            }
            LocalDaemonRequest::SetWorkflowWatchdogEnabled(request) => {
                let watchdog = self.sessions_mut().set_workflow_watchdog_enabled(
                    &request.session_id,
                    &request.watchdog_ref,
                    request.enabled,
                )?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowWatchdogUpdated { watchdog, session })
            }
            LocalDaemonRequest::RemoveWorkflowWatchdog(request) => {
                let watchdog = self
                    .sessions_mut()
                    .remove_workflow_watchdog(&request.session_id, &request.watchdog_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowWatchdogRemoved { watchdog, session })
            }
            LocalDaemonRequest::SetWorkflowFlushContext(request) => {
                let workflow = self
                    .sessions_mut()
                    .set_workflow_flush_agent_context_before_run(
                        &request.session_id,
                        &request.workflow_ref,
                        request.flush_agent_context_before_run,
                    )?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowFlushContextUpdated { workflow, session })
            }
            LocalDaemonRequest::SetWorkflowRunOutputSchema(request) => {
                let workflow = self.sessions_mut().set_workflow_run_output_schema_ref(
                    &request.session_id,
                    &request.workflow_ref,
                    request.run_output_schema_ref.clone(),
                )?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowRunOutputSchemaUpdated { workflow, session })
            }
            LocalDaemonRequest::SetWorkflowIntermediateOutputSchema(request) => {
                let workflow = self
                    .sessions_mut()
                    .set_workflow_intermediate_output_schema_ref(
                        &request.session_id,
                        &request.workflow_ref,
                        request.intermediate_output_schema_ref.clone(),
                    )?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(
                    LocalDaemonResponse::WorkflowIntermediateOutputSchemaUpdated {
                        workflow,
                        session,
                    },
                )
            }
            LocalDaemonRequest::SetWorkflowLaunchPolicy(request) => {
                let session = self
                    .sessions_mut()
                    .set_workflow_launch_policy(&request.session_id, request.policy)?;
                let mut session = session;
                session.set_agents(self.agents().get_session_agents(&request.session_id));
                Ok(LocalDaemonResponse::WorkflowLaunchPolicyUpdated { session })
            }
            LocalDaemonRequest::ListQueuedWorkflowLaunches(request) => {
                Ok(LocalDaemonResponse::QueuedWorkflowLaunchesListed {
                    queued_launches: self
                        .sessions()
                        .list_queued_workflow_launches(&request.session_id)?,
                })
            }
            LocalDaemonRequest::RemoveQueuedWorkflowLaunch(request) => {
                let queued_launch = self
                    .sessions_mut()
                    .remove_queued_workflow_launch(&request.session_id, &request.queue_item_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::QueuedWorkflowLaunchRemoved {
                    queued_launch,
                    session,
                })
            }
            LocalDaemonRequest::ClearQueuedWorkflowLaunches(request) => {
                let queued_launches = self
                    .sessions_mut()
                    .clear_queued_workflow_launches(&request.session_id)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::QueuedWorkflowLaunchesCleared {
                    queued_launches,
                    session,
                })
            }
            LocalDaemonRequest::ValidateWorkflowOutput(request) => {
                let result = crate::transport::runtime_tools::dispatch_runtime_tool_call(
                    self,
                    crate::transport::runtime_tools::RuntimeToolCall {
                        tool_name: crate::transport::runtime_tools::VALIDATE_WORKFLOW_OUTPUT_TOOL
                            .to_string(),
                        arguments: serde_json::json!({
                            "output_schema_ref": request.output_schema_ref,
                            "output_json": request.output_json,
                        }),
                        context: crate::transport::runtime_tools::WorkflowRuntimeToolContext {
                            session_id: request.session_id.clone(),
                            workflow_run_ref: String::new(),
                            workflow_node_run_id: String::new(),
                            delivery_token: None,
                            allowed_output_schema_refs: vec![request.output_schema_ref.clone()],
                            workflow_run_output_schema_ref: None,
                            workflow_intermediate_output_schema_ref: None,
                            can_complete_workflow_run: false,
                            can_emit_intermediate_workflow_run_output: false,
                        },
                    },
                )?;
                Ok(LocalDaemonResponse::WorkflowOutputValidated {
                    valid: result.payload["valid"].as_bool().unwrap_or(false),
                    warning: result.payload["warning"]
                        .as_str()
                        .map(str::to_string)
                        .filter(|value| !value.is_empty()),
                })
            }
            LocalDaemonRequest::AckWorkflowTurn(request) => {
                crate::transport::runtime_tools::dispatch_runtime_tool_call(
                    self,
                    crate::transport::runtime_tools::RuntimeToolCall {
                        tool_name: crate::transport::runtime_tools::ACK_WORKFLOW_TURN_TOOL
                            .to_string(),
                        arguments: serde_json::json!({
                            "delivery_token": request.delivery_token,
                        }),
                        context: crate::transport::runtime_tools::WorkflowRuntimeToolContext {
                            session_id: request.session_id.clone(),
                            workflow_run_ref: request.workflow_run_ref.clone(),
                            workflow_node_run_id: request.workflow_node_run_id.clone(),
                            delivery_token: Some(request.delivery_token.clone()),
                            allowed_output_schema_refs: Vec::new(),
                            workflow_run_output_schema_ref: None,
                            workflow_intermediate_output_schema_ref: None,
                            can_complete_workflow_run: false,
                            can_emit_intermediate_workflow_run_output: false,
                        },
                    },
                )?;
                let workflow_run = self
                    .sessions()
                    .resolve_workflow_run_ref(&request.session_id, &request.workflow_run_ref)?
                    .clone();
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowTurnAcknowledged {
                    workflow_run,
                    session,
                })
            }
        }
    }
}
