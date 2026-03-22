use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::app::{DaemonApp, SessionHistoryCursor, SessionHistoryPageEntry};
use crate::attachment::{AttachRequest, ClientCapabilityLevel, RuntimeAttachment};
use crate::capability::{
    CaptureScreenshotResult, EditFileResult, InspectGitResult, ReadDirectoryTreeResult,
    ReadFileResult, RunShellCommandRequest, RunShellCommandResult, StoredTransferArtifact,
};
use crate::error::DaemonError;
use crate::provider::{LaunchProviderRequest, RuntimeProviderRun};
use crate::session::{
    CreateSessionRequest, PromptCancellation, PromptCompletion, PromptSubmissionOutcome,
    RuntimeSession, SessionConfigState,
};
use crate::terminal::{RuntimeNoticeRecord, TerminalOutputRecord};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachToSessionRequest {
    pub session_id: String,
    pub client_id: String,
    pub capability_level: ClientCapabilityLevel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchProviderRunRequest {
    pub session_id: String,
    pub adapter_key: String,
    pub provider: String,
    pub account_profile: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetachFromSessionRequest {
    pub attachment_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitPromptRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletePromptRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelActivePromptRequest {
    pub session_id: String,
    pub attachment_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateSessionConfigRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub values: BTreeMap<String, String>,
    pub requires_idle: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetSessionStateRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetProviderRunRequest {
    pub provider_run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListSessionsRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveSessionRequest {
    pub session_ref: String,
    pub workspace_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetSessionHistoryRequest {
    pub session_id: String,
    pub round_count: Option<usize>,
    pub max_chars: Option<usize>,
    pub before_entry_index: Option<usize>,
    pub before_entry_char_offset: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollRuntimeNoticesRequest {
    pub session_id: String,
    pub attachment_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResizeTerminalRequest {
    pub session_id: String,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PumpTerminalOutputRequest {
    pub session_id: String,
    pub attachment_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndSessionRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteSessionRequest {
    pub session_ref: String,
    pub workspace_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunShellCapabilityRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub command: String,
    pub args: Vec<String>,
    pub working_directory: Option<PathBuf>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadDirectoryTreeCapabilityRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub path: Option<PathBuf>,
    pub max_depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadFileCapabilityRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditFileCapabilityRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub path: PathBuf,
    pub contents: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InspectGitCapabilityRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub working_directory: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureScreenshotCapabilityRequest {
    pub session_id: String,
    pub attachment_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreTransferredFileCapabilityRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub source_path: PathBuf,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocalDaemonRequest {
    CreateSession(CreateSessionRequest),
    AttachToSession(AttachToSessionRequest),
    DetachFromSession(DetachFromSessionRequest),
    LaunchProviderRun(LaunchProviderRunRequest),
    ListSessions(ListSessionsRequest),
    ResolveSession(ResolveSessionRequest),
    GetSessionState(GetSessionStateRequest),
    GetProviderRun(GetProviderRunRequest),
    GetSessionHistory(GetSessionHistoryRequest),
    PollRuntimeNotices(PollRuntimeNoticesRequest),
    SubmitPrompt(SubmitPromptRequest),
    CompletePrompt(CompletePromptRequest),
    CancelActivePrompt(CancelActivePromptRequest),
    UpdateSessionConfig(UpdateSessionConfigRequest),
    ResizeTerminal(ResizeTerminalRequest),
    PumpTerminalOutput(PumpTerminalOutputRequest),
    RunShellCommand(RunShellCapabilityRequest),
    ReadDirectoryTree(ReadDirectoryTreeCapabilityRequest),
    ReadFile(ReadFileCapabilityRequest),
    EditFile(EditFileCapabilityRequest),
    InspectGit(InspectGitCapabilityRequest),
    CaptureScreenshot(CaptureScreenshotCapabilityRequest),
    StoreTransferredFile(StoreTransferredFileCapabilityRequest),
    EndSession(EndSessionRequest),
    DeleteSession(DeleteSessionRequest),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocalDaemonResponse {
    SessionCreated {
        session: RuntimeSession,
    },
    SessionAttached {
        attachment: RuntimeAttachment,
    },
    SessionDetached {
        attachment: RuntimeAttachment,
    },
    ProviderRunLaunched {
        provider_run: RuntimeProviderRun,
    },
    SessionsListed {
        sessions: Vec<RuntimeSession>,
    },
    SessionResolved {
        session: RuntimeSession,
    },
    SessionState {
        session: RuntimeSession,
    },
    ProviderRun {
        provider_run: RuntimeProviderRun,
    },
    SessionHistory {
        entries: Vec<SessionHistoryPageEntry>,
        next_cursor: Option<SessionHistoryCursor>,
    },
    RuntimeNotices {
        notices: Vec<RuntimeNoticeRecord>,
    },
    PromptSubmitted {
        outcome: PromptSubmissionOutcome,
        session: RuntimeSession,
    },
    PromptCompleted {
        completion: PromptCompletion,
    },
    PromptCancelled {
        cancellation: PromptCancellation,
    },
    SessionConfigUpdated {
        config: SessionConfigState,
        session: RuntimeSession,
    },
    TerminalResized {
        session_id: String,
        cols: u16,
        rows: u16,
    },
    TerminalOutput {
        records: Vec<TerminalOutputRecord>,
    },
    ShellCommandCompleted {
        result: RunShellCommandResult,
    },
    DirectoryTreeRead {
        result: ReadDirectoryTreeResult,
    },
    FileRead {
        result: ReadFileResult,
    },
    FileEdited {
        result: EditFileResult,
    },
    GitInspected {
        result: InspectGitResult,
    },
    ScreenshotCaptured {
        result: CaptureScreenshotResult,
    },
    FileTransferred {
        result: StoredTransferArtifact,
    },
    SessionEnded {
        session: RuntimeSession,
    },
    SessionDeleted {
        session: RuntimeSession,
    },
}

impl DaemonApp {
    pub fn handle_local_request(
        &mut self,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        match request {
            LocalDaemonRequest::CreateSession(request) => {
                let session = self.sessions_mut().create_session(request)?;
                crate::logging::info_with_fields(
                    "daemon.session",
                    "session created",
                    serde_json::json!({
                        "session_id": session.id(),
                        "session_alias": session.alias(),
                        "workspace_id": session.workspace_id(),
                        "worktree_id": session.worktree_id(),
                        "execution_mode": format!("{:?}", session.execution_mode()),
                    }),
                );
                Ok(LocalDaemonResponse::SessionCreated { session })
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
                let provider_run = self.launch_provider(LaunchProviderRequest::new(
                    request.session_id,
                    request.adapter_key,
                    request.provider,
                    request.account_profile,
                    request.model,
                ))?;
                crate::logging::debug_with_fields(
                    "daemon.local_api",
                    "returning launched provider run to client",
                    serde_json::json!({
                        "provider_run_id": provider_run.id(),
                        "session_id": provider_run.session_id(),
                        "provider": provider_run.provider(),
                        "model": provider_run.model(),
                        "state": provider_run.state().to_string(),
                    }),
                );
                Ok(LocalDaemonResponse::ProviderRunLaunched { provider_run })
            }
            LocalDaemonRequest::ListSessions(_) => Ok(LocalDaemonResponse::SessionsListed {
                sessions: self.sessions().list_sessions(),
            }),
            LocalDaemonRequest::ResolveSession(request) => {
                Ok(LocalDaemonResponse::SessionResolved {
                    session: self.resolve_session_ref(
                        &request.session_ref,
                        request.workspace_id.as_deref(),
                    )?,
                })
            }
            LocalDaemonRequest::GetSessionState(request) => Ok(LocalDaemonResponse::SessionState {
                session: self.sessions().get_session(&request.session_id)?,
            }),
            LocalDaemonRequest::GetProviderRun(request) => {
                let provider_run = self.providers().get_run(&request.provider_run_id)?;
                crate::logging::debug_with_fields(
                    "daemon.local_api",
                    "returning provider run lookup to client",
                    serde_json::json!({
                        "provider_run_id": provider_run.id(),
                        "session_id": provider_run.session_id(),
                        "provider": provider_run.provider(),
                        "model": provider_run.model(),
                        "state": provider_run.state().to_string(),
                    }),
                );
                Ok(LocalDaemonResponse::ProviderRun { provider_run })
            }
            LocalDaemonRequest::GetSessionHistory(request) => {
                let page = self.session_history_page(
                    &request.session_id,
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
                let outcome = self.submit_prompt(
                    &request.session_id,
                    &request.attachment_id,
                    &request.prompt,
                )?;
                let session = self.sessions().get_session(&request.session_id)?;
                Ok(LocalDaemonResponse::PromptSubmitted { outcome, session })
            }
            LocalDaemonRequest::CompletePrompt(request) => {
                Ok(LocalDaemonResponse::PromptCompleted {
                    completion: self.complete_active_prompt(&request.session_id)?,
                })
            }
            LocalDaemonRequest::CancelActivePrompt(request) => {
                Ok(LocalDaemonResponse::PromptCancelled {
                    cancellation: self
                        .cancel_active_prompt(&request.session_id, &request.attachment_id)?,
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
                let session = self.sessions().get_session(&session_id)?;
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
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::attachment::ClientCapabilityLevel;
    use crate::session::{CreateSessionRequest, PromptSubmissionOutcome};
    use crate::{DaemonApp, DaemonConfig, DaemonError};

    use super::{
        AttachToSessionRequest, CancelActivePromptRequest, CaptureScreenshotCapabilityRequest,
        CompletePromptRequest, DeleteSessionRequest, DetachFromSessionRequest,
        EditFileCapabilityRequest, EndSessionRequest, GetSessionStateRequest,
        InspectGitCapabilityRequest, LaunchProviderRunRequest, LocalDaemonRequest,
        LocalDaemonResponse, PollRuntimeNoticesRequest, ReadDirectoryTreeCapabilityRequest,
        ReadFileCapabilityRequest, ResolveSessionRequest, RunShellCapabilityRequest,
        StoreTransferredFileCapabilityRequest, SubmitPromptRequest, UpdateSessionConfigRequest,
    };

    #[test]
    fn local_request_api_supports_session_attach_and_end() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-1", "worktree-1"),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session } => session,
            _ => panic!("unexpected local response"),
        };

        let attachment = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "client-1".to_string(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                },
            ))
            .expect("attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };

        let detached = match app
            .handle_local_request(LocalDaemonRequest::DetachFromSession(
                DetachFromSessionRequest {
                    attachment_id: attachment.id().to_string(),
                },
            ))
            .expect("detach should succeed")
        {
            LocalDaemonResponse::SessionDetached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };

        let ended = match app
            .handle_local_request(LocalDaemonRequest::EndSession(EndSessionRequest {
                session_id: session.id().to_string(),
            }))
            .expect("end session should succeed")
        {
            LocalDaemonResponse::SessionEnded { session } => session,
            _ => panic!("unexpected local response"),
        };

        assert_eq!(detached.id(), attachment.id());
        assert_eq!(ended.id(), session.id());
        assert!(app.attachments().get_attachment(detached.id()).is_err());
    }

    #[test]
    fn local_request_api_resolves_and_deletes_sessions_by_ref() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-1", "worktree-1").with_alias("main"),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session } => session,
            _ => panic!("unexpected local response"),
        };

        let resolved = match app
            .handle_local_request(LocalDaemonRequest::ResolveSession(ResolveSessionRequest {
                session_ref: "mai".to_string(),
                workspace_id: Some("workspace-1".to_string()),
            }))
            .expect("resolve should succeed")
        {
            LocalDaemonResponse::SessionResolved { session } => session,
            _ => panic!("unexpected local response"),
        };

        let deleted = match app
            .handle_local_request(LocalDaemonRequest::DeleteSession(DeleteSessionRequest {
                session_ref: session.id()[..8].to_string(),
                workspace_id: Some("workspace-1".to_string()),
            }))
            .expect("delete should succeed")
        {
            LocalDaemonResponse::SessionDeleted { session } => session,
            _ => panic!("unexpected local response"),
        };

        assert_eq!(resolved.id(), session.id());
        assert_eq!(deleted.id(), session.id());
        assert_eq!(deleted.alias(), Some("main"));
        assert_eq!(deleted.status(), crate::session::SessionStatus::Ended);
    }

    #[test]
    fn detaching_one_attachment_keeps_the_session_open_for_others() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-1", "worktree-1"),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session } => session,
            _ => panic!("unexpected local response"),
        };

        let first = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "client-1".to_string(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                },
            ))
            .expect("first attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };

        let second = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "client-2".to_string(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                },
            ))
            .expect("second attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };

        let detached = match app
            .handle_local_request(LocalDaemonRequest::DetachFromSession(
                DetachFromSessionRequest {
                    attachment_id: first.id().to_string(),
                },
            ))
            .expect("detach should succeed")
        {
            LocalDaemonResponse::SessionDetached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };

        let state = match app
            .handle_local_request(LocalDaemonRequest::GetSessionState(
                GetSessionStateRequest {
                    session_id: session.id().to_string(),
                },
            ))
            .expect("state request should succeed")
        {
            LocalDaemonResponse::SessionState { session } => session,
            _ => panic!("unexpected local response"),
        };

        assert_eq!(detached.id(), first.id());
        assert_eq!(state.status().to_string(), "created");
        assert_eq!(state.attachment_ids().len(), 1);
        assert!(state.has_attachment(second.id()));
        assert!(app.attachments().get_attachment(second.id()).is_ok());
    }

    #[test]
    fn local_request_api_rejects_prompt_without_active_provider_run() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-1", "worktree-1"),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session } => session,
            _ => panic!("unexpected local response"),
        };
        let attachment = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "client-1".to_string(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                },
            ))
            .expect("attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };

        let error = app
            .handle_local_request(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                prompt: "whoami".to_string(),
            }))
            .expect_err("prompt submit should fail without active provider run");

        match error {
            DaemonError::NoActiveProviderRun { session_id } => assert_eq!(session_id, session.id()),
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn local_request_api_rejects_invalid_provider_adapter() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-1", "worktree-1"),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session } => session,
            _ => panic!("unexpected local response"),
        };

        let error = app
            .handle_local_request(LocalDaemonRequest::LaunchProviderRun(
                LaunchProviderRunRequest {
                    session_id: session.id().to_string(),
                    adapter_key: "missing-adapter".to_string(),
                    provider: "claude-code".to_string(),
                    account_profile: "default".to_string(),
                    model: "sonnet".to_string(),
                },
            ))
            .expect_err("unknown adapters should be rejected");

        match error {
            DaemonError::ProviderAdapterNotFound { adapter_key } => {
                assert_eq!(adapter_key, "missing-adapter")
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn local_request_api_exposes_queue_config_and_notices() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-1", "worktree-1"),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session } => session,
            _ => panic!("unexpected local response"),
        };
        let a = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "client-a".to_string(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                },
            ))
            .expect("attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };
        let b = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "client-b".to_string(),
                    capability_level: ClientCapabilityLevel::InteractiveStructured,
                },
            ))
            .expect("attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };
        let _ = app
            .handle_local_request(LocalDaemonRequest::LaunchProviderRun(
                LaunchProviderRunRequest {
                    session_id: session.id().to_string(),
                    adapter_key: "dev-stub".to_string(),
                    provider: "claude-code".to_string(),
                    account_profile: "default".to_string(),
                    model: "sonnet".to_string(),
                },
            ))
            .expect("provider launch should succeed");

        let first = app
            .handle_local_request(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: session.id().to_string(),
                attachment_id: a.id().to_string(),
                prompt: "first".to_string(),
            }))
            .expect("first prompt should start");
        let second = app
            .handle_local_request(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: session.id().to_string(),
                attachment_id: b.id().to_string(),
                prompt: "second".to_string(),
            }))
            .expect("second prompt should queue");
        let config = app
            .handle_local_request(LocalDaemonRequest::UpdateSessionConfig(
                UpdateSessionConfigRequest {
                    session_id: session.id().to_string(),
                    attachment_id: a.id().to_string(),
                    values: BTreeMap::from([("theme".to_string(), "compact".to_string())]),
                    requires_idle: false,
                },
            ))
            .expect("config update should succeed");

        match first {
            LocalDaemonResponse::PromptSubmitted {
                outcome: PromptSubmissionOutcome::Started { .. },
                session,
            } => {
                assert!(session.active_prompt().is_some());
            }
            _ => panic!("unexpected first prompt response"),
        }
        match second {
            LocalDaemonResponse::PromptSubmitted {
                outcome: PromptSubmissionOutcome::Queued { .. },
                session,
            } => {
                assert_eq!(session.queued_prompts().len(), 1);
            }
            _ => panic!("unexpected second prompt response"),
        }
        match config {
            LocalDaemonResponse::SessionConfigUpdated { config, session } => {
                assert_eq!(config.version(), 1);
                assert_eq!(session.config_state().version(), 1);
            }
            _ => panic!("unexpected config response"),
        }

        let notices = app
            .handle_local_request(LocalDaemonRequest::PollRuntimeNotices(
                PollRuntimeNoticesRequest {
                    session_id: session.id().to_string(),
                    attachment_id: b.id().to_string(),
                },
            ))
            .expect("notice polling should succeed");
        match notices {
            LocalDaemonResponse::RuntimeNotices { notices } => assert!(!notices.is_empty()),
            _ => panic!("unexpected notices response"),
        }

        let state = app
            .handle_local_request(LocalDaemonRequest::GetSessionState(
                GetSessionStateRequest {
                    session_id: session.id().to_string(),
                },
            ))
            .expect("state request should succeed");
        match state {
            LocalDaemonResponse::SessionState { session } => {
                assert_eq!(session.queued_prompts().len(), 1);
                assert_eq!(session.config_state().version(), 1);
            }
            _ => panic!("unexpected state response"),
        }

        let completed = app
            .handle_local_request(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
                session_id: session.id().to_string(),
            }))
            .expect("complete prompt should succeed");
        match completed {
            LocalDaemonResponse::PromptCompleted { completion } => {
                assert!(completion.started_next.is_some())
            }
            _ => panic!("unexpected completion response"),
        }
    }

    #[test]
    fn local_request_api_can_cancel_an_active_prompt() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");

        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-1", "worktree-1"),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session } => session,
            _ => panic!("unexpected local response"),
        };

        let attachment = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "client-a".to_string(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                },
            ))
            .expect("attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };

        let _provider_run = match app
            .handle_local_request(LocalDaemonRequest::LaunchProviderRun(
                LaunchProviderRunRequest {
                    session_id: session.id().to_string(),
                    adapter_key: "dev-stub".to_string(),
                    provider: "claude-code".to_string(),
                    account_profile: "default".to_string(),
                    model: "sonnet".to_string(),
                },
            ))
            .expect("provider launch should succeed")
        {
            LocalDaemonResponse::ProviderRunLaunched { provider_run } => provider_run,
            _ => panic!("unexpected local response"),
        };

        let _ = app
            .handle_local_request(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                prompt: "first prompt\n".to_string(),
            }))
            .expect("first prompt should start");
        let _ = app
            .handle_local_request(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                prompt: "second prompt\n".to_string(),
            }))
            .expect("second prompt should queue");

        let response = app
            .handle_local_request(LocalDaemonRequest::CancelActivePrompt(
                CancelActivePromptRequest {
                    session_id: session.id().to_string(),
                    attachment_id: attachment.id().to_string(),
                },
            ))
            .expect("cancel should succeed");

        match response {
            LocalDaemonResponse::PromptCancelled { cancellation } => {
                assert_eq!(
                    cancellation.prompt.status(),
                    crate::session::PromptStatus::Cancelling
                );
                assert!(cancellation.started_next.is_none());
            }
            _ => panic!("unexpected local response"),
        }
    }

    #[test]
    fn local_request_api_runs_shell_command_capability() {
        let worktree_root = std::env::temp_dir().join("arroba-shell-local-api-test");
        std::fs::create_dir_all(&worktree_root).expect("worktree dir should exist");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-1", worktree_root.display().to_string()),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session } => session,
            _ => panic!("unexpected local response"),
        };
        let attachment = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "client-shell".to_string(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                },
            ))
            .expect("attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };

        let response = app
            .handle_local_request(LocalDaemonRequest::RunShellCommand(
                RunShellCapabilityRequest {
                    session_id: session.id().to_string(),
                    attachment_id: attachment.id().to_string(),
                    command: "/bin/sh".to_string(),
                    args: vec!["-lc".to_string(), "printf capability".to_string()],
                    working_directory: None,
                    timeout_ms: None,
                },
            ))
            .expect("shell capability should succeed");

        match response {
            LocalDaemonResponse::ShellCommandCompleted { result } => {
                assert_eq!(result.exit_code, 0);
                assert_eq!(result.stdout, "capability");
            }
            _ => panic!("unexpected shell response"),
        }
    }

    #[test]
    fn local_request_api_rejects_shell_command_for_unauthorized_attachment() {
        let worktree_root = std::env::temp_dir().join("arroba-shell-local-api-denied-test");
        std::fs::create_dir_all(&worktree_root).expect("worktree dir should exist");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-1", worktree_root.display().to_string()),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session } => session,
            _ => panic!("unexpected local response"),
        };
        let attachment = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "client-automation".to_string(),
                    capability_level: ClientCapabilityLevel::AutomationOnly,
                },
            ))
            .expect("attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };

        let error = app
            .handle_local_request(LocalDaemonRequest::RunShellCommand(
                RunShellCapabilityRequest {
                    session_id: session.id().to_string(),
                    attachment_id: attachment.id().to_string(),
                    command: "/bin/sh".to_string(),
                    args: vec!["-lc".to_string(), "printf denied".to_string()],
                    working_directory: None,
                    timeout_ms: None,
                },
            ))
            .expect_err("automation-only attachment should not run shell commands");

        match error {
            DaemonError::AttachmentCapabilityDenied { session_id, .. } => {
                assert_eq!(session_id, session.id());
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn local_request_api_rejects_file_capability_for_unauthorized_attachment() {
        let worktree_root = std::env::temp_dir().join("arroba-file-local-api-denied-test");
        let _ = std::fs::remove_dir_all(&worktree_root);
        std::fs::create_dir_all(&worktree_root).expect("worktree dir should exist");
        std::fs::write(worktree_root.join("notes.txt"), "hello").expect("file should exist");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-1", worktree_root.display().to_string()),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session } => session,
            _ => panic!("unexpected local response"),
        };
        let attachment = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "client-automation".to_string(),
                    capability_level: ClientCapabilityLevel::AutomationOnly,
                },
            ))
            .expect("attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };

        let error = app
            .handle_local_request(LocalDaemonRequest::ReadFile(ReadFileCapabilityRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                path: worktree_root.join("notes.txt"),
            }))
            .expect_err("automation-only attachment should not read files");

        match error {
            DaemonError::AttachmentCapabilityDenied { session_id, .. } => {
                assert_eq!(session_id, session.id());
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn local_request_api_reads_directory_tree_file_and_git_status() {
        let worktree_root = std::env::temp_dir().join("arroba-capability-local-api-test");
        let _ = std::fs::remove_dir_all(&worktree_root);
        std::fs::create_dir_all(worktree_root.join("src")).expect("worktree should exist");
        std::fs::write(worktree_root.join("README.md"), "hello").expect("file should exist");
        std::fs::write(worktree_root.join("src/lib.rs"), "before").expect("file should exist");
        std::process::Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(&worktree_root)
            .output()
            .expect("git init should work");

        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-1", worktree_root.display().to_string()),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session } => session,
            _ => panic!("unexpected local response"),
        };
        let attachment = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "client-capability".to_string(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                },
            ))
            .expect("attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };

        let tree = app
            .handle_local_request(LocalDaemonRequest::ReadDirectoryTree(
                ReadDirectoryTreeCapabilityRequest {
                    session_id: session.id().to_string(),
                    attachment_id: attachment.id().to_string(),
                    path: None,
                    max_depth: 2,
                },
            ))
            .expect("tree read should succeed");
        let file = app
            .handle_local_request(LocalDaemonRequest::ReadFile(ReadFileCapabilityRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                path: worktree_root.join("src/lib.rs"),
            }))
            .expect("file read should succeed");
        let edit = app
            .handle_local_request(LocalDaemonRequest::EditFile(EditFileCapabilityRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                path: worktree_root.join("src/lib.rs"),
                contents: "after".to_string(),
            }))
            .expect("file edit should succeed");
        let git = app
            .handle_local_request(LocalDaemonRequest::InspectGit(
                InspectGitCapabilityRequest {
                    session_id: session.id().to_string(),
                    attachment_id: attachment.id().to_string(),
                    working_directory: None,
                },
            ))
            .expect("git inspect should succeed");

        match tree {
            LocalDaemonResponse::DirectoryTreeRead { result } => {
                assert!(result
                    .entries
                    .iter()
                    .any(|entry| entry.relative_path == "README.md"));
            }
            _ => panic!("unexpected tree response"),
        }
        match file {
            LocalDaemonResponse::FileRead { result } => assert_eq!(result.contents, "before"),
            _ => panic!("unexpected file response"),
        }
        match edit {
            LocalDaemonResponse::FileEdited { result } => {
                assert_eq!(result.bytes_written, 5);
                assert_eq!(result.old_size, 6);
                assert_eq!(result.new_size, 5);
                assert!(result.changed);
            }
            _ => panic!("unexpected edit response"),
        }
        match git {
            LocalDaemonResponse::GitInspected { result } => assert!(result.status.contains("main")),
            _ => panic!("unexpected git response"),
        }
    }

    #[test]
    fn local_request_api_returns_structured_screenshot_unavailable_result() {
        std::env::set_var("ARROBA_SCREENSHOT_DISABLE", "1");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new(
                    "workspace-1",
                    std::env::temp_dir().display().to_string(),
                ),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session } => session,
            _ => panic!("unexpected local response"),
        };
        let attachment = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "client-screenshot".to_string(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                },
            ))
            .expect("attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };

        let response = app
            .handle_local_request(LocalDaemonRequest::CaptureScreenshot(
                CaptureScreenshotCapabilityRequest {
                    session_id: session.id().to_string(),
                    attachment_id: attachment.id().to_string(),
                },
            ))
            .expect("screenshot request should succeed with unavailable result");
        std::env::remove_var("ARROBA_SCREENSHOT_DISABLE");

        match response {
            LocalDaemonResponse::ScreenshotCaptured { result } => {
                assert_eq!(
                    result.status,
                    crate::capability::ScreenshotStatus::Unavailable
                );
            }
            _ => panic!("unexpected screenshot response"),
        }
    }

    #[test]
    fn local_request_api_stores_transferred_file_under_session_artifacts() {
        let worktree_root = std::env::temp_dir().join("arroba-transfer-local-api-test");
        let _ = std::fs::remove_dir_all(&worktree_root);
        std::fs::create_dir_all(&worktree_root).expect("worktree should exist");
        let source = worktree_root.join("artifact.txt");
        std::fs::write(&source, "artifact").expect("file should exist");

        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-1", worktree_root.display().to_string()),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session } => session,
            _ => panic!("unexpected local response"),
        };
        let attachment = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "client-transfer".to_string(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                },
            ))
            .expect("attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };

        let response = app
            .handle_local_request(LocalDaemonRequest::StoreTransferredFile(
                StoreTransferredFileCapabilityRequest {
                    session_id: session.id().to_string(),
                    attachment_id: attachment.id().to_string(),
                    source_path: source,
                    display_name: None,
                },
            ))
            .expect("transfer should succeed");

        match response {
            LocalDaemonResponse::FileTransferred { result } => {
                assert!(result
                    .stored_path
                    .to_string_lossy()
                    .contains("arroba-session-artifacts"));
                assert_eq!(result.bytes, 8);
            }
            _ => panic!("unexpected transfer response"),
        }
    }
}
