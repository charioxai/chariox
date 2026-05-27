use std::time::{SystemTime, UNIX_EPOCH};

use rand::RngCore;

use crate::error::DaemonError;
use crate::local::{
    AttachWorkspaceLinkRequest, CreateSessionInviteRequest, CreateWorkspaceLinkRequest,
    DetachWorkspaceLinkRequest, GetWorkspaceLiveSyncStatusRequest, JoinSessionInviteRequest,
    ListSessionMembersRequest, ListWorkspaceLinksRequest, LocalDaemonRequest, LocalDaemonResponse,
    RevokeSessionInviteRequest, SessionInviteRecord, ShowWorkspaceLinkRequest,
    WorkspaceLiveSyncConflictSummary, WorkspaceLiveSyncFooterState, WorkspaceLiveSyncIgnoreStatus,
    WorkspaceLiveSyncStatus, WorkspaceLiveSyncTargetState, WorkspaceLiveSyncTargetStatus,
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
        LocalDaemonRequest::GetWorkspaceLiveSyncStatus(request) => {
            execute_get_workspace_live_sync_status_request(
                runtime_state,
                config_projection,
                request,
            )
            .await
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

pub(crate) async fn execute_get_workspace_live_sync_status_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    request: GetWorkspaceLiveSyncStatusRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let mode = config_projection
        .snapshot()
        .provider_workspace_live_sync_mode("default");
    let links = runtime_state.list_workspace_links(&request.session_id)?;
    let target_results = runtime_state.workspace_live_sync_target_results(&request.session_id);
    let mut targets = Vec::new();
    for link in links {
        for attachment in link.attachments() {
            let result_status = workspace_live_sync_target_status_from_results(
                &target_results,
                attachment.repo_root(),
            );
            targets.push(WorkspaceLiveSyncTargetStatus {
                link_id: link.link_id().to_string(),
                link_name: link.name().to_string(),
                user_id: attachment.user_id().to_string(),
                machine_id: attachment.machine_id().to_string(),
                kernel_id: attachment.kernel_id().to_string(),
                repo_root: attachment.repo_root().to_string(),
                branch: attachment.branch().map(str::to_string),
                repo_fingerprint: attachment.repo_fingerprint().map(str::to_string),
                status: result_status,
                attached_at_ms: attachment.attached_at_ms(),
            });
        }
    }
    let conflicts = workspace_live_sync_conflicts_from_results(&target_results);
    let degraded = target_results.iter().any(|target_result| {
        target_result.path_results.iter().any(|path_result| {
            path_result.status == crate::git_observer::TrackedWorkspaceLiveSyncApplyStatus::FailedIo
        })
    });
    let footer_state = if !conflicts.is_empty() {
        WorkspaceLiveSyncFooterState::Conflict
    } else if degraded {
        WorkspaceLiveSyncFooterState::Degraded
    } else {
        match mode {
            crate::config::WorkspaceLiveSyncMode::Managed => WorkspaceLiveSyncFooterState::Managed,
            crate::config::WorkspaceLiveSyncMode::Tracked => WorkspaceLiveSyncFooterState::Tracked,
            crate::config::WorkspaceLiveSyncMode::Unrestricted => WorkspaceLiveSyncFooterState::Off,
        }
    };
    Ok(LocalDaemonResponse::WorkspaceLiveSyncStatus {
        status: WorkspaceLiveSyncStatus {
            session_id: request.session_id,
            mode,
            footer_state,
            targets,
            conflicts,
            ignore: WorkspaceLiveSyncIgnoreStatus {
                ignore_file: Some(".arrobaignore".to_string()),
                force_excludes: vec![
                    ".git/**".to_string(),
                    ".arroba/**".to_string(),
                    ".arrobaignore".to_string(),
                    ".env*".to_string(),
                    "node_modules/**".to_string(),
                    "target/**".to_string(),
                    ".next/**".to_string(),
                    "dist/**".to_string(),
                    "build/**".to_string(),
                    ".venv/**".to_string(),
                    "venv/**".to_string(),
                    "__pycache__/**".to_string(),
                    ".pytest_cache/**".to_string(),
                ],
            },
        },
    })
}

fn workspace_live_sync_target_status_from_results(
    target_results: &[crate::git_observer::TrackedWorkspaceLiveSyncTargetResult],
    repo_root: &str,
) -> WorkspaceLiveSyncTargetState {
    let mut has_failure = false;
    for target_result in target_results
        .iter()
        .filter(|result| result.target_repo_root == repo_root)
    {
        for path_result in &target_result.path_results {
            match path_result.status {
                crate::git_observer::TrackedWorkspaceLiveSyncApplyStatus::Applied => {}
                crate::git_observer::TrackedWorkspaceLiveSyncApplyStatus::SkippedConflict => {
                    return WorkspaceLiveSyncTargetState::Conflict;
                }
                crate::git_observer::TrackedWorkspaceLiveSyncApplyStatus::FailedIo => {
                    has_failure = true;
                }
            }
        }
    }
    if has_failure {
        WorkspaceLiveSyncTargetState::Degraded
    } else {
        WorkspaceLiveSyncTargetState::Ready
    }
}

fn workspace_live_sync_conflicts_from_results(
    target_results: &[crate::git_observer::TrackedWorkspaceLiveSyncTargetResult],
) -> Vec<WorkspaceLiveSyncConflictSummary> {
    let mut conflicts = Vec::new();
    for target_result in target_results {
        for path_result in &target_result.path_results {
            if path_result.status
                != crate::git_observer::TrackedWorkspaceLiveSyncApplyStatus::SkippedConflict
            {
                continue;
            }
            conflicts.push(WorkspaceLiveSyncConflictSummary {
                conflict_id: format!(
                    "{}:{}:{}",
                    target_result.link_id, target_result.target_repo_root, path_result.path
                ),
                link_id: target_result.link_id.clone(),
                source_agent_id: target_result.source_agent_id.clone(),
                target_user_id: target_result.target_user_id.clone(),
                target_repo_root: target_result.target_repo_root.clone(),
                path: path_result.path.clone(),
                next_action: format!(
                    "{}. Reread the target and ask a resolver agent to reconcile.",
                    path_result.message
                ),
            });
        }
    }
    conflicts
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
