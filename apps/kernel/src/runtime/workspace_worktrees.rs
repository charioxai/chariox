use std::path::PathBuf;

use crate::error::DaemonError;
use crate::local::WorkspaceWorktreeRecord;
use crate::runtime::workspace_git_common::{
    detect_git_branch, same_fs_path, worktree_display_label,
};

pub(crate) fn list_workspace_worktrees(
    workspace_id: &str,
    current_worktree: Option<&str>,
) -> Result<Vec<WorkspaceWorktreeRecord>, DaemonError> {
    let workspace_path = PathBuf::from(workspace_id);
    let output = std::process::Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(&workspace_path)
        .output();
    let Ok(output) = output else {
        return Ok(vec![fallback_worktree_record(workspace_id, None)]);
    };
    if !output.status.success() {
        let branch = detect_git_branch(workspace_id).ok();
        return Ok(vec![fallback_worktree_record(workspace_id, branch)]);
    }
    let current_worktree_path = current_worktree.unwrap_or(workspace_id);
    let mut worktrees = parse_git_worktree_list(String::from_utf8_lossy(&output.stdout).as_ref())
        .into_iter()
        .map(|(path, branch)| WorkspaceWorktreeRecord {
            current: same_fs_path(&path, current_worktree_path),
            label: worktree_display_label(&path, workspace_id, branch.as_deref()),
            branch,
            path,
        })
        .collect::<Vec<_>>();
    if worktrees.is_empty() {
        let branch = detect_git_branch(workspace_id).ok();
        worktrees.push(fallback_worktree_record(workspace_id, branch));
    }
    Ok(worktrees)
}

fn fallback_worktree_record(workspace_id: &str, branch: Option<String>) -> WorkspaceWorktreeRecord {
    WorkspaceWorktreeRecord {
        path: workspace_id.to_string(),
        label: worktree_display_label(workspace_id, workspace_id, branch.as_deref()),
        branch,
        current: true,
    }
}

pub(crate) fn parse_git_worktree_list(stdout: &str) -> Vec<(String, Option<String>)> {
    let mut entries = Vec::new();
    let mut current_path: Option<String> = None;
    let mut current_branch: Option<String> = None;
    for raw_line in stdout.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            if let Some(path) = current_path.take() {
                entries.push((path, current_branch.take()));
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("worktree ") {
            if let Some(path) = current_path.replace(rest.trim().to_string()) {
                entries.push((path, current_branch.take()));
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("branch ") {
            current_branch = Some(rest.trim().trim_start_matches("refs/heads/").to_string());
        }
    }
    if let Some(path) = current_path.take() {
        entries.push((path, current_branch.take()));
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::parse_git_worktree_list;

    #[test]
    fn parse_git_worktree_list_reads_porcelain_entries() {
        let entries = parse_git_worktree_list(
            "worktree /repo/main\nHEAD abc\nbranch refs/heads/main\n\nworktree /repo/feature\nHEAD def\nbranch refs/heads/feature\n\n",
        );
        assert_eq!(
            entries,
            vec![
                ("/repo/main".to_string(), Some("main".to_string())),
                ("/repo/feature".to_string(), Some("feature".to_string())),
            ]
        );
    }

    #[test]
    fn parse_git_worktree_list_keeps_detached_entries() {
        let entries = parse_git_worktree_list("worktree /repo/detached\nHEAD abc\n\n");
        assert_eq!(entries, vec![("/repo/detached".to_string(), None)]);
    }

    #[test]
    fn parse_git_worktree_list_handles_missing_blank_separator() {
        let entries = parse_git_worktree_list(
            "worktree /repo/main\nbranch refs/heads/main\nworktree /repo/feature\nbranch refs/heads/feature\n",
        );
        assert_eq!(
            entries,
            vec![
                ("/repo/main".to_string(), Some("main".to_string())),
                ("/repo/feature".to_string(), Some("feature".to_string())),
            ]
        );
    }
}
