//! MP-08 / MP-10 / MP-11: only the owned nondumpable worker performs Git I/O.
use super::{Query, Value};
use crate::error::DaemonError;
use crate::local::WaitingRoomLaunchTarget;
use crate::runtime::workspace_git_common::{
    detect_git_branch, workspace_display_label, worktree_display_label,
};

pub(crate) fn measure(query: Query) -> Result<Value, DaemonError> {
    Ok(match query {
        Query::Workspace(workspace) => Value::Label(workspace_display_label(&workspace)),
        Query::Worktree {
            worktree,
            workspace,
        } => {
            let branch = detect_git_branch(&worktree).ok();
            Value::Label(worktree_display_label(
                &worktree,
                &workspace,
                branch.as_deref(),
            ))
        }
        Query::Launch(cwd) => Value::Launch(launch(cwd)),
    })
}
fn launch(cwd: String) -> Option<WaitingRoomLaunchTarget> {
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(&cwd)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let worktree = git(&["rev-parse", "--show-toplevel"]).unwrap_or_else(|| cwd.clone());
    let workspace = git(&["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .map(|common| {
            common
                .strip_suffix("/.git")
                .map(str::to_owned)
                .unwrap_or_else(|| worktree.clone())
        })
        .unwrap_or_else(|| cwd.clone());
    let branch = detect_git_branch(&worktree).ok();
    Some(WaitingRoomLaunchTarget {
        repository_state: Some(
            crate::runtime::workspace_git_common::workspace_repository_state(&workspace),
        ),
        workspace_label: workspace_display_label(&workspace),
        directory: Some(workspace.clone()),
        worktree_label: worktree_display_label(&worktree, &workspace, branch.as_deref()),
        workspace_id: workspace,
        worktree_id: worktree,
    })
}
