use std::collections::BTreeSet;

use crate::config::WorkspaceLiveSyncMode;
use crate::error::DaemonError;
use crate::local::{
    AttachWorkspaceLinkRequest, CreateWorkspaceLinkRequest, DetachWorkspaceLinkRequest,
    GetWorkspaceLiveSyncStatusRequest, ListWorkspaceLinksRequest, LocalDaemonResponse,
    ShowWorkspaceLinkRequest, WorkspaceLiveSyncConflictSummary, WorkspaceLiveSyncFooterState,
    WorkspaceLiveSyncGroupStatus, WorkspaceLiveSyncIgnoreStatus, WorkspaceLiveSyncStatus,
    WorkspaceLiveSyncTargetState, WorkspaceLiveSyncTargetStatus,
};
use crate::runtime::command::{command_caller_user_id, KernelCommand};
use crate::runtime::projection::DaemonConfigProjectionStore;
use crate::runtime::state::KernelRuntimeState;

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
    default_workspace_live_sync_mode: WorkspaceLiveSyncMode,
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
    let repo_root_path = std::path::Path::new(&repo_root);
    validate_workspace_live_sync_attachment_target(repo_root_path)?;
    let branch = request
        .branch
        .or_else(|| crate::git_observer::workspace_live_sync_git_branch(repo_root_path));
    let repo_fingerprint = request
        .repo_fingerprint
        .or_else(|| crate::git_observer::workspace_live_sync_repo_fingerprint(repo_root_path));
    let (session, link, attachment) = runtime_state.attach_workspace_link(
        &request.session_id,
        &request.link_ref,
        user_id,
        machine_id,
        kernel_id,
        repo_root,
        branch,
        repo_fingerprint,
    )?;
    let mode = session
        .workspace_live_sync_mode()
        .unwrap_or(default_workspace_live_sync_mode);
    runtime_state.record_workspace_live_sync_enrollment_notice(
        session.id(),
        link.name(),
        attachment.repo_root(),
        mode,
    );
    Ok(LocalDaemonResponse::WorkspaceLinkAttached {
        link,
        attachment,
        session,
    })
}

fn validate_workspace_live_sync_attachment_target(
    repo_root_path: &std::path::Path,
) -> Result<(), DaemonError> {
    let canonical_root =
        repo_root_path
            .canonicalize()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "attach workspace link",
                message: format!(
                "workspace live sync target `{}` must be an existing Git worktree root: {error}",
                repo_root_path.display()
            ),
            })?;
    if !canonical_root.is_dir() {
        return Err(DaemonError::LocalTransport {
            operation: "attach workspace link",
            message: format!(
                "workspace live sync target `{}` must be a directory",
                repo_root_path.display()
            ),
        });
    }
    let top_level =
        crate::git_observer::git_output(repo_root_path, &["rev-parse", "--show-toplevel"])
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "attach workspace link",
                message: format!(
                    "workspace live sync target `{}` must be a Git worktree root",
                    repo_root_path.display()
                ),
            })?;
    let canonical_top_level = std::path::Path::new(top_level.trim())
        .canonicalize()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "attach workspace link",
            message: format!(
                "workspace live sync target `{}` Git worktree root could not be verified: {error}",
                repo_root_path.display()
            ),
        })?;
    if canonical_root != canonical_top_level {
        return Err(DaemonError::LocalTransport {
            operation: "attach workspace link",
            message: format!(
                "workspace live sync target `{}` must be the Git worktree root `{}`",
                repo_root_path.display(),
                canonical_top_level.display()
            ),
        });
    }
    Ok(())
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
    let session = runtime_state.session_snapshot(&request.session_id).await?;
    let mode = session.workspace_live_sync_mode().unwrap_or_else(|| {
        config_projection
            .snapshot()
            .provider_workspace_live_sync_mode("default")
    });
    let has_prompt_work = session.has_any_prompt_work();
    let links = runtime_state.list_workspace_links(&request.session_id)?;
    let target_results = runtime_state.workspace_live_sync_target_results(&request.session_id);
    let latest_path_results = workspace_live_sync_latest_path_results(&target_results);
    let conflicts = workspace_live_sync_conflicts_from_latest_results(&latest_path_results);
    let degraded = latest_path_results.iter().any(|(_, path_result)| {
        path_result.status == crate::git_observer::WorkspaceLiveSyncApplyStatus::FailedIo
    });
    let mut targets = Vec::new();
    let mut sync_groups = Vec::new();
    for link in links {
        let mut group_targets = Vec::new();
        for attachment in link.attachments() {
            let result_status = workspace_live_sync_target_status_from_latest_results(
                &latest_path_results,
                link.link_id(),
                attachment.repo_root(),
            );
            let target = WorkspaceLiveSyncTargetStatus {
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
            };
            targets.push(target.clone());
            group_targets.push(target);
        }
        sync_groups.push(workspace_live_sync_group_status(
            link.link_id(),
            link.name(),
            &group_targets,
        ));
    }
    let footer_state =
        workspace_live_sync_footer_state(mode, has_prompt_work, !conflicts.is_empty(), degraded);
    Ok(LocalDaemonResponse::WorkspaceLiveSyncStatus {
        status: WorkspaceLiveSyncStatus {
            session_id: request.session_id,
            mode,
            footer_state,
            sync_groups,
            targets,
            conflicts,
            ignore: WorkspaceLiveSyncIgnoreStatus {
                ignore_file: Some(".arrobaignore".to_string()),
                rules: crate::workspace_live_sync_ignore::workspace_live_sync_user_ignore_patterns(
                    std::path::Path::new(session.worktree_id()),
                ),
                force_excludes:
                    crate::workspace_live_sync_ignore::workspace_live_sync_force_exclude_patterns(),
            },
        },
    })
}

