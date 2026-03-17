use crate::app::DaemonApp;
use crate::attachment::{AttachRequest, AttachmentMode, ClientCapabilityLevel, RuntimeAttachment};
use crate::error::DaemonError;
use crate::provider::{LaunchProviderRequest, RuntimeProviderRun};
use crate::session::{CreateSessionRequest, RuntimeSession};
use crate::terminal::TerminalOutputRecord;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachToSessionRequest {
    pub session_id: String,
    pub client_id: String,
    pub capability_level: ClientCapabilityLevel,
    pub mode: AttachmentMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchProviderRunRequest {
    pub session_id: String,
    pub adapter_key: String,
    pub provider: String,
    pub account_profile: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetachFromSessionRequest {
    pub attachment_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendTerminalInputRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResizeTerminalRequest {
    pub session_id: String,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PumpTerminalOutputRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndSessionRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalDaemonRequest {
    CreateSession(CreateSessionRequest),
    AttachToSession(AttachToSessionRequest),
    DetachFromSession(DetachFromSessionRequest),
    LaunchProviderRun(LaunchProviderRunRequest),
    SendTerminalInput(SendTerminalInputRequest),
    ResizeTerminal(ResizeTerminalRequest),
    PumpTerminalOutput(PumpTerminalOutputRequest),
    EndSession(EndSessionRequest),
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
    TerminalInputAccepted {
        session_id: String,
        attachment_id: String,
    },
    TerminalResized {
        session_id: String,
        cols: u16,
        rows: u16,
    },
    TerminalOutput {
        records: Vec<TerminalOutputRecord>,
    },
    SessionEnded {
        session: RuntimeSession,
    },
}

impl DaemonApp {
    pub fn handle_local_request(
        &mut self,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        match request {
            LocalDaemonRequest::CreateSession(request) => Ok(LocalDaemonResponse::SessionCreated {
                session: self.sessions_mut().create_session(request)?,
            }),
            LocalDaemonRequest::AttachToSession(request) => {
                Ok(LocalDaemonResponse::SessionAttached {
                    attachment: self.attach(AttachRequest::new(
                        request.session_id,
                        request.client_id,
                        request.capability_level,
                        request.mode,
                    ))?,
                })
            }
            LocalDaemonRequest::DetachFromSession(request) => {
                Ok(LocalDaemonResponse::SessionDetached {
                    attachment: self.detach(&request.attachment_id)?,
                })
            }
            LocalDaemonRequest::LaunchProviderRun(request) => {
                Ok(LocalDaemonResponse::ProviderRunLaunched {
                    provider_run: self.launch_provider(LaunchProviderRequest::new(
                        request.session_id,
                        request.adapter_key,
                        request.provider,
                        request.account_profile,
                        request.model,
                    ))?,
                })
            }
            LocalDaemonRequest::SendTerminalInput(request) => {
                self.send_terminal_input(
                    &request.session_id,
                    &request.attachment_id,
                    &request.bytes,
                )?;

                Ok(LocalDaemonResponse::TerminalInputAccepted {
                    session_id: request.session_id,
                    attachment_id: request.attachment_id,
                })
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
                    records: self.pump_terminal_output(&request.session_id)?,
                })
            }
            LocalDaemonRequest::EndSession(request) => Ok(LocalDaemonResponse::SessionEnded {
                session: self.end_session(&request.session_id)?,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::attachment::{AttachmentMode, ClientCapabilityLevel};
    use crate::session::CreateSessionRequest;
    use crate::{DaemonApp, DaemonConfig, DaemonError};

    use super::{
        AttachToSessionRequest, DetachFromSessionRequest, EndSessionRequest,
        LaunchProviderRunRequest, LocalDaemonRequest, LocalDaemonResponse,
        SendTerminalInputRequest,
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
                    mode: AttachmentMode::Controller,
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
    fn local_request_api_rejects_terminal_input_without_active_provider_run() {
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

        let controller = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "client-1".to_string(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                    mode: AttachmentMode::Controller,
                },
            ))
            .expect("attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };

        let error = app
            .handle_local_request(LocalDaemonRequest::SendTerminalInput(
                SendTerminalInputRequest {
                    session_id: session.id().to_string(),
                    attachment_id: controller.id().to_string(),
                    bytes: b"whoami\n".to_vec(),
                },
            ))
            .expect_err("terminal input should fail without an active provider run");

        match error {
            DaemonError::NoActiveProviderRun { session_id } => {
                assert_eq!(session_id, session.id());
            }
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
                assert_eq!(adapter_key, "missing-adapter");
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn local_request_api_rejects_terminal_input_from_non_controller() {
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

        let _controller = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "controller".to_string(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                    mode: AttachmentMode::Controller,
                },
            ))
            .expect("controller attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };

        let observer = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "observer".to_string(),
                    capability_level: ClientCapabilityLevel::InteractiveStructured,
                    mode: AttachmentMode::Observer,
                },
            ))
            .expect("observer attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };

        let _run = match app
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

        let error = app
            .handle_local_request(LocalDaemonRequest::SendTerminalInput(
                SendTerminalInputRequest {
                    session_id: session.id().to_string(),
                    attachment_id: observer.id().to_string(),
                    bytes: b"pwd\n".to_vec(),
                },
            ))
            .expect_err("non-controller terminal input should be rejected");

        match error {
            DaemonError::AttachmentIsNotController {
                session_id,
                attachment_id,
            } => {
                assert_eq!(session_id, session.id());
                assert_eq!(attachment_id, observer.id());
            }
            other => panic!("unexpected error: {other}"),
        }
    }
}
