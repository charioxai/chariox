use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use rand::RngCore;
use tokio::sync::Mutex;

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::local::{
    AttachWorkspaceLinkRequest, CreateSessionInviteRequest, CreateWorkspaceLinkRequest,
    DetachWorkspaceLinkRequest, JoinSessionInviteRequest, ListSessionMembersRequest,
    ListWorkspaceLinksRequest, LocalDaemonRequest, LocalDaemonResponse, RevokeSessionInviteRequest,
    SessionInviteRecord, ShowWorkspaceLinkRequest,
};
use crate::runtime::command::KernelCommand;
use crate::runtime::invite_tokens::{
    decode_session_invite_token, encode_session_invite_token, SessionInviteToken,
};
use crate::runtime::projection::{DaemonConfigProjectionStore, SessionStateProjectionStore};
use crate::session::DEFAULT_LOCAL_USER_ID;

pub(crate) async fn execute_session_collaboration_request(
    app: &Arc<Mutex<DaemonApp>>,
    session_projection: &SessionStateProjectionStore,
    config_projection: &DaemonConfigProjectionStore,
    command: &KernelCommand,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::ListSessionMembers(request) => {
            execute_list_session_members_request(app, request).await
        }
        LocalDaemonRequest::CreateSessionInvite(request) => {
            execute_create_session_invite_request(app, session_projection, command, request).await
        }
        LocalDaemonRequest::JoinSessionInvite(request) => {
            execute_join_session_invite_request(app, session_projection, request).await
        }
        LocalDaemonRequest::RevokeSessionInvite(request) => {
            execute_revoke_session_invite_request(app, session_projection, request).await
        }
        LocalDaemonRequest::CreateWorkspaceLink(request) => {
            execute_create_workspace_link_request(app, session_projection, command, request).await
        }
        LocalDaemonRequest::ListWorkspaceLinks(request) => {
            execute_list_workspace_links_request(app, request).await
        }
        LocalDaemonRequest::ShowWorkspaceLink(request) => {
            execute_show_workspace_link_request(app, request).await
        }
        LocalDaemonRequest::AttachWorkspaceLink(request) => {
            let config = config_projection.snapshot();
            execute_attach_workspace_link_request(
                app,
                session_projection,
                command,
                config.host_machine_id,
                config.daemon_id,
                request,
            )
            .await
        }
        LocalDaemonRequest::DetachWorkspaceLink(request) => {
            execute_detach_workspace_link_request(app, session_projection, command, request).await
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "session collaboration request",
            message: "unsupported session collaboration request".to_string(),
        }),
    }
}

pub(crate) async fn execute_list_session_members_request(
    app: &Arc<Mutex<DaemonApp>>,
    request: ListSessionMembersRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let app = app.lock().await;
    let (members, invites) = app.sessions().list_session_members(&request.session_id)?;
    Ok(LocalDaemonResponse::SessionMembersListed { members, invites })
}

pub(crate) async fn execute_create_session_invite_request(
    app: &Arc<Mutex<DaemonApp>>,
    session_projection: &SessionStateProjectionStore,
    command: &KernelCommand,
    request: CreateSessionInviteRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let now_ms = current_unix_ms();
    let expires_at_ms = request
        .expires_in_ms
        .map(|expires_in_ms| now_ms.saturating_add(expires_in_ms));
    let invite_id = random_hex_id();
    let created_by_user_id = command_caller_user_id(command);
    let (session, invite) = {
        let app = app.lock().await;
        let result = app.sessions_mut().create_session_invite(
            &request.session_id,
            invite_id,
            created_by_user_id,
            expires_at_ms,
            request.max_uses.or(Some(1)),
        )?;
        result
    };
    let invite_token = encode_session_invite_token(&SessionInviteToken {
        version: 1,
        session_id: session.id().to_string(),
        invite_id: invite.invite_id().to_string(),
        created_by_user_id: invite.created_by_user_id().to_string(),
        issued_at_ms: invite.created_at_ms(),
        expires_at_ms: invite.expires_at_ms(),
        max_uses: invite.max_uses(),
    })?;
    session_projection.update(session.clone());
    Ok(LocalDaemonResponse::SessionInviteCreated {
        invite: SessionInviteRecord {
            invite,
            invite_token,
        },
        session,
    })
}

