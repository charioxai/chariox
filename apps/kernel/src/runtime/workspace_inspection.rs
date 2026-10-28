//! MP-08 / MP-10 / MP-11: background Git metadata cannot occupy runtime threads.
use crate::error::DaemonError;
use crate::local::{
    GetWorkspaceGitOverviewRequest, ListWorkspaceFilesRequest, LocalDaemonResponse,
    WorkspaceGitOverview, WorkspaceRepoFileListing, WorkspaceWorktreeRecord,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, Instant};

// Metadata only. File contents and mutating Git commands never enter this worker.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) enum Inspection {
    Labels(super::waiting_room_git_metadata::Query),
    Worktrees {
        workspace: String,
        current: Option<String>,
    },
    GitOverview(GetWorkspaceGitOverviewRequest),
    Files(ListWorkspaceFilesRequest),
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) enum Metadata {
    Labels(super::waiting_room_git_metadata::Value),
    Worktrees {
        workspace: String,
        worktrees: Vec<WorkspaceWorktreeRecord>,
    },
    GitOverview(WorkspaceGitOverview),
    Files(WorkspaceRepoFileListing),
}

pub(crate) async fn inspect(
    root: PathBuf,
    inspection: Inspection,
) -> Result<LocalDaemonResponse, DaemonError> {
    inspect_until(root, inspection, Instant::now() + Duration::from_secs(10)).await
}
async fn inspect_until(
    root: PathBuf,
    inspection: Inspection,
    deadline: Instant,
) -> Result<LocalDaemonResponse, DaemonError> {
    let metadata = tokio::task::spawn_blocking(move || {
        crate::project_environment::inspect_workspace_metadata(&root, inspection, deadline)
    })
    .await
    .map_err(|_| unavailable())??;
    Ok(match metadata {
        Metadata::Worktrees {
            workspace,
            worktrees,
        } => LocalDaemonResponse::WorkspaceWorktreesListed {
            workspace_id: workspace,
            worktrees,
        },
        Metadata::GitOverview(overview) => LocalDaemonResponse::WorkspaceGitOverview { overview },
        Metadata::Files(listing) => LocalDaemonResponse::WorkspaceFilesListed { listing },
        Metadata::Labels(_) => return Err(unavailable()),
    })
}
pub(crate) fn unavailable() -> DaemonError {
    DaemonError::LocalTransport {
        operation: "inspect workspace metadata",
        message: "Workspace inspection unavailable; retry when storage and Git are available"
            .into(),
    }
}
pub(crate) fn measure(inspection: Inspection) -> Result<Metadata, DaemonError> {
    #[cfg(test)]
    if let Some(marker) = std::env::var_os("CHARIOX_WORKSPACE_METADATA_STALL_MARKER") {
        std::fs::write(marker, std::process::id().to_string()).unwrap();
        std::thread::sleep(Duration::from_secs(12));
    }
    match inspection {
        Inspection::Labels(query) => {
            super::waiting_room_git_metadata::measure(query).map(Metadata::Labels)
        }
        Inspection::Worktrees { workspace, current } => {
            let worktrees = super::workspace_worktrees::list_workspace_worktrees(
                &workspace,
                current.as_deref(),
            )?;
            Ok(Metadata::Worktrees {
                workspace,
                worktrees,
            })
        }
        Inspection::GitOverview(request) => {
            super::workspace_git_overview::inspect_workspace_git_overview(
                &request.workspace_id,
                &request.worktree_id,
                request.compare_ref.as_deref(),
            )
            .map(Metadata::GitOverview)
        }
        Inspection::Files(request) => super::workspace_repo_files::list_workspace_repo_files(
            &request.workspace_id,
            &request.worktree_id,
            request.path_prefix.as_deref(),
            request.compare_ref.as_deref(),
            request.limit,
        )
        .map(Metadata::Files),
    }
}

#[cfg(test)]
#[path = "workspace_inspection_tests.rs"]
mod tests;
