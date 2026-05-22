use crate::local::{LocalDaemonRequest, QueryHistoryRequest};

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
        LocalDaemonRequest::RequestNativeProviderInteraction(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
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
        LocalDaemonRequest::ValidateWorkflowHandoff(request) => Some(
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
