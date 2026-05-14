use std::sync::Arc;

use tokio::sync::Mutex;

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, QueryHistoryRequest};
use crate::runtime::command::{KernelCallerKind, KernelCommand};
use crate::runtime::projection::SessionStateProjectionStore;
use crate::session::DEFAULT_LOCAL_USER_ID;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SessionMembershipScope {
    AllSessions,
    SessionId(String),
    SessionRef {
        session_ref: String,
        workspace_id: Option<String>,
    },
    AttachmentId(String),
}

pub(crate) fn command_session_user_id(command: &KernelCommand) -> Option<String> {
    match command.caller.caller_kind {
        KernelCallerKind::LocalClient => command
            .caller
            .user_id
            .clone()
            .or_else(|| Some(DEFAULT_LOCAL_USER_ID.to_string())),
        KernelCallerKind::RemoteClient
        | KernelCallerKind::RemoteKernel
        | KernelCallerKind::HostedService => command.caller.user_id.clone(),
    }
}

pub(crate) fn is_implicit_local_session_caller(command: &KernelCommand) -> bool {
    matches!(command.caller.caller_kind, KernelCallerKind::LocalClient)
        && command.caller.user_id.is_none()
}

pub(crate) async fn authorize_session_membership(
    app: &Arc<Mutex<DaemonApp>>,
    session_projection: &SessionStateProjectionStore,
    command: &KernelCommand,
    request: &LocalDaemonRequest,
) -> Result<String, DaemonError> {
    if is_implicit_local_session_caller(command) {
        return Ok(DEFAULT_LOCAL_USER_ID.to_string());
    }
    if matches!(
        request,
        LocalDaemonRequest::CreateSession(_) | LocalDaemonRequest::JoinSessionInvite(_)
    ) {
        return Ok(
            command_session_user_id(command).unwrap_or_else(|| DEFAULT_LOCAL_USER_ID.to_string())
        );
    }

    let Some(scope) = request_session_scope(request) else {
        return Ok(
            command_session_user_id(command).unwrap_or_else(|| DEFAULT_LOCAL_USER_ID.to_string())
        );
    };
    let user_id = command_session_user_id(command).ok_or_else(|| {
        DaemonError::MissingSessionCallerIdentity {
            operation: command.command_type.clone(),
        }
    })?;

    match scope {
        SessionMembershipScope::AllSessions => Ok(user_id),
        SessionMembershipScope::SessionId(session_id) => {
            ensure_session_member(app, session_projection, &session_id, &user_id).await?;
            Ok(user_id)
        }
        SessionMembershipScope::SessionRef {
            session_ref,
            workspace_id,
        } => {
            let session = resolve_session_for_membership(
                app,
                session_projection,
                &session_ref,
                workspace_id.as_deref(),
            )
            .await?;
            if !session.has_member(&user_id) {
                return Err(DaemonError::SessionAccessDenied {
                    session_id: session.id().to_string(),
                    user_id,
                });
            }
            Ok(user_id)
        }
        SessionMembershipScope::AttachmentId(attachment_id) => {
            let session_id = if let Some(session_id) =
                session_projection.session_id_for_attachment(&attachment_id)
            {
                session_id
            } else {
                let app = app.lock().await;
                app.sessions()
                    .list_sessions()
                    .into_iter()
                    .find(|session| session.has_attachment(&attachment_id))
                    .map(|session| session.id().to_string())
                    .ok_or_else(|| DaemonError::AttachmentNotFound {
                        attachment_id: attachment_id.clone(),
                    })?
            };
            ensure_session_member(app, session_projection, &session_id, &user_id).await?;
            Ok(user_id)
        }
    }
}

async fn ensure_session_member(
    app: &Arc<Mutex<DaemonApp>>,
    session_projection: &SessionStateProjectionStore,
    session_id: &str,
    user_id: &str,
) -> Result<(), DaemonError> {
    if let Some(session) = session_projection.get(session_id) {
        if session.has_member(user_id) {
            return Ok(());
        }
        return Err(DaemonError::SessionAccessDenied {
            session_id: session.id().to_string(),
            user_id: user_id.to_string(),
        });
    }
    let session = {
        let app = app.lock().await;
        app.sessions().get_session(session_id)?
    };
    if session.has_member(user_id) {
        Ok(())
    } else {
        Err(DaemonError::SessionAccessDenied {
            session_id: session.id().to_string(),
            user_id: user_id.to_string(),
        })
    }
}