pub(super) fn workspace_live_sync_footer_state(
    mode: crate::config::WorkspaceLiveSyncMode,
    has_prompt_work: bool,
    has_conflicts: bool,
    degraded: bool,
) -> WorkspaceLiveSyncFooterState {
    if has_conflicts {
        return WorkspaceLiveSyncFooterState::Conflict;
    }
    if degraded {
        return WorkspaceLiveSyncFooterState::Degraded;
    }
    if has_prompt_work && mode != crate::config::WorkspaceLiveSyncMode::Unrestricted {
        return WorkspaceLiveSyncFooterState::Syncing;
    }
    match mode {
        crate::config::WorkspaceLiveSyncMode::Managed => WorkspaceLiveSyncFooterState::Managed,
        crate::config::WorkspaceLiveSyncMode::Tracked => WorkspaceLiveSyncFooterState::Tracked,
        crate::config::WorkspaceLiveSyncMode::Unrestricted => WorkspaceLiveSyncFooterState::Off,
    }
}

fn workspace_live_sync_group_status(
    group_id: &str,
    group_name: &str,
    targets: &[WorkspaceLiveSyncTargetStatus],
) -> WorkspaceLiveSyncGroupStatus {
    WorkspaceLiveSyncGroupStatus {
        group_id: group_id.to_string(),
        group_name: group_name.to_string(),
        target_count: targets.len(),
        ready_targets: targets
            .iter()
            .filter(|target| target.status == WorkspaceLiveSyncTargetState::Ready)
            .count(),
        degraded_targets: targets
            .iter()
            .filter(|target| target.status == WorkspaceLiveSyncTargetState::Degraded)
            .count(),
        conflicted_targets: targets
            .iter()
            .filter(|target| target.status == WorkspaceLiveSyncTargetState::Conflict)
            .count(),
    }
}

fn workspace_live_sync_target_status_from_latest_results(
    latest_path_results: &[(
        &crate::git_observer::WorkspaceLiveSyncTargetResult,
        &crate::git_observer::WorkspaceLiveSyncPathApplyResult,
    )],
    link_id: &str,
    repo_root: &str,
) -> WorkspaceLiveSyncTargetState {
    let mut has_failure = false;
    for (_, path_result) in latest_path_results.iter().filter(|(target_result, _)| {
        target_result.link_id == link_id && target_result.target_repo_root == repo_root
    }) {
        match path_result.status {
            crate::git_observer::WorkspaceLiveSyncApplyStatus::Applied
            | crate::git_observer::WorkspaceLiveSyncApplyStatus::Rebased => {}
            crate::git_observer::WorkspaceLiveSyncApplyStatus::SkippedConflict => {
                return WorkspaceLiveSyncTargetState::Conflict;
            }
            crate::git_observer::WorkspaceLiveSyncApplyStatus::FailedIo => {
                has_failure = true;
            }
        }
    }
    if has_failure {
        WorkspaceLiveSyncTargetState::Degraded
    } else {
        WorkspaceLiveSyncTargetState::Ready
    }
}

#[cfg(test)]
pub(super) fn workspace_live_sync_target_status_from_results(
    target_results: &[crate::git_observer::WorkspaceLiveSyncTargetResult],
    link_id: &str,
    repo_root: &str,
) -> WorkspaceLiveSyncTargetState {
    let latest_path_results = workspace_live_sync_latest_path_results(target_results);
    workspace_live_sync_target_status_from_latest_results(&latest_path_results, link_id, repo_root)
}

fn workspace_live_sync_conflicts_from_latest_results(
    latest_path_results: &[(
        &crate::git_observer::WorkspaceLiveSyncTargetResult,
        &crate::git_observer::WorkspaceLiveSyncPathApplyResult,
    )],
) -> Vec<WorkspaceLiveSyncConflictSummary> {
    let mut conflicts = Vec::new();
    for (target_result, path_result) in latest_path_results {
        if path_result.status != crate::git_observer::WorkspaceLiveSyncApplyStatus::SkippedConflict
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
    conflicts
}

#[cfg(test)]
pub(super) fn workspace_live_sync_conflicts_from_results(
    target_results: &[crate::git_observer::WorkspaceLiveSyncTargetResult],
) -> Vec<WorkspaceLiveSyncConflictSummary> {
    let latest_path_results = workspace_live_sync_latest_path_results(target_results);
    workspace_live_sync_conflicts_from_latest_results(&latest_path_results)
}

pub(super) fn workspace_live_sync_latest_path_results(
    target_results: &[crate::git_observer::WorkspaceLiveSyncTargetResult],
) -> Vec<(
    &crate::git_observer::WorkspaceLiveSyncTargetResult,
    &crate::git_observer::WorkspaceLiveSyncPathApplyResult,
)> {
    let mut seen = BTreeSet::new();
    let mut latest = Vec::new();
    for target_result in target_results.iter().rev() {
        for path_result in target_result.path_results.iter().rev() {
            let key = (
                target_result.link_id.as_str(),
                target_result.target_repo_root.as_str(),
                path_result.path.as_str(),
            );
            if seen.insert(key) {
                latest.push((target_result, path_result));
            }
        }
    }
    latest.reverse();
    latest
}
