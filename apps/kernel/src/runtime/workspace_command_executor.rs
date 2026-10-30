use crate::error::DaemonError;
use crate::local::{
    CommitAndPushWorkspaceChangesRequest, CommitWorkspaceChangesRequest,
    CreateWorkspaceDirectoryRequest, CreateWorkspacePullRequestRequest,
    CreateWorkspaceWorktreeRequest, DeleteWorkspaceWorktreeRequest, GetWorkspaceFileContentRequest,
    LocalDaemonRequest, LocalDaemonResponse, PushWorkspaceBranchRequest,
    SearchWorkspaceDirectoriesRequest,
};
use crate::runtime::projection::SessionStateProjectionStore;
use crate::runtime::state::KernelRuntimeState;
use crate::runtime::waiting_room_public_projection::infer_waiting_room_launch_target;
use crate::runtime::workspace_git_actions::{
    commit_and_push_workspace_changes, commit_workspace_changes, create_workspace_pull_request,
    push_workspace_branch,
};
use crate::runtime::workspace_repo_files::get_workspace_file_content;
use crate::runtime::workspace_search::{create_workspace_directory, search_workspace_directories};
use crate::runtime::workspace_worktrees::{
    create_waiting_room_worktree, delete_workspace_worktree,
};

pub(crate) async fn execute_workspace_command_request(
    runtime_state: &KernelRuntimeState,
    session_projection: &SessionStateProjectionStore,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::SearchWorkspaceDirectories(request) => {
            execute_search_workspace_directories_request(request)
        }
        LocalDaemonRequest::CreateWorkspaceDirectory(request) => {
            execute_create_workspace_directory_request(request)
        }
        LocalDaemonRequest::ListWorkspaceWorktrees(request) => {
            let current = infer_waiting_room_launch_target(&[]).map(|target| target.worktree_id);
            crate::runtime::workspace_inspection::inspect(
                runtime_state
                    .config_snapshot()
                    .await
                    .private_runtime_state_root(),
                crate::runtime::workspace_inspection::Inspection::Worktrees {
                    workspace: request.workspace_id,
                    current,
                },
            )
            .await
        }
        LocalDaemonRequest::CreateWorkspaceWorktree(request) => {
            execute_create_workspace_worktree_request(request)
        }
        LocalDaemonRequest::DeleteWorkspaceWorktree(request) => {
            execute_delete_workspace_worktree_request(request, session_projection, runtime_state)
                .await
        }
        LocalDaemonRequest::CreateWorkspacePullRequest(request) => {
            execute_create_workspace_pull_request_request(request)
        }
        LocalDaemonRequest::GetWorkspaceGitOverview(request) => {
            crate::runtime::workspace_inspection::inspect(
                runtime_state
                    .config_snapshot()
                    .await
                    .private_runtime_state_root(),
                crate::runtime::workspace_inspection::Inspection::GitOverview(request),
            )
            .await
        }
        LocalDaemonRequest::ListWorkspaceFiles(request) => {
            crate::runtime::workspace_inspection::inspect(
                runtime_state
                    .config_snapshot()
                    .await
                    .private_runtime_state_root(),
                crate::runtime::workspace_inspection::Inspection::Files(request),
            )
            .await
        }
        LocalDaemonRequest::GetWorkspaceFileContent(request) => {
            execute_get_workspace_file_content_request(request)
        }
        LocalDaemonRequest::CommitWorkspaceChanges(request) => {
            execute_commit_workspace_changes_request(request)
        }
        LocalDaemonRequest::PushWorkspaceBranch(request) => {
            execute_push_workspace_branch_request(request)
        }
        LocalDaemonRequest::CommitAndPushWorkspaceChanges(request) => {
            execute_commit_and_push_workspace_changes_request(request)
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "workspace request",
            message: "unsupported workspace request".to_string(),
        }),
    }
}

