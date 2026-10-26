use crate::error::DaemonError;
use crate::local::{
    deployment_credential_enrollment_interaction_id, LocalDaemonRequest,
    DEPLOYMENT_CREDENTIAL_ENROLLMENT_SERVICE_SUBJECT_PREFIX,
};
use crate::runtime::command::{KernelCallerKind, KernelCommand, KernelCommandSource};
use crate::runtime::projection::SessionStateProjectionStore;
use crate::runtime::state::KernelRuntimeState;
use crate::session::DEFAULT_LOCAL_USER_ID;

pub(crate) mod scope;
use scope::{request_session_scope, SessionMembershipScope};

pub(crate) fn command_session_user_id(command: &KernelCommand) -> Option<String> {
    match command.caller.caller_kind {
        KernelCallerKind::LocalClient | KernelCallerKind::Metaagent => command
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
    runtime_state: &KernelRuntimeState,
    session_projection: &SessionStateProjectionStore,
    command: &KernelCommand,
    request: &LocalDaemonRequest,
) -> Result<String, DaemonError> {
    if matches!(command.caller.caller_kind, KernelCallerKind::HostedService)
        && !hosted_service_request_is_authorized(command, request)
    {
        return Err(DaemonError::LocalTransport {
            operation: "authorize hosted service request",
            message: "hosted service identity is not authorized for this request".to_string(),
        });
    }
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
            ensure_session_member(
                runtime_state,
                session_projection,
                command,
                &session_id,
                &user_id,
            )
            .await?;
            Ok(user_id)
        }
        SessionMembershipScope::SessionIds(session_ids) => {
            for session_id in session_ids {
                ensure_session_member(
                    runtime_state,
                    session_projection,
                    command,
                    &session_id,
                    &user_id,
                )
                .await?;
            }
            Ok(user_id)
        }
        SessionMembershipScope::SessionRef {
            session_ref,
            workspace_id,
        } => {
            let session = resolve_session_for_membership(
                runtime_state,
                session_projection,
                &session_ref,
                workspace_id.as_deref(),
            )
            .await?;
            ensure_not_leased_backing(runtime_state, &session).await?;
            if !is_member(runtime_state, command, &session, &user_id) {
                return Err(DaemonError::SessionAccessDenied {
                    session_id: session.id().to_string(),
                    user_id,
                });
            }
            Ok(user_id)
        }
        SessionMembershipScope::DeleteSessionRef {
            session_ref,
            workspace_id,
        } => {
            let session_id = runtime_state
                .resolve_session_ref_id_for_delete(&session_ref, workspace_id.as_deref())
                .await?;
            let session = runtime_state.session_snapshot(&session_id).await?;
            ensure_not_leased_backing(runtime_state, &session).await?;
            if !is_member(runtime_state, command, &session, &user_id) {
                return Err(DaemonError::SessionAccessDenied {
                    session_id,
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
                runtime_state.attachment_session_id(&attachment_id).await?
            };
            ensure_session_member(
                runtime_state,
                session_projection,
                command,
                &session_id,
                &user_id,
            )
            .await?;
            Ok(user_id)
        }
    }
}

/// The identity the addressed room stores for an admitted caller, for the projections that
/// show the room to its member (what that member may see). Ownership checks never use it:
/// they keep the authenticated identity and relate it to the owner with `acts_for_owner`.
pub(crate) fn request_room_member_user_id(
    runtime_state: &KernelRuntimeState,
    command: &KernelCommand,
    request: &LocalDaemonRequest,
    user_id: &str,
) -> String {
    if !command.is_terminal_caller() && !command.is_sudo_command() {
        return user_id.to_string();
    }
    // A provider run carries no session scope of its own; it belongs to the room it ran in.
    if let LocalDaemonRequest::GetProviderRun(run_request) = request {
        return runtime_state
            .provider_run_session_id(&run_request.provider_run_id)
            .map_or_else(
                || user_id.to_string(),
                |session_id| runtime_state.room_member_user_id(&session_id, user_id),
            );
    }
    match request_session_scope(request) {
        Some(SessionMembershipScope::SessionId(session_id)) => {
            runtime_state.room_member_user_id(&session_id, user_id)
        }
        _ => user_id.to_string(),
    }
}

fn hosted_service_request_is_authorized(
    command: &KernelCommand,
    request: &LocalDaemonRequest,
) -> bool {
    if command.source != KernelCommandSource::RelayClient {
        return false;
    }
    match request {
        LocalDaemonRequest::RequestCredentialEnrollmentInteraction(_) => true,
        LocalDaemonRequest::RespondToInteraction(request) => {
            let Some(enrollment_id) = command
                .caller
                .caller_id
                .strip_prefix(DEPLOYMENT_CREDENTIAL_ENROLLMENT_SERVICE_SUBJECT_PREFIX)
                .filter(|enrollment_id| !enrollment_id.is_empty())
            else {
                return false;
            };
            request.choice_id == "cancel"
                && request.custom_reply.is_none()
                && request.interaction_id
                    == deployment_credential_enrollment_interaction_id(enrollment_id)
        }
        _ => false,
    }
}

/// Membership of the room as the caller's own identity, or as the room's `local` member when the
/// caller is the enrolled owner's terminal or an admitted sudo command.
fn is_member(
    runtime_state: &KernelRuntimeState,
    command: &KernelCommand,
    session: &crate::session::RuntimeSession,
    user_id: &str,
) -> bool {
    // Sudo admission rechecks the exact live grant before this membership check.
    if command.is_terminal_caller() || command.is_sudo_command() {
        runtime_state.is_room_member(session, user_id)
    } else {
        session.has_member(user_id)
    }
}

/// The home kernel is the session authority for a leased agent, so clients never reach its
/// backing session on the worker. The same hidden flag that keeps the session out of lists marks it.
async fn ensure_not_leased_backing(
    runtime_state: &KernelRuntimeState,
    session: &crate::session::RuntimeSession,
) -> Result<(), DaemonError> {
    if !session.is_hidden() {
        return Ok(());
    }
    let session_id = session.id().to_string();
    let backing_session_id = session_id.clone();
    let home_session_id = runtime_state
        .with_app_side_effect(move |app| {
            crate::app::RemoteLeaseRuntime::new(app)
                .leased_backing_home_session(&backing_session_id)
        })
        .await;
    match home_session_id {
        Some(home_session_id) => Err(DaemonError::LeasedBackingSessionRefused {
            session_id,
            home_session_id,
        }),
        None => Ok(()),
    }
}

async fn ensure_session_member(
    runtime_state: &KernelRuntimeState,
    session_projection: &SessionStateProjectionStore,
    command: &KernelCommand,
    session_id: &str,
    user_id: &str,
) -> Result<(), DaemonError> {
    if let Some(session) = session_projection.get(session_id) {
        ensure_not_leased_backing(runtime_state, &session).await?;
        if is_member(runtime_state, command, &session, user_id) {
            return Ok(());
        }
        return Err(DaemonError::SessionAccessDenied {
            session_id: session.id().to_string(),
            user_id: user_id.to_string(),
        });
    }
    let session = runtime_state.session_snapshot(session_id).await?;
    ensure_not_leased_backing(runtime_state, &session).await?;
    if is_member(runtime_state, command, &session, user_id) {
        Ok(())
    } else {
        Err(DaemonError::SessionAccessDenied {
            session_id: session.id().to_string(),
            user_id: user_id.to_string(),
        })
    }
}

async fn resolve_session_for_membership(
    runtime_state: &KernelRuntimeState,
    session_projection: &SessionStateProjectionStore,
    session_ref: &str,
    workspace_id: Option<&str>,
) -> Result<crate::session::RuntimeSession, DaemonError> {
    if let Some(session) = session_projection.resolve_session_ref(session_ref, workspace_id) {
        return Ok(session);
    }
    let session_id = runtime_state
        .resolve_session_ref_id(session_ref, workspace_id)
        .await?;
    runtime_state.session_snapshot(&session_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::local::ListSessionsRequest;
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
}
