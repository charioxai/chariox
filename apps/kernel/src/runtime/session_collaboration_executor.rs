use std::time::{SystemTime, UNIX_EPOCH};

use rand::RngCore;

use crate::error::DaemonError;
use crate::local::{
    AttachWorkspaceLinkRequest, CreateSessionInviteRequest, CreateWorkspaceLinkRequest,
    DetachWorkspaceLinkRequest, JoinSessionInviteRequest, ListSessionMembersRequest,
    ListWorkspaceLinksRequest, LocalDaemonRequest, LocalDaemonResponse, RevokeSessionInviteRequest,
    SessionInviteRecord, ShowWorkspaceLinkRequest,
};
use crate::runtime::command::{command_caller_user_id, KernelCommand};
use crate::runtime::invite_tokens::{
    decode_session_invite_token, encode_session_invite_token, SessionInviteToken,
};
use crate::runtime::projection::DaemonConfigProjectionStore;
use crate::runtime::state::KernelRuntimeState;

pub(crate) async fn execute_session_collaboration_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    command: &KernelCommand,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::ListSessionMembers(request) => {
            execute_list_session_members_request(runtime_state, request).await
        }
        LocalDaemonRequest::CreateSessionInvite(request) => {
            execute_create_session_invite_request(runtime_state, command, request).await
        }
        LocalDaemonRequest::JoinSessionInvite(request) => {
            execute_join_session_invite_request(runtime_state, request).await
        }
        LocalDaemonRequest::RevokeSessionInvite(request) => {
            execute_revoke_session_invite_request(runtime_state, request).await
        }
        LocalDaemonRequest::CreateWorkspaceLink(request) => {
            execute_create_workspace_link_request(runtime_state, command, request).await
        }
        LocalDaemonRequest::ListWorkspaceLinks(request) => {
            execute_list_workspace_links_request(runtime_state, request).await
        }
        LocalDaemonRequest::ShowWorkspaceLink(request) => {
            execute_show_workspace_link_request(runtime_state, request).await
        }
        LocalDaemonRequest::AttachWorkspaceLink(request) => {
            let config = config_projection.snapshot();
            execute_attach_workspace_link_request(
                runtime_state,
                command,
                config.host_machine_id,
                config.daemon_id,
                request,
            )
            .await
        }
        LocalDaemonRequest::DetachWorkspaceLink(request) => {
            execute_detach_workspace_link_request(runtime_state, command, request).await
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "session collaboration request",
            message: "unsupported session collaboration request".to_string(),
        }),
    }
}

pub(crate) async fn execute_list_session_members_request(
    runtime_state: &KernelRuntimeState,
    request: ListSessionMembersRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let (members, invites) = runtime_state.list_session_members(&request.session_id)?;
    Ok(LocalDaemonResponse::SessionMembersListed { members, invites })
}

pub(crate) async fn execute_create_session_invite_request(
    runtime_state: &KernelRuntimeState,
    command: &KernelCommand,
    request: CreateSessionInviteRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let now_ms = current_unix_ms();
    let expires_at_ms = request
        .expires_in_ms
        .map(|expires_in_ms| now_ms.saturating_add(expires_in_ms));
    let invite_id = random_hex_id();
    let created_by_user_id = command_caller_user_id(command);
    let (session, invite) = runtime_state.create_session_invite(
        &request.session_id,
        invite_id,
        created_by_user_id,
        expires_at_ms,
        request.max_uses.or(Some(1)),
        request.collaboration_level,
    )?;
    let invite_token = encode_session_invite_token(&SessionInviteToken {
        version: 1,
        session_id: session.id().to_string(),
        invite_id: invite.invite_id().to_string(),
        created_by_user_id: invite.created_by_user_id().to_string(),
        issued_at_ms: invite.created_at_ms(),
        expires_at_ms: invite.expires_at_ms(),
        max_uses: invite.max_uses(),
    })?;
    Ok(LocalDaemonResponse::SessionInviteCreated {
        invite: SessionInviteRecord {
            invite,
            invite_token,
        },
        session,
    })
}

pub(crate) async fn execute_join_session_invite_request(
    runtime_state: &KernelRuntimeState,
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
    let (session, member) = runtime_state.join_session_invite(
        &token.session_id,
        &token.invite_id,
        request.user_id,
        now_ms,
    )?;
    Ok(LocalDaemonResponse::SessionInviteJoined { member, session })
}

pub(crate) async fn execute_revoke_session_invite_request(
    runtime_state: &KernelRuntimeState,
    request: RevokeSessionInviteRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let (session, invite) =
        runtime_state.revoke_session_invite(&request.session_id, &request.invite_ref)?;
    Ok(LocalDaemonResponse::SessionInviteRevoked { invite, session })
}

pub(crate) async fn execute_create_workspace_link_request(
    runtime_state: &KernelRuntimeState,
    command: &KernelCommand,
    request: CreateWorkspaceLinkRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let created_by_user_id = command_caller_user_id(command);
    let (session, link) = runtime_state.create_workspace_link(
        &request.session_id,
        request.name,
        created_by_user_id,
    )?;
    Ok(LocalDaemonResponse::WorkspaceLinkCreated { link, session })
}

pub(crate) async fn execute_list_workspace_links_request(
    runtime_state: &KernelRuntimeState,
    request: ListWorkspaceLinksRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let links = runtime_state.list_workspace_links(&request.session_id)?;
    Ok(LocalDaemonResponse::WorkspaceLinksListed { links })
}

pub(crate) async fn execute_show_workspace_link_request(
    runtime_state: &KernelRuntimeState,
    request: ShowWorkspaceLinkRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let link = runtime_state.resolve_workspace_link_ref(&request.session_id, &request.link_ref)?;
    Ok(LocalDaemonResponse::WorkspaceLinkShown { link })
}

pub(crate) async fn execute_attach_workspace_link_request(
    runtime_state: &KernelRuntimeState,
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
        runtime_state
            .session_snapshot(&request.session_id)
            .await?
            .worktree_id()
            .to_string()
    };
    let (session, link, attachment) = runtime_state.attach_workspace_link(
        &request.session_id,
        &request.link_ref,
        user_id,
        machine_id,
        kernel_id,
        repo_root,
        request.branch,
        request.repo_fingerprint,
    )?;
    Ok(LocalDaemonResponse::WorkspaceLinkAttached {
        link,
        attachment,
        session,
    })
}

pub(crate) async fn execute_detach_workspace_link_request(
    runtime_state: &KernelRuntimeState,
    command: &KernelCommand,
    request: DetachWorkspaceLinkRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let user_id = command_caller_user_id(command);
    let repo_root = request.repo_root.as_deref().map(std::path::Path::new);
    let (session, link, detached) = runtime_state.detach_workspace_link(
        &request.session_id,
        &request.link_ref,
        user_id,
        repo_root,
    )?;
    Ok(LocalDaemonResponse::WorkspaceLinkDetached {
        link,
        detached,
        session,
    })
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