pub(crate) fn execute_search_workspace_directories_request(
    request: SearchWorkspaceDirectoriesRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let limit = request.limit.unwrap_or(12).clamp(1, 50);
    let directories =
        search_workspace_directories(&request.query, limit, infer_waiting_room_launch_target(&[]))?;
    let workspace_git = directories
        .iter()
        .map(|path| crate::local::WorkspaceDirectoryGitSummary {
            workspace_id: path.clone(),
            repository_state: crate::runtime::workspace_git_common::workspace_repository_state(
                path,
            ),
            workspace_label: crate::runtime::workspace_git_common::workspace_display_label(path),
        })
        .collect();
    Ok(LocalDaemonResponse::WorkspaceDirectoriesSearched {
        directories,
        workspace_git,
    })
}

pub(crate) fn execute_create_workspace_directory_request(
    request: CreateWorkspaceDirectoryRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let directory = create_workspace_directory(&request.path)?;
    Ok(LocalDaemonResponse::WorkspaceDirectoryCreated { directory })
}

pub(crate) fn execute_create_workspace_worktree_request(
    request: CreateWorkspaceWorktreeRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let launch_target = infer_waiting_room_launch_target(&[]);
    let worktree = create_waiting_room_worktree(
        &request.workspace_id,
        request.path.as_deref(),
        request.branch.as_deref(),
        request.base_ref.as_deref(),
        launch_target
            .as_ref()
            .map(|target| target.worktree_id.as_str()),
        launch_target
            .as_ref()
            .map(|target| target.workspace_id.as_str()),
    )?;
    Ok(LocalDaemonResponse::WorkspaceWorktreeCreated {
        workspace_id: request.workspace_id,
        worktree,
    })
}

pub(crate) async fn execute_delete_workspace_worktree_request(
    request: DeleteWorkspaceWorktreeRequest,
    session_projection: &SessionStateProjectionStore,
    runtime_state: &KernelRuntimeState,
) -> Result<LocalDaemonResponse, DaemonError> {
    let sessions = if let Some(sessions) = session_projection.list() {
        sessions
    } else {
        runtime_state.list_session_snapshots()
    };
    let path = delete_workspace_worktree(
        &request.workspace_id,
        &request.worktree_id,
        request.force,
        &sessions,
    )?;
    Ok(LocalDaemonResponse::WorkspaceWorktreeDeleted {
        workspace_id: request.workspace_id,
        worktree_id: request.worktree_id,
        path,
    })
}

pub(crate) fn execute_create_workspace_pull_request_request(
    request: CreateWorkspacePullRequestRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let pull_request = create_workspace_pull_request(
        &request.workspace_id,
        &request.worktree_id,
        request.title.as_deref(),
        request.body.as_deref(),
        request.base_ref.as_deref(),
        request.draft,
    )?;
    Ok(LocalDaemonResponse::WorkspacePullRequestCreated { pull_request })
}

pub(crate) fn execute_get_workspace_file_content_request(
    request: GetWorkspaceFileContentRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    get_workspace_file_content(
        &request.workspace_id,
        &request.worktree_id,
        &request.path,
        request.compare_ref.as_deref(),
        request.known_fingerprint.as_deref(),
        request.max_bytes,
    )
}

pub(crate) fn execute_commit_workspace_changes_request(
    request: CommitWorkspaceChangesRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let result = commit_workspace_changes(
        &request.workspace_id,
        &request.worktree_id,
        &request.message,
    )?;
    Ok(LocalDaemonResponse::WorkspaceGitActionCompleted { result })
}

pub(crate) fn execute_push_workspace_branch_request(
    request: PushWorkspaceBranchRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let result = push_workspace_branch(
        &request.workspace_id,
        &request.worktree_id,
        request.force_with_lease,
    )?;
    Ok(LocalDaemonResponse::WorkspaceGitActionCompleted { result })
}

pub(crate) fn execute_commit_and_push_workspace_changes_request(
    request: CommitAndPushWorkspaceChangesRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let result = commit_and_push_workspace_changes(
        &request.workspace_id,
        &request.worktree_id,
        &request.message,
    )?;
    Ok(LocalDaemonResponse::WorkspaceGitActionCompleted { result })
}