async fn resolve_session_for_membership(
    app: &Arc<Mutex<DaemonApp>>,
    session_projection: &SessionStateProjectionStore,
    session_ref: &str,
    workspace_id: Option<&str>,
) -> Result<crate::session::RuntimeSession, DaemonError> {
    if let Some(session) = session_projection.resolve_session_ref(session_ref, workspace_id) {
        return Ok(session);
    }
    let app = app.lock().await;
    app.sessions()
        .resolve_session_ref(session_ref, workspace_id)
}

pub(crate) fn request_session_scope(
    request: &LocalDaemonRequest,
) -> Option<SessionMembershipScope> {
    match request {
        LocalDaemonRequest::ListSessions(_) => Some(SessionMembershipScope::AllSessions),
        LocalDaemonRequest::ResolveSession(request) => Some(SessionMembershipScope::SessionRef {
            session_ref: request.session_ref.clone(),
            workspace_id: request.workspace_id.clone(),
        }),
        LocalDaemonRequest::DeleteSession(request) => Some(SessionMembershipScope::SessionRef {
            session_ref: request.session_ref.clone(),
            workspace_id: request.workspace_id.clone(),
        }),
        LocalDaemonRequest::DetachFromSession(request) => Some(
            SessionMembershipScope::AttachmentId(request.attachment_id.clone()),
        ),
        LocalDaemonRequest::QueryHistory(request) => optional_session_scope(request),
        LocalDaemonRequest::SearchHistory(request) => request
            .session_id
            .as_ref()
            .map(|session_id| SessionMembershipScope::SessionId(session_id.clone())),
        LocalDaemonRequest::SemanticSearchHistory(request) => request
            .session_id
            .as_ref()
            .map(|session_id| SessionMembershipScope::SessionId(session_id.clone())),
        LocalDaemonRequest::AttachToSession(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::LaunchProviderRun(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::UpdateProviderRunSelection(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::ListSessionMembers(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::CreateSessionInvite(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::RevokeSessionInvite(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::CreateWorkspaceLink(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::ListWorkspaceLinks(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::ShowWorkspaceLink(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::AttachWorkspaceLink(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::DetachWorkspaceLink(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::SubmitPrompt(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::CompletePrompt(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::CancelActivePrompt(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::UpdateSessionConfig(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::UpdateAgentConfig(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::UpdateAgentProfile(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::AliasAgent(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::UpdateAgentSubstitutes(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::RequestNativeProviderInteraction(request) => {
            Some(SessionMembershipScope::SessionId(request.session_id.clone()))
        }
        LocalDaemonRequest::GetSessionState(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::AliasSession(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::GetSessionHistory(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::GetPromptInputHistory(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::RecordPromptInputHistory(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::PollRuntimeNotices(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::ResizeTerminal(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::SendTerminalInput(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::PumpTerminalOutput(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::AppendNativeProviderOutput(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::EndSession(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::RunShellCommand(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::ReadDirectoryTree(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::ReadFile(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::EditFile(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::InspectGit(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::CaptureScreenshot(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::StoreTransferredFile(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::SpawnAgent(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::MoveAgentToRemote(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::DestroyAgent(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::FocusAgent(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::CycleAgentFocus(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::ListAgents(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::CreateWorkflow(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::ApplyWorkflowDesignOp(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::AliasWorkflow(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::ListWorkflows(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::ResolveWorkflow(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::CreateWorkflowPublication(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::ListWorkflowPublications(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::GetWorkflowPublication(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::DisableWorkflowPublication(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::CreateWorkflowPublicationPairCode(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::RedeemWorkflowPublicationPairCode(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::ListWorkflowPublicationSenders(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::RevokeWorkflowPublicationSender(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::AuthenticateWorkflowPublicationSender(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::CreateWorkflowEndpoint(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::AliasWorkflowEndpoint(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::BindWorkflowEndpoint(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::AddWorkflowNode(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::RemoveWorkflowNode(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::UpdateWorkflowNodeInstructions(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::SetWorkflowNodeCanCompleteRun(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::SetWorkflowNodeCanEmitIntermediateOutput(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::SetWorkflowNodeIntermediateOutputSchema(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::SetWorkflowNodeMaxTurns(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::AddWorkflowEdge(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::ValidateWorkflowOutput(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::AckWorkflowTurn(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::RemoveWorkflowEdge(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::UpdateWorkflowCanvasLayout(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::InvokeWorkflowEndpoint(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::ListWorkflowRuns(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::GetWorkflowRun(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::CancelWorkflowRun(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::ResumeWorkflowRun(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::CreateWorkflowWatchdog(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::ListWorkflowWatchdogs(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::SetWorkflowWatchdogEnabled(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::RemoveWorkflowWatchdog(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::SetWorkflowFlushContext(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::SetWorkflowRunOutputSchema(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::SetWorkflowIntermediateOutputSchema(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::SetWorkflowLaunchPolicy(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::ListQueuedWorkflowLaunches(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::RemoveQueuedWorkflowLaunch(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::ClearQueuedWorkflowLaunches(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        _ => None,
    }
}

fn optional_session_scope(request: &QueryHistoryRequest) -> Option<SessionMembershipScope> {
    request
        .session_id
        .as_ref()
        .map(|session_id| SessionMembershipScope::SessionId(session_id.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::attachment::ClientCapabilityLevel;
    use crate::local::{
        AttachToSessionRequest, DetachFromSessionRequest, ListSessionsRequest, QueryHistoryRequest,
        RelayStatusRequest, ResolveSessionRequest,
    };
    use crate::runtime::command::{KernelCaller, KernelCommandSource};

    #[test]
    fn local_client_session_identity_falls_back_to_default_user() {
        let request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let command = KernelCommand::from_local_request("cmd", None, None, &request);

        assert!(is_implicit_local_session_caller(&command));
        assert_eq!(
            command_session_user_id(&command).as_deref(),
            Some(DEFAULT_LOCAL_USER_ID)
        );
    }

    #[test]
    fn remote_client_session_identity_requires_user_id() {
        let request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let caller = KernelCaller::for_source(&KernelCommandSource::RelayClient);
        let command = KernelCommand::from_local_request_with_caller(
            "cmd",
            KernelCommandSource::RelayClient,
            caller,
            None,
            None,
            &request,
        );

        assert!(!is_implicit_local_session_caller(&command));
        assert_eq!(command_session_user_id(&command), None);
    }

    #[test]
    fn request_session_scope_maps_session_and_attachment_requests() {
        assert_eq!(
            request_session_scope(&LocalDaemonRequest::ListSessions(ListSessionsRequest)),
            Some(SessionMembershipScope::AllSessions)
        );
        assert_eq!(
            request_session_scope(&LocalDaemonRequest::ResolveSession(ResolveSessionRequest {
                session_ref: "session-alias".to_string(),
                workspace_id: Some("workspace-1".to_string()),
            })),
            Some(SessionMembershipScope::SessionRef {
                session_ref: "session-alias".to_string(),
                workspace_id: Some("workspace-1".to_string()),
            })
        );
        assert_eq!(
            request_session_scope(&LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: "session-1".to_string(),
                    client_id: "client-1".to_string(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                }
            )),
            Some(SessionMembershipScope::SessionId("session-1".to_string()))
        );
        assert_eq!(
            request_session_scope(&LocalDaemonRequest::DetachFromSession(
                DetachFromSessionRequest {
                    attachment_id: "attachment-1".to_string(),
                },
            )),
            Some(SessionMembershipScope::AttachmentId(
                "attachment-1".to_string()
            ))
        );
    }

    #[test]
    fn request_session_scope_keeps_global_queries_unscoped() {
        assert_eq!(
            request_session_scope(&LocalDaemonRequest::QueryHistory(QueryHistoryRequest {
                session_id: None,
                ..QueryHistoryRequest::default()
            })),
            None
        );
        assert_eq!(
            request_session_scope(&LocalDaemonRequest::RelayStatus(RelayStatusRequest)),
            None
        );
    }
}
