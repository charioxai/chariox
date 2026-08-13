use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::error::DaemonError;
use crate::local::WorkspaceWorktreeRecord;
use crate::runtime::workspace_git_common::{
    detect_git_branch, git_ref_exists, resolve_repo_root, run_git, same_fs_path,
    worktree_display_label,
};
use crate::runtime::workspace_search::expand_workspace_query_path;
use crate::session::{RuntimeSession, SessionStatus};

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

pub(crate) fn create_waiting_room_worktree(
    workspace_path: &str,
    requested_path: Option<&str>,
    requested_branch: Option<&str>,
    requested_base_ref: Option<&str>,
    current_worktree: Option<&str>,
    label_workspace_path: Option<&str>,
) -> Result<WorkspaceWorktreeRecord, DaemonError> {
    let repo_root = resolve_repo_root(workspace_path)?;
    let base_ref = requested_base_ref
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or(resolve_preferred_base_ref(&repo_root)?);
    let description = std::env::var("CHARIOX_WAITING_ROOM_WORKTREE_DESCRIPTION")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            format!(
                "{}-session",
                repo_root
                    .file_name()
                    .and_then(OsStr::to_str)
                    .unwrap_or("workspace")
            )
        });
    let branch_base = format!(
        "chariox/{}-{}",
        slugify_segment(&description),
        timestamp_slug(),
    );
    let branch = match requested_branch
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(value) => value.to_string(),
        None => resolve_available_branch_name(&repo_root, &branch_base)?,
    };
    let parent = repo_root.parent().unwrap_or(&repo_root);
    let directory_base = default_worktree_directory_base(
        repo_root
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("workspace"),
        &branch,
    );
    let directory = requested_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| resolve_requested_worktree_directory(parent, value))
        .unwrap_or_else(|| resolve_available_worktree_directory(parent, &directory_base));
    run_git(
        &repo_root,
        &[
            "worktree",
            "add",
            "-b",
            &branch,
            directory.to_str().unwrap_or(""),
            &base_ref,
        ],
    )?;
    let path = directory.display().to_string();
    let branch = detect_git_branch(&path).ok();
    Ok(WorkspaceWorktreeRecord {
        current: current_worktree
            .map(|current| current == path)
            .unwrap_or(false),
        label: worktree_display_label(
            &path,
            label_workspace_path.unwrap_or(workspace_path),
            branch.as_deref(),
        ),
        branch,
        path,
    })
}

pub(crate) fn delete_workspace_worktree(
    workspace_id: &str,
    worktree_id: &str,
    force: bool,
    sessions: &[RuntimeSession],
) -> Result<String, DaemonError> {
    let (repo_root, worktree_path) = resolve_deletable_git_worktree(workspace_id, worktree_id)?;
    let blockers = active_worktree_session_blockers(workspace_id, &worktree_path, sessions);
    if !blockers.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "workspace worktree delete",
            message: format!(
                "worktree is still used by active runtime sessions: {}",
                blockers.join(", ")
            ),
        });
    }
    let mut args = vec!["worktree", "remove"];
    if force {
        args.push("--force");
    }
    args.push(worktree_path.as_str());
    run_git(&repo_root, &args)?;
    Ok(worktree_path)
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

fn resolve_deletable_git_worktree(
    workspace_id: &str,
    worktree_id: &str,
) -> Result<(PathBuf, String), DaemonError> {
    let target = worktree_id.trim();
    if target.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "workspace worktree delete",
            message: "worktree_id is required".to_string(),
        });
    }
    let repo_root = resolve_repo_root(workspace_id)?;
    let output = std::process::Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(&repo_root)
        .output()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "workspace worktree delete",
            message: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(DaemonError::LocalTransport {
            operation: "workspace worktree delete",
            message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    let worktree_path = parse_git_worktree_list(String::from_utf8_lossy(&output.stdout).as_ref())
        .into_iter()
        .map(|(path, _branch)| path)
        .find(|path| same_fs_path(path, target))
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "workspace worktree delete",
            message: format!("worktree is not registered: {target}"),
        })?;
    if same_fs_path(&worktree_path, repo_root.to_string_lossy().as_ref()) {
        return Err(DaemonError::LocalTransport {
            operation: "workspace worktree delete",
            message: "refusing to delete the main workspace worktree".to_string(),
        });
    }
    Ok((repo_root, worktree_path))
}

fn active_worktree_session_blockers(
    workspace_id: &str,
    worktree_id: &str,
    sessions: &[RuntimeSession],
) -> Vec<String> {
    let mut blockers = Vec::new();
    for session in sessions {
        if session.status() == SessionStatus::Ended {
            continue;
        }
        let session_owns_worktree = worktree_ids_match(session.worktree_id(), worktree_id)
            || session.agents().iter().any(|agent| {
                let agent_worktree = agent.worktree_id().unwrap_or(session.worktree_id());
                worktree_ids_match(agent_worktree, worktree_id)
            });
        if session_owns_worktree
            || (workspace_id == session.workspace_id()
                && worktree_ids_match(session.worktree_id(), worktree_id))
        {
            blockers.push(session.id().to_string());
        }
    }
    blockers.sort();
    blockers.dedup();
    blockers
}

