use crate::error::DaemonError;
use crate::local::LocalDaemonRequest;
use crate::runtime::projection::SessionStateProjectionStore;
use crate::runtime::session_actor::SESSION_CREATE_LANE_ID;

use super::store::SessionRuntimeStore;

pub(super) async fn resolve_session_lane_key(
    store: &SessionRuntimeStore,
    session_projection: &SessionStateProjectionStore,
    request: &LocalDaemonRequest,
) -> Result<String, DaemonError> {
    match request {
        LocalDaemonRequest::CreateSession(_)
        | LocalDaemonRequest::ListProjects(_)
        | LocalDaemonRequest::RenameProject(_)
        | LocalDaemonRequest::ArchiveProject(_)
        | LocalDaemonRequest::DeleteProject(_)
        | LocalDaemonRequest::RestoreProject(_) => Ok(SESSION_CREATE_LANE_ID.to_string()),
        LocalDaemonRequest::AttachToSession(request) => {
            resolve_direct_session_lane_key(session_projection, &request.session_id)
        }
        LocalDaemonRequest::FocusAgent(request) => {
            resolve_direct_session_lane_key(session_projection, &request.session_id)
        }
        LocalDaemonRequest::AcknowledgeAgentOutputSeen(request) => {
            resolve_direct_session_lane_key(session_projection, &request.session_id)
        }
        LocalDaemonRequest::CycleAgentFocus(request) => {
            resolve_direct_session_lane_key(session_projection, &request.session_id)
        }
        LocalDaemonRequest::ResizeTerminal(request) => {
            resolve_direct_session_lane_key(session_projection, &request.session_id)
        }
        LocalDaemonRequest::SendTerminalInput(request) => {
            resolve_attachment_scoped_session_lane_key(
                session_projection,
                &request.session_id,
                &request.attachment_id,
            )
        }
        LocalDaemonRequest::PollRuntimeNotices(request) => {
            resolve_attachment_scoped_session_lane_key(
                session_projection,
                &request.session_id,
                &request.attachment_id,
            )
        }
        LocalDaemonRequest::UpdateSessionConfig(request) => {
            resolve_attachment_scoped_session_lane_key(
                session_projection,
                &request.session_id,
                &request.attachment_id,
            )
        }
        LocalDaemonRequest::StartRoomEnvironment(request) => {
            resolve_direct_session_lane_key(session_projection, &request.session_id)
        }
        LocalDaemonRequest::BindRoomEnvironmentSlice(request) => {
            resolve_direct_session_lane_key(session_projection, &request.session_id)
        }
        LocalDaemonRequest::StopRoomEnvironment(request) => {
            resolve_direct_session_lane_key(session_projection, &request.session_id)
        }
        LocalDaemonRequest::RetryRoomEnvironment(request) => {
            resolve_direct_session_lane_key(session_projection, &request.session_id)
        }
        LocalDaemonRequest::UpdateRoomEnvironmentViewport(request) => {
            resolve_direct_session_lane_key(session_projection, &request.session_id)
        }
        LocalDaemonRequest::UpdateRoomEnvironmentPointer(request) => {
            resolve_direct_session_lane_key(session_projection, &request.session_id)
        }
        LocalDaemonRequest::RequestRoomEnvironmentInputTakeover(request) => {
            resolve_direct_session_lane_key(session_projection, &request.session_id)
        }
        LocalDaemonRequest::ReleaseRoomEnvironmentInput(request) => {
            resolve_direct_session_lane_key(session_projection, &request.session_id)
        }
        LocalDaemonRequest::SubmitRoomEnvironmentAction(request) => {
            resolve_direct_session_lane_key(session_projection, &request.session_id)
        }
        LocalDaemonRequest::CancelRoomEnvironmentAction(request) => {
            resolve_direct_session_lane_key(session_projection, &request.session_id)
        }
        LocalDaemonRequest::CreateAgentPromptSchedule(request) => {
            resolve_direct_session_lane_key(session_projection, &request.session_id)
        }
        LocalDaemonRequest::CancelAgentPromptSchedule(request) => {
            resolve_direct_session_lane_key(session_projection, &request.session_id)
        }
        LocalDaemonRequest::UpdateAgentConfig(request) => {
            resolve_direct_session_lane_key(session_projection, &request.session_id)
        }
        LocalDaemonRequest::UpdateAgentProfile(request) => {
            resolve_direct_session_lane_key(session_projection, &request.session_id)
        }
        LocalDaemonRequest::AliasAgent(request) => {
            resolve_direct_session_lane_key(session_projection, &request.session_id)
        }
        LocalDaemonRequest::UpdateAgentSubstitutes(request) => {
            resolve_direct_session_lane_key(session_projection, &request.session_id)
        }
        LocalDaemonRequest::RespondToInteraction(request) => {
            resolve_direct_session_lane_key(session_projection, &request.session_id)
        }
        LocalDaemonRequest::AliasSession(request) => {
            resolve_direct_session_lane_key(session_projection, &request.session_id)
        }
        LocalDaemonRequest::SpawnAgent(request) => {
            resolve_direct_session_lane_key(session_projection, &request.session_id)
        }
        LocalDaemonRequest::SpawnAgents(request) => {
            resolve_direct_session_lane_key(session_projection, &request.session_id)
        }
        LocalDaemonRequest::UndoTurn(request) => {
            resolve_direct_session_lane_key(session_projection, &request.session_id)
        }
        LocalDaemonRequest::ForkAgent(request) => {
            resolve_direct_session_lane_key(session_projection, &request.session_id)
        }
        LocalDaemonRequest::DestroyAgent(request) => {
            resolve_direct_session_lane_key(session_projection, &request.session_id)
        }
        LocalDaemonRequest::EndSession(request) => {
            resolve_direct_session_lane_key(session_projection, &request.session_id)
        }
        LocalDaemonRequest::DeleteSession(request) => {
            if let Some(session_id) = session_projection
                .resolve_session_ref_id(&request.session_ref, request.workspace_id.as_deref())
            {
                return Ok(session_id);
            }
            if let Some(result) = session_projection.resolve_session_ref_id_from_warmed_list(
                &request.session_ref,
                request.workspace_id.as_deref(),
            ) {
                return result;
            }
            store
                .resolve_session_ref_id(&request.session_ref, request.workspace_id.as_deref())
                .await
        }
        LocalDaemonRequest::DetachFromSession(request) => {
            if let Some(session_id) =
                session_projection.session_id_for_attachment(&request.attachment_id)
            {
                return Ok(session_id);
            }
            if session_projection.has_warmed_list() {
                return Err(DaemonError::AttachmentNotFound {
                    attachment_id: request.attachment_id.clone(),
                });
            }
            store.attachment_session_id(&request.attachment_id).await
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "route session kernel command",
            message: "request is not handled by the session runtime".to_string(),
        }),
    }
}

fn resolve_direct_session_lane_key(
    session_projection: &SessionStateProjectionStore,
    session_id: &str,
) -> Result<String, DaemonError> {
    if session_projection.get(session_id).is_some() || !session_projection.has_warmed_list() {
        return Ok(session_id.to_string());
    }
    Err(DaemonError::SessionNotFound {
        session_id: session_id.to_string(),
    })
}

fn resolve_attachment_scoped_session_lane_key(
    session_projection: &SessionStateProjectionStore,
    session_id: &str,
    attachment_id: &str,
) -> Result<String, DaemonError> {
    if session_projection
        .get(session_id)
        .is_some_and(|session| session.has_attachment(attachment_id))
        || !session_projection.has_warmed_list()
    {
        return Ok(session_id.to_string());
    }
    if session_projection
        .session_id_for_attachment(attachment_id)
        .is_some()
    {
        return Err(DaemonError::AttachmentNotInSession {
            session_id: session_id.to_string(),
            attachment_id: attachment_id.to_string(),
        });
    }
    Err(DaemonError::AttachmentNotFound {
        attachment_id: attachment_id.to_string(),
    })
}
