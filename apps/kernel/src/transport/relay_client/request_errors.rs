//! Relay request classification and error projection.

use chariox_relay::protocol::RelayError;

use crate::error::DaemonError;
use crate::local::LocalDaemonRequest;

pub(super) fn map_relay_error(error: &DaemonError) -> RelayError {
    match error {
        DaemonError::SessionNotFound { .. } => {
            relay_error("session_not_found", &error.to_string(), false)
        }
        DaemonError::AttachmentNotFound { .. } => {
            relay_error("attachment_not_found", &error.to_string(), false)
        }
        DaemonError::AttachmentNotInSession { .. } => {
            relay_error("attachment_not_in_session", &error.to_string(), false)
        }
        DaemonError::NoActiveProviderRun { .. } => {
            relay_error("no_active_provider_run", &error.to_string(), false)
        }
        DaemonError::LocalTransport { .. } => {
            relay_error("transport_error", &error.to_string(), true)
        }
        DaemonError::RemoteLeasesDisabled { .. } => {
            relay_error("remote_leases_disabled", &error.to_string(), false)
        }
        DaemonError::ExecutionLeaseNotFound { .. } => {
            relay_error("execution_lease_not_found", &error.to_string(), false)
        }
        DaemonError::LeasedAgentNotFound { .. } => {
            relay_error("leased_agent_not_found", &error.to_string(), false)
        }
        _ => relay_error("relay_request_failed", &error.to_string(), false),
    }
}

pub(super) fn relay_error(code: &str, message: &str, retryable: bool) -> RelayError {
    RelayError {
        code: code.to_string(),
        message: message.to_string(),
        retryable,
    }
}

pub(super) fn relay_request_kind(request: &LocalDaemonRequest) -> &'static str {
    match request {
        LocalDaemonRequest::GetWaitingRoomInventory(_) => "waiting_room.inventory.get",
        LocalDaemonRequest::GetWaitingRoomPublicSnapshot(_) => "waiting_room.public_snapshot.get",
        LocalDaemonRequest::GetTerminalCommandCatalog(_) => "terminal.command_catalog.get",
        LocalDaemonRequest::GetProviderCatalog(_) => "provider.catalog.get",
        LocalDaemonRequest::SearchRecall(_) => "recall.search",
        LocalDaemonRequest::SemanticSearchRecall(_) => "recall.semantic_search",
        LocalDaemonRequest::ListSessions(_) => "session.list",
        LocalDaemonRequest::CreateSession(_) => "session.create",
        LocalDaemonRequest::AttachToSession(_) => "session.attach",
        LocalDaemonRequest::GetSessionState(_) => "session.state.get",
        LocalDaemonRequest::GetSessionHistoryOutline(_) => "session.history.outline.get",
        LocalDaemonRequest::GetSessionHistoryBlobContent(_) => "session.history.blob.get",
        LocalDaemonRequest::ListSlices(_) => "slice.list",
        LocalDaemonRequest::CreateSlice(_) => "slice.create",
        LocalDaemonRequest::GetSlice(_) => "slice.get",
        LocalDaemonRequest::StartSlice(_) => "slice.start",
        LocalDaemonRequest::StopSlice(_) => "slice.stop",
        LocalDaemonRequest::DeleteSlice(_) => "slice.delete",
        LocalDaemonRequest::ImportSliceProviderAuth(_) => "slice.auth.import",
        LocalDaemonRequest::RemoveSliceProviderAuth(_) => "slice.auth.remove",
        LocalDaemonRequest::StartSliceProviderLogin(_) => "slice.auth.login",
        LocalDaemonRequest::GetSliceDisplayEndpoint(_) => "slice.display_endpoint.get",
        LocalDaemonRequest::GetSliceLogs(_) => "slice.logs.get",
        LocalDaemonRequest::ListSliceAudit(_) => "slice.audit.list",
        LocalDaemonRequest::SaveSliceState(_) => "slice.state.save",
        LocalDaemonRequest::GetSliceStateStatus(_) => "slice.state.status",
        LocalDaemonRequest::ResetSliceState(_) => "slice.state.reset",
        LocalDaemonRequest::CreateSliceBackup(_) => "slice.backup.create",
        LocalDaemonRequest::LaunchProviderRun(_) => "provider.run.launch",
        LocalDaemonRequest::UpdateProviderRunSelection(_) => "provider.run.selection.update",
        LocalDaemonRequest::CreateWorkspaceDirectory(_) => "workspace.directory.create",
        LocalDaemonRequest::CreateWorkspaceWorktree(_) => "workspace.worktree.create",
        LocalDaemonRequest::GetWorkspaceGitOverview(_) => "workspace.git.overview",
        LocalDaemonRequest::ListWorkspaceFiles(_) => "workspace.files.list",
        LocalDaemonRequest::GetWorkspaceFileContent(_) => "workspace.file.content",
        LocalDaemonRequest::RunAgentUtility(_) => "agent.utility.run",
        LocalDaemonRequest::GenerateWorkspaceCommitMessage(_) => {
            "workspace.commit_message.generate"
        }
        LocalDaemonRequest::CommitWorkspaceChanges(_) => "workspace.git.commit",
        LocalDaemonRequest::PushWorkspaceBranch(_) => "workspace.git.push",
        LocalDaemonRequest::CommitAndPushWorkspaceChanges(_) => "workspace.git.commit_and_push",
        _ => "other",
    }
}