fn worktree_ids_match(left: &str, right: &str) -> bool {
    left == right || same_fs_path(left, right)
}

fn resolve_preferred_base_ref(repo_root: &Path) -> Result<String, DaemonError> {
    for candidate in ["main", "master"] {
        if git_ref_exists(repo_root, &format!("refs/heads/{candidate}"))? {
            return Ok(candidate.to_string());
        }
    }
    let branch = detect_git_branch(repo_root.to_string_lossy().as_ref())?;
    Ok(if branch == "HEAD" || branch.is_empty() {
        "HEAD".to_string()
    } else {
        branch
    })
}

fn resolve_available_branch_name(repo_root: &Path, base_name: &str) -> Result<String, DaemonError> {
    let mut attempt = base_name.to_string();
    let mut index = 1;
    while git_ref_exists(repo_root, &format!("refs/heads/{attempt}"))? {
        attempt = format!("{base_name}-{index}");
        index += 1;
    }
    Ok(attempt)
}

fn resolve_available_worktree_directory(parent: &Path, base_name: &str) -> PathBuf {
    let mut attempt = parent.join(base_name);
    let mut index = 1;
    while attempt.exists() {
        attempt = parent.join(format!("{base_name}-{index}"));
        index += 1;
    }
    attempt
}

fn default_worktree_directory_base(repo_name: &str, branch: &str) -> String {
    let repo_slug = non_empty_slug(repo_name, "workspace");
    let branch_leaf = branch
        .rsplit('/')
        .find(|segment| !segment.trim().is_empty())
        .unwrap_or(branch);
    let branch_slug = non_empty_slug(branch_leaf, "worktree");
    let repo_prefix = format!("{repo_slug}-");
    if branch_slug == repo_slug || branch_slug.starts_with(&repo_prefix) {
        branch_slug
    } else {
        format!("{repo_slug}-{branch_slug}")
    }
}

fn non_empty_slug(value: &str, fallback: &str) -> String {
    let slug = slugify_segment(value);
    if slug.is_empty() {
        fallback.to_string()
    } else {
        slug
    }
}

fn resolve_requested_worktree_directory(parent: &Path, value: &str) -> PathBuf {
    let expanded = expand_workspace_query_path(value);
    if expanded.is_absolute() {
        expanded
    } else {
        parent.join(expanded)
    }
}

fn slugify_segment(value: &str) -> String {
    let slug = value
        .trim()
        .to_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    slug.trim_matches('-')
        .split('-')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn timestamp_slug() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    now.to_string()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        default_worktree_directory_base, parse_git_worktree_list,
        resolve_available_worktree_directory, resolve_requested_worktree_directory,
        slugify_segment,
    };

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

    #[test]
    fn slugify_segment_normalizes_worktree_names() {
        assert_eq!(slugify_segment(" Feature/Add Thing "), "feature-add-thing");
        assert_eq!(slugify_segment("///"), "");
        assert_eq!(slugify_segment("A__B"), "a-b");
    }

    #[test]
    fn default_worktree_directory_base_uses_branch_leaf_without_duplicate_repo_prefix() {
        assert_eq!(
            default_worktree_directory_base(
                "chariox-cloud",
                "chariox/chariox-cloud-session-1783622367"
            ),
            "chariox-cloud-session-1783622367"
        );
        assert_eq!(
            default_worktree_directory_base("chariox", "chariox/chariox-session-1779647319"),
            "chariox-session-1779647319"
        );
        assert_eq!(
            default_worktree_directory_base("chariox-cloud", "feature/worktree-name"),
            "chariox-cloud-worktree-name"
        );
    }

    #[test]
    fn requested_worktree_directory_expands_relative_paths_from_parent() {
        let parent = PathBuf::from("/repo-parent");

        assert_eq!(
            resolve_requested_worktree_directory(&parent, "feature")
                .display()
                .to_string(),
            "/repo-parent/feature"
        );
        assert_eq!(
            resolve_requested_worktree_directory(&parent, "/tmp/feature")
                .display()
                .to_string(),
            "/tmp/feature"
        );
    }

    #[test]
    fn available_worktree_directory_advances_existing_paths() {
        let parent =
            std::env::temp_dir().join(format!("chariox-worktree-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&parent);
        std::fs::create_dir_all(parent.join("repo-feature")).unwrap();
        std::fs::create_dir_all(parent.join("repo-feature-1")).unwrap();

        let available = resolve_available_worktree_directory(&parent, "repo-feature");

        assert_eq!(available, parent.join("repo-feature-2"));
        let _ = std::fs::remove_dir_all(&parent);
    }
}
