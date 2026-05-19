use crate::error::DaemonError;
use crate::local::LocalDaemonRequest;
use crate::runtime::command::{KernelCallerKind, KernelCommand};
use crate::runtime::projection::SessionStateProjectionStore;
use crate::runtime::state::KernelRuntimeState;
use crate::session::DEFAULT_LOCAL_USER_ID;

mod scope;
use scope::{request_session_scope, SessionMembershipScope};

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
    runtime_state: &KernelRuntimeState,
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
            ensure_session_member(runtime_state, session_projection, &session_id, &user_id).await?;
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
                runtime_state.attachment_session_id(&attachment_id).await?
            };
            ensure_session_member(runtime_state, session_projection, &session_id, &user_id).await?;
            Ok(user_id)
        }
    }
}

async fn ensure_session_member(
    runtime_state: &KernelRuntimeState,
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
    let session = runtime_state.session_snapshot(session_id).await?;
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
