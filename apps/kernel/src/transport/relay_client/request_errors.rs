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
        DaemonError::LocalTransport { operation, message }
            if operation.starts_with("environment.") =>
        {
            let code = message
                .split_once(':')
                .map_or(message.as_str(), |(code, _)| code);
            relay_error(code, &error.to_string(), false)
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
        LocalDaemonRequest::GetRoomEnvironmentState(_) => "environment.state.get",
        LocalDaemonRequest::GetRoomEnvironmentEvents(_) => "environment.events.get",
        LocalDaemonRequest::ListRoomEnvironmentActionHistory(_) => "environment.history.list",
        LocalDaemonRequest::StartRoomEnvironment(_) => "environment.start",
        LocalDaemonRequest::GetRoomEnvironmentSlice(_) => "environment.slice.get",
        LocalDaemonRequest::BindRoomEnvironmentSlice(_) => "environment.slice.bind",
        LocalDaemonRequest::StopRoomEnvironment(_) => "environment.stop",
        LocalDaemonRequest::RetryRoomEnvironment(_) => "environment.retry",
        LocalDaemonRequest::UpdateRoomEnvironmentViewport(_) => "environment.viewport.update",
        LocalDaemonRequest::RequestRoomEnvironmentInputTakeover(_) => "environment.input.takeover",
        LocalDaemonRequest::ReleaseRoomEnvironmentInput(_) => "environment.input.release",
        LocalDaemonRequest::CancelRoomEnvironmentAction(_) => "environment.action.cancel",
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::{
        CancelRoomEnvironmentActionRequest, GetRoomEnvironmentEventsRequest,
        GetRoomEnvironmentStateRequest, ListRoomEnvironmentActionHistoryRequest,
        ReleaseRoomEnvironmentInputRequest, RequestRoomEnvironmentInputTakeoverRequest,
        RetryRoomEnvironmentRequest, RoomEnvironmentViewportRequest, StartRoomEnvironmentRequest,
        StopRoomEnvironmentRequest, UpdateRoomEnvironmentViewportRequest,
    };

    #[test]
    fn room_environment_state_uses_the_shared_relay_request_path() {
        let request = LocalDaemonRequest::GetRoomEnvironmentState(GetRoomEnvironmentStateRequest {
            session_id: "session-1".to_string(),
        });

        assert_eq!(relay_request_kind(&request), "environment.state.get");
    }

    #[test]
    fn room_environment_events_use_the_shared_relay_request_path() {
        let request =
            LocalDaemonRequest::GetRoomEnvironmentEvents(GetRoomEnvironmentEventsRequest {
                session_id: "session-1".to_string(),
                cursor: 12,
            });

        assert_eq!(relay_request_kind(&request), "environment.events.get");
    }

    #[test]
    fn room_environment_history_uses_the_shared_relay_request_path() {
        let request = LocalDaemonRequest::ListRoomEnvironmentActionHistory(
            ListRoomEnvironmentActionHistoryRequest {
                session_id: "session-1".to_string(),
                before_sequence: Some(12),
                limit: Some(25),
            },
        );

        assert_eq!(relay_request_kind(&request), "environment.history.list");
    }

    #[test]
    fn room_environment_start_uses_the_shared_relay_request_path() {
        let request = LocalDaemonRequest::StartRoomEnvironment(StartRoomEnvironmentRequest {
            session_id: "session-1".to_string(),
            viewport: RoomEnvironmentViewportRequest {
                css_width: 1280,
                css_height: 800,
                device_scale_factor: 1,
                desktop_pixel_width: 1280,
                desktop_pixel_height: 800,
            },
        });

        assert_eq!(relay_request_kind(&request), "environment.start");
    }

    #[test]
    fn room_environment_stop_uses_the_shared_relay_request_path() {
        let request = LocalDaemonRequest::StopRoomEnvironment(StopRoomEnvironmentRequest {
            session_id: "session-1".to_string(),
        });

        assert_eq!(relay_request_kind(&request), "environment.stop");
    }

    #[test]
    fn room_environment_retry_uses_the_shared_relay_request_path() {
        let request = LocalDaemonRequest::RetryRoomEnvironment(RetryRoomEnvironmentRequest {
            session_id: "session-1".to_string(),
        });

        assert_eq!(relay_request_kind(&request), "environment.retry");
    }

    #[test]
    fn room_environment_control_errors_keep_stable_relay_codes() {
        let error = DaemonError::LocalTransport {
            operation: "environment.start",
            message: "environment_invalid_lifecycle_transition: invalid transition".to_string(),
        };

        let relay_error = map_relay_error(&error);
        assert_eq!(relay_error.code, "environment_invalid_lifecycle_transition");
        assert!(!relay_error.retryable);
    }

    #[test]
    fn room_environment_placement_errors_keep_stable_relay_codes() {
        let error = DaemonError::LocalTransport {
            operation: "environment.slice.bind",
            message: "environment_slice_binding_rejected: slice belongs to another Room"
                .to_string(),
        };
        assert_eq!(
            map_relay_error(&error).code,
            "environment_slice_binding_rejected"
        );
    }

    #[test]
    fn room_environment_viewport_uses_the_shared_relay_request_path() {
        let request = LocalDaemonRequest::UpdateRoomEnvironmentViewport(
            UpdateRoomEnvironmentViewportRequest {
                session_id: "session-1".to_string(),
                expected_revision: 1,
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 1280,
                    css_height: 800,
                    device_scale_factor: 1,
                    desktop_pixel_width: 1280,
                    desktop_pixel_height: 800,
                },
            },
        );

        assert_eq!(relay_request_kind(&request), "environment.viewport.update");
    }

    #[test]
    fn room_environment_takeover_uses_the_shared_relay_request_path() {
        let request = LocalDaemonRequest::RequestRoomEnvironmentInputTakeover(
            RequestRoomEnvironmentInputTakeoverRequest {
                session_id: "session-1".to_string(),
                target: crate::session::InputTarget::Desktop,
            },
        );

        assert_eq!(relay_request_kind(&request), "environment.input.takeover");
    }

    #[test]
    fn room_environment_input_release_uses_the_shared_relay_request_path() {
        let request =
            LocalDaemonRequest::ReleaseRoomEnvironmentInput(ReleaseRoomEnvironmentInputRequest {
                session_id: "session-1".to_string(),
                target: crate::session::InputTarget::Desktop,
            });

        assert_eq!(relay_request_kind(&request), "environment.input.release");
    }

    #[test]
    fn room_environment_action_cancel_uses_the_shared_relay_request_path() {
        let request =
            LocalDaemonRequest::CancelRoomEnvironmentAction(CancelRoomEnvironmentActionRequest {
                session_id: "session-1".to_string(),
                action_id: "action-1".to_string(),
            });

        assert_eq!(relay_request_kind(&request), "environment.action.cancel");
    }
}