pub(crate) async fn execute_join_session_invite_request(
    app: &Arc<Mutex<DaemonApp>>,
    session_projection: &SessionStateProjectionStore,
    request: JoinSessionInviteRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let token = decode_session_invite_token(&request.invite_token)?;
    let now_ms = current_unix_ms();
    if token
        .expires_at_ms
        .is_some_and(|expires_at_ms| expires_at_ms <= now_ms)
    {
        return Err(DaemonError::LocalTransport {
            operation: "join session invite",
            message: "session invite is expired".to_string(),
        });
    }
    let (session, member) = {
        let app = app.lock().await;
        let result = app.sessions_mut().join_session_invite(
            &token.session_id,
            &token.invite_id,
            request.user_id,
            now_ms,
        )?;
        result
    };
    session_projection.update(session.clone());
    Ok(LocalDaemonResponse::SessionInviteJoined { member, session })
}

pub(crate) async fn execute_revoke_session_invite_request(
    app: &Arc<Mutex<DaemonApp>>,
    session_projection: &SessionStateProjectionStore,
    request: RevokeSessionInviteRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let (session, invite) = {
        let app = app.lock().await;
        let result = app
            .sessions_mut()
            .revoke_session_invite(&request.session_id, &request.invite_ref)?;
        result
    };
    session_projection.update(session.clone());
    Ok(LocalDaemonResponse::SessionInviteRevoked { invite, session })
}

pub(crate) async fn execute_create_workspace_link_request(
    app: &Arc<Mutex<DaemonApp>>,
    session_projection: &SessionStateProjectionStore,
    command: &KernelCommand,
    request: CreateWorkspaceLinkRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let created_by_user_id = command_caller_user_id(command);
    let (session, link) = {
        let app = app.lock().await;
        let result = app.sessions_mut().create_workspace_link(
            &request.session_id,
            request.name,
            created_by_user_id,
        )?;
        result
    };
    session_projection.update(session.clone());
    Ok(LocalDaemonResponse::WorkspaceLinkCreated { link, session })
}

pub(crate) async fn execute_list_workspace_links_request(
    app: &Arc<Mutex<DaemonApp>>,
    request: ListWorkspaceLinksRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let app = app.lock().await;
    let links = app.sessions().list_workspace_links(&request.session_id)?;
    Ok(LocalDaemonResponse::WorkspaceLinksListed { links })
}

pub(crate) async fn execute_show_workspace_link_request(
    app: &Arc<Mutex<DaemonApp>>,
    request: ShowWorkspaceLinkRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let app = app.lock().await;
    let link = app
        .sessions()
        .resolve_workspace_link_ref(&request.session_id, &request.link_ref)?;
    Ok(LocalDaemonResponse::WorkspaceLinkShown { link })
}

pub(crate) async fn execute_attach_workspace_link_request(
    app: &Arc<Mutex<DaemonApp>>,
    session_projection: &SessionStateProjectionStore,
    command: &KernelCommand,
    host_machine_id: String,
    kernel_id: String,
    request: AttachWorkspaceLinkRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let user_id = command_caller_user_id(command);
    let machine_id = command.caller.machine_id.clone().unwrap_or(host_machine_id);
    let repo_root = if let Some(repo_root) = request.repo_root {
        repo_root
    } else {
        let app = app.lock().await;
        app.sessions()
            .get_session(&request.session_id)?
            .worktree_id()
            .to_string()
    };
    let (session, link, attachment) = {
        let app = app.lock().await;
        let result = app.sessions_mut().attach_workspace_link(
            &request.session_id,
            &request.link_ref,
            user_id,
            machine_id,
            kernel_id,
            repo_root,
            request.branch,
            request.repo_fingerprint,
        )?;
        result
    };
    session_projection.update(session.clone());
    Ok(LocalDaemonResponse::WorkspaceLinkAttached {
        link,
        attachment,
        session,
    })
}

pub(crate) async fn execute_detach_workspace_link_request(
    app: &Arc<Mutex<DaemonApp>>,
    session_projection: &SessionStateProjectionStore,
    command: &KernelCommand,
    request: DetachWorkspaceLinkRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let user_id = command_caller_user_id(command);
    let repo_root = request.repo_root.as_deref().map(std::path::Path::new);
    let (session, link, detached) = {
        let app = app.lock().await;
        let result = app.sessions_mut().detach_workspace_link(
            &request.session_id,
            &request.link_ref,
            user_id,
            repo_root,
        )?;
        result
    };
    session_projection.update(session.clone());
    Ok(LocalDaemonResponse::WorkspaceLinkDetached {
        link,
        detached,
        session,
    })
}

fn command_caller_user_id(command: &KernelCommand) -> String {
    command
        .caller
        .user_id
        .clone()
        .unwrap_or_else(|| DEFAULT_LOCAL_USER_ID.to_string())
}

fn random_hex_id() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
