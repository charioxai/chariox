use std::time::{SystemTime, UNIX_EPOCH};

use rand::RngCore;

use crate::error::DaemonError;
use crate::local::{
    CreateSessionInviteRequest, JoinSessionInviteRequest, ListSessionMembersRequest,
    LocalDaemonRequest, LocalDaemonResponse, RevokeSessionInviteRequest, SessionInviteRecord,
};
use crate::runtime::command::{command_caller_user_id, KernelCommand};
use crate::runtime::invite_tokens::{
    decode_session_invite_token, encode_session_invite_token, SessionInviteToken,
};
use crate::runtime::projection::DaemonConfigProjectionStore;
use crate::runtime::state::KernelRuntimeState;
use workspace_live_sync::{
    execute_attach_workspace_link_request, execute_create_workspace_link_request,
    execute_detach_workspace_link_request, execute_get_workspace_live_sync_status_request,
    execute_list_workspace_links_request, execute_show_workspace_link_request,
};

mod workspace_live_sync;

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
            let default_workspace_live_sync_mode =
                config.provider_workspace_live_sync_mode("default");
            execute_attach_workspace_link_request(
                runtime_state,
                command,
                config.host_machine_id,
                config.daemon_id,
                default_workspace_live_sync_mode,
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

#[cfg(test)]
mod tests {
    use super::workspace_live_sync::{
        workspace_live_sync_conflicts_from_results, workspace_live_sync_footer_state,
        workspace_live_sync_latest_path_results, workspace_live_sync_target_status_from_results,
    };
    use crate::local::{WorkspaceLiveSyncFooterState, WorkspaceLiveSyncTargetState};

    #[test]
    fn workspace_live_sync_footer_state_reports_syncing_for_active_managed_or_tracked_work() {
        assert_eq!(
            workspace_live_sync_footer_state(
                crate::config::WorkspaceLiveSyncMode::Managed,
                true,
                false,
                false,
            ),
            WorkspaceLiveSyncFooterState::Syncing
        );
        assert_eq!(
            workspace_live_sync_footer_state(
                crate::config::WorkspaceLiveSyncMode::Tracked,
                true,
                false,
                false,
            ),
            WorkspaceLiveSyncFooterState::Syncing
        );
        assert_eq!(
            workspace_live_sync_footer_state(
                crate::config::WorkspaceLiveSyncMode::Unrestricted,
                true,
                false,
                false,
            ),
            WorkspaceLiveSyncFooterState::Off
        );
    }

    #[test]
    fn workspace_live_sync_footer_state_prioritizes_conflict_and_degraded() {
        assert_eq!(
            workspace_live_sync_footer_state(
                crate::config::WorkspaceLiveSyncMode::Managed,
                true,
                true,
                true,
            ),
            WorkspaceLiveSyncFooterState::Conflict
        );
        assert_eq!(
            workspace_live_sync_footer_state(
                crate::config::WorkspaceLiveSyncMode::Tracked,
                true,
                false,
                true,
            ),
            WorkspaceLiveSyncFooterState::Degraded
        );
    }

    #[test]
    fn workspace_live_sync_status_uses_latest_result_for_target_path() {
        let results = vec![
            target_result(
                "link-1",
                "/repo/target",
                "agent-a",
                "src/lib.rs",
                crate::git_observer::WorkspaceLiveSyncApplyStatus::SkippedConflict,
                "overlap",
            ),
            target_result(
                "link-1",
                "/repo/target",
                "agent-b",
                "src/lib.rs",
                crate::git_observer::WorkspaceLiveSyncApplyStatus::Applied,
                "applied after reconciliation",
            ),
        ];

        assert_eq!(
            workspace_live_sync_target_status_from_results(&results, "link-1", "/repo/target"),
            WorkspaceLiveSyncTargetState::Ready
        );
        assert!(workspace_live_sync_conflicts_from_results(&results).is_empty());
        assert!(!workspace_live_sync_latest_path_results(&results)
            .into_iter()
            .any(|(_, path_result)| {
                path_result.status == crate::git_observer::WorkspaceLiveSyncApplyStatus::FailedIo
            }));
    }

    #[test]
    fn workspace_live_sync_status_keeps_latest_unresolved_conflict() {
        let results = vec![
            target_result(
                "link-1",
                "/repo/target",
                "agent-a",
                "src/lib.rs",
                crate::git_observer::WorkspaceLiveSyncApplyStatus::Applied,
                "applied first",
            ),
            target_result(
                "link-1",
                "/repo/target",
                "agent-b",
                "src/lib.rs",
                crate::git_observer::WorkspaceLiveSyncApplyStatus::SkippedConflict,
                "new overlap",
            ),
        ];

        assert_eq!(
            workspace_live_sync_target_status_from_results(&results, "link-1", "/repo/target"),
            WorkspaceLiveSyncTargetState::Conflict
        );
        let conflicts = workspace_live_sync_conflicts_from_results(&results);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].source_agent_id, "agent-b");
        assert_eq!(conflicts[0].path, "src/lib.rs");
    }

    #[test]
    fn workspace_live_sync_target_status_is_scoped_by_link_and_repo_root() {
        let results = vec![
            target_result(
                "link-1",
                "/repo/shared-target",
                "agent-a",
                "src/lib.rs",
                crate::git_observer::WorkspaceLiveSyncApplyStatus::SkippedConflict,
                "overlap",
            ),
            target_result(
                "link-2",
                "/repo/shared-target",
                "agent-b",
                "src/lib.rs",
                crate::git_observer::WorkspaceLiveSyncApplyStatus::Applied,
                "applied cleanly",
            ),
        ];

        assert_eq!(
            workspace_live_sync_target_status_from_results(
                &results,
                "link-1",
                "/repo/shared-target",
            ),
            WorkspaceLiveSyncTargetState::Conflict
        );
        assert_eq!(
            workspace_live_sync_target_status_from_results(
                &results,
                "link-2",
                "/repo/shared-target",
            ),
            WorkspaceLiveSyncTargetState::Ready
        );
    }

    #[test]
    fn workspace_live_sync_degraded_footer_uses_latest_path_result() {
        let unresolved = vec![
            target_result(
                "link-1",
                "/repo/target",
                "agent-a",
                "src/lib.rs",
                crate::git_observer::WorkspaceLiveSyncApplyStatus::FailedIo,
                "permission denied",
            ),
            target_result(
                "link-1",
                "/repo/other-target",
                "agent-a",
                "src/lib.rs",
                crate::git_observer::WorkspaceLiveSyncApplyStatus::Applied,
                "applied cleanly",
            ),
        ];
        let resolved = vec![
            target_result(
                "link-1",
                "/repo/target",
                "agent-a",
                "src/lib.rs",
                crate::git_observer::WorkspaceLiveSyncApplyStatus::FailedIo,
                "permission denied",
            ),
            target_result(
                "link-1",
                "/repo/target",
                "agent-b",
                "src/lib.rs",
                crate::git_observer::WorkspaceLiveSyncApplyStatus::Applied,
                "applied after retry",
            ),
        ];

        let unresolved_degraded = workspace_live_sync_latest_path_results(&unresolved)
            .into_iter()
            .any(|(_, path_result)| {
                path_result.status == crate::git_observer::WorkspaceLiveSyncApplyStatus::FailedIo
            });
        let resolved_degraded = workspace_live_sync_latest_path_results(&resolved)
            .into_iter()
            .any(|(_, path_result)| {
                path_result.status == crate::git_observer::WorkspaceLiveSyncApplyStatus::FailedIo
            });

        assert!(unresolved_degraded);
        assert!(!resolved_degraded);
    }

    fn target_result(
        link_id: &str,
        target_repo_root: &str,
        source_agent_id: &str,
        path: &str,
        status: crate::git_observer::WorkspaceLiveSyncApplyStatus,
        message: &str,
    ) -> crate::git_observer::WorkspaceLiveSyncTargetResult {
        crate::git_observer::WorkspaceLiveSyncTargetResult {
            session_id: "session-1".to_string(),
            link_id: link_id.to_string(),
            link_name: "shared".to_string(),
            source_agent_id: source_agent_id.to_string(),
            source_worktree_path: "/repo/source".to_string(),
            target_user_id: "user-2".to_string(),
            target_machine_id: "machine-2".to_string(),
            target_kernel_id: "kernel-2".to_string(),
            target_repo_root: target_repo_root.to_string(),
            path_results: vec![crate::git_observer::WorkspaceLiveSyncPathApplyResult {
                path: path.to_string(),
                status,
                message: message.to_string(),
            }],
        }
    }
}
