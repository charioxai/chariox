use std::collections::HashSet;

use crate::local::{LocalDaemonRequest, QueryRecallRequest};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SessionMembershipScope {
    AllSessions,
    SessionId(String),
    SessionIds(Vec<String>),
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
        LocalDaemonRequest::QueryRecall(request) => optional_session_scope(request),
        LocalDaemonRequest::SearchRecall(request) => request
            .session_id
            .as_ref()
            .map(|session_id| SessionMembershipScope::SessionId(session_id.clone())),
        LocalDaemonRequest::SemanticSearchRecall(request) => request
            .session_id
            .as_ref()
            .map(|session_id| SessionMembershipScope::SessionId(session_id.clone())),
        LocalDaemonRequest::AttachToSession(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::LaunchProviderRun(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::LaunchProviderRuns(request) => unique_session_scope(
            request
                .launches
                .iter()
                .map(|launch| launch.session_id.as_str()),
        ),
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
        LocalDaemonRequest::GetWorkspaceLiveSyncStatus(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::SubmitPrompt(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::SubmitPrompts(request) => unique_session_scope(
            std::iter::once(request.session_id.as_str()).chain(
                request
                    .prompts
                    .iter()
                    .filter_map(|prompt| prompt.session_id.as_deref()),
            ),
        ),
        LocalDaemonRequest::CreateAgentPromptSchedule(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::CancelAgentPromptSchedule(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::CompletePrompt(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::CancelActivePrompt(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::SteerQueuedPrompt(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::CancelQueuedPrompt(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::UpdateQueuedPrompt(request) => Some(SessionMembershipScope::SessionId(
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
        LocalDaemonRequest::ArmDeploymentCredentialEnrollment(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::RespondToInteraction(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::RequestNativeProviderInteraction(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::GetSessionState(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::GetRoomEnvironmentState(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::GetRoomEnvironmentSlice(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::BindRoomEnvironmentSlice(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::GetRoomEnvironmentEvents(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::ListRoomEnvironmentActionHistory(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::StartRoomEnvironment(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::StopRoomEnvironment(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::RetryRoomEnvironment(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::UpdateRoomEnvironmentViewport(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::RequestRoomEnvironmentInputTakeover(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::ReleaseRoomEnvironmentInput(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::SubmitRoomEnvironmentAction(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::CancelRoomEnvironmentAction(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::UpdateMetaagentTask(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::PauseMetaagentTask(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::ResumeMetaagentTask(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::AbortMetaagentTask(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::ExportDebugBundle(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::AliasSession(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::GetSessionHistoryOutline(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::GetSessionHistoryBlobContent(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
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
        LocalDaemonRequest::AppendNativeProviderOutputBatch(request) => Some(
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
        LocalDaemonRequest::SpawnAgents(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::ImportExternalProviderAgent(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::MoveAgentToRemote(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::MoveAgentToLocal(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::DestroyAgent(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::FocusAgent(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::AcknowledgeAgentOutputSeen(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::CycleAgentFocus(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::ListAgents(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::CreateWorkflow(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::ValidateWorkflowCode(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::ApplyWorkflowCode(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::RunWorkflowCode(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::CreateWorkflowCodeArtifact(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::UpdateWorkflowCodeArtifact(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::GetWorkflowCodeArtifact(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::ListWorkflowCodeArtifacts(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::DeleteWorkflowCodeArtifact(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::ExportWorkflowCodeArtifact(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::ImportWorkflowCodeArtifact(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::ExportWorkflowCodePackage(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::ImportWorkflowCodePackage(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::ExportWorkflowCodeSource(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
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
        LocalDaemonRequest::ExportWorkflowPublicationPackage(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::DisableWorkflowPublication(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::CreateWorkflowEventBinding(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::ListWorkflowEventBindings(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::SetWorkflowEventBindingStatus(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::TransferWorkflowEventBinding(request) => Some(
            SessionMembershipScope::SessionId(request.source_session_id.clone()),
        ),
        LocalDaemonRequest::TestWorkflowEventBinding(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::ControlWorkflowPublicationRuntime(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::BindWorkflowPublicationDeployment(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::RegisterWorkflowPublicationEndpoint(request) => Some(
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
        LocalDaemonRequest::SetWorkflowNodeWaitForAllInputs(request) => Some(
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
        LocalDaemonRequest::PauseWorkflowRun(request) => Some(SessionMembershipScope::SessionId(
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
        LocalDaemonRequest::CreateWorkflowSchedule(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::ListWorkflowSchedules(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::SetWorkflowScheduleEnabled(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::RemoveWorkflowSchedule(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::PreviewWorkflowSchedule(_) => None,
        LocalDaemonRequest::SetWorkflowFlushContext(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::SetWorkflowRunOutputSchema(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::ListWorkflowPromptQueues(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::CreateWorkflowPromptQueue(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::UpdateWorkflowPromptQueue(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::RemoveWorkflowPromptQueue(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::ListQueuedWorkflowPrompts(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::UpdateQueuedWorkflowPrompt(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::RemoveQueuedWorkflowPrompt(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::ClearWorkflowPromptQueue(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        _ => None,
    }
}

fn optional_session_scope(request: &QueryRecallRequest) -> Option<SessionMembershipScope> {
    request
        .session_id
        .as_ref()
        .map(|session_id| SessionMembershipScope::SessionId(session_id.clone()))
}

fn unique_session_scope<'a>(
    session_ids: impl IntoIterator<Item = &'a str>,
) -> Option<SessionMembershipScope> {
    let mut seen = HashSet::new();
    let mut unique = Vec::new();
    for session_id in session_ids {
        if seen.insert(session_id) {
            unique.push(session_id.to_string());
        }
    }
    match unique.len() {
        0 => None,
        1 => unique.pop().map(SessionMembershipScope::SessionId),
        _ => Some(SessionMembershipScope::SessionIds(unique)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::attachment::ClientCapabilityLevel;
    use crate::local::{
        ArmDeploymentCredentialEnrollmentRequest, AttachToSessionRequest, DetachFromSessionRequest,
        LaunchProviderRunRequest, LaunchProviderRunsRequest, ListSessionsRequest,
        QueryRecallRequest, RelayStatusRequest, ResolveSessionRequest, RespondToInteractionRequest,
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
        assert_eq!(
            request_session_scope(&LocalDaemonRequest::RespondToInteraction(
                RespondToInteractionRequest {
                    session_id: "session-1".to_string(),
                    interaction_id: "interaction-1".to_string(),
                    choice_id: "cancel".to_string(),
                    custom_reply: None,
                },
            )),
            Some(SessionMembershipScope::SessionId("session-1".to_string()))
        );
        assert_eq!(
            request_session_scope(&LocalDaemonRequest::ArmDeploymentCredentialEnrollment(
                ArmDeploymentCredentialEnrollmentRequest {
                    session_id: "session-1".to_string(),
                    attachment_id: "attachment-1".to_string(),
                    agent_id: "agent-1".to_string(),
                    enrollment_id: "enrollment-1".to_string(),
                    profile_id: "profile-1".to_string(),
                    target_version: 1,
                },
            )),
            Some(SessionMembershipScope::SessionId("session-1".to_string()))
        );
    }

    #[test]
    fn request_session_scope_keeps_global_queries_unscoped() {
        assert_eq!(
            request_session_scope(&LocalDaemonRequest::QueryRecall(QueryRecallRequest {
                session_id: None,
                ..QueryRecallRequest::default()
            })),
            None
        );
        assert_eq!(
            request_session_scope(&LocalDaemonRequest::RelayStatus(RelayStatusRequest)),
            None
        );
    }

    #[test]
    fn request_session_scope_covers_every_session_in_provider_batch() {
        let request = LocalDaemonRequest::LaunchProviderRuns(LaunchProviderRunsRequest {
            max_concurrency: Some(4),
            launches: vec![
                launch("session-a", "agent-a-1"),
                launch("session-a", "agent-a-2"),
                launch("session-b", "agent-b-1"),
                launch("session-c", "agent-c-1"),
                launch("session-b", "agent-b-2"),
            ],
        });

        assert_eq!(
            request_session_scope(&request),
            Some(SessionMembershipScope::SessionIds(vec![
                "session-a".to_string(),
                "session-b".to_string(),
                "session-c".to_string(),
            ]))
        );
    }

    fn launch(session_id: &str, agent_id: &str) -> LaunchProviderRunRequest {
        LaunchProviderRunRequest {
            session_id: session_id.to_string(),
            agent_id: Some(agent_id.to_string()),
            adapter_key: "dev-stub".to_string(),
            provider: "claude-code".to_string(),
            account_profile: "default".to_string(),
            model: "sonnet".to_string(),
            variant: None,
            structured_endpoint: None,
            provider_session_id: None,
            native_tui: false,
        }
    }
}
