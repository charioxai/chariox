use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::error::DaemonError;
use crate::local::WorkspaceGitFileChange;

pub(crate) fn workspace_git_file_changes(
    worktree_path: &str,
    compare_ref: &str,
) -> Result<Vec<WorkspaceGitFileChange>, DaemonError> {
    let status_by_path = workspace_git_status_by_path(worktree_path)?;
    let mut files = workspace_git_numstat(worktree_path, compare_ref).unwrap_or_default();
    for file in &mut files {
        if let Some(status) = status_by_path.get(&file.path) {
            file.status = status.clone();
        }
    }
    let mut known = files
        .iter()
        .map(|file| file.path.clone())
        .collect::<HashSet<_>>();
    for (path, status) in status_by_path {
        if !known.insert(path.clone()) {
            continue;
        }
        let additions = if status == "untracked" {
            count_file_lines(Path::new(worktree_path).join(&path))
        } else {
            0
        };
        files.push(WorkspaceGitFileChange {
            path,
            status,
            additions,
            deletions: 0,
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

pub(crate) fn workspace_git_diff_text(
    worktree_path: &str,
    compare_ref: &str,
    max_bytes: usize,
) -> Result<String, DaemonError> {
    let output = std::process::Command::new("git")
        .args([
            "diff",
            "--no-ext-diff",
            "--no-color",
            "--find-renames",
            compare_ref,
            "--",
        ])
        .current_dir(worktree_path)
        .output()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "workspace git diff",
            message: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(DaemonError::LocalTransport {
            operation: "workspace git diff",
            message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    let mut diff = String::from_utf8_lossy(&output.stdout).to_string();
    if diff.len() > max_bytes {
        diff.truncate(max_bytes);
        diff.push_str("\n\n[diff truncated]");
    }
    Ok(diff)
}

pub(crate) fn workspace_git_status_by_path(
    worktree_path: &str,
) -> Result<HashMap<String, String>, DaemonError> {
    let output = std::process::Command::new("git")
        .args(["status", "--porcelain=v1", "--untracked-files=all"])
        .current_dir(worktree_path)
        .output()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "workspace git status",
            message: error.to_string(),
        })?;
    if !output.status.success() {
        return Ok(HashMap::new());
    }
    let mut statuses = HashMap::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if line.len() < 4 {
            continue;
        }
        let code = &line[..2];
        let path = line[3..].trim();
        if path.is_empty() {
            continue;
        }
        let normalized_path = normalize_git_status_path(path);
        statuses.insert(normalized_path, git_status_label(code).to_string());
    }
    Ok(statuses)
}

fn workspace_git_numstat(
    worktree_path: &str,
    compare_ref: &str,
) -> Result<Vec<WorkspaceGitFileChange>, DaemonError> {
    let output = std::process::Command::new("git")
        .args(["diff", "--numstat", "--find-renames", compare_ref, "--"])
        .current_dir(worktree_path)
        .output()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "workspace git diff",
            message: error.to_string(),
        })?;
    if !output.status.success() {
        return Ok(Vec::new());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse_git_numstat_line)
        .collect())
}

fn parse_git_numstat_line(line: &str) -> Option<WorkspaceGitFileChange> {
    let mut parts = line.split('\t');
    let additions = parse_git_numstat_count(parts.next()?)?;
    let deletions = parse_git_numstat_count(parts.next()?)?;
    let path = parts.next()?.trim();
    if path.is_empty() {
        return None;
    }
    Some(WorkspaceGitFileChange {
        path: normalize_git_change_path(path),
        status: "modified".to_string(),
        additions,
        deletions,
    })
}

fn parse_git_numstat_count(value: &str) -> Option<u32> {
    if value == "-" {
        return Some(0);
    }
    value.parse().ok()
}

fn normalize_git_change_path(path: &str) -> String {
    if let Some((_, right)) = path.rsplit_once(" => ") {
        return right.trim_matches(|ch| ch == '{' || ch == '}').to_string();
    }
    path.to_string()
}

fn normalize_git_status_path(path: &str) -> String {
    if let Some((_, right)) = path.rsplit_once(" -> ") {
        return right.to_string();
    }
    path.to_string()
}

fn git_status_label(code: &str) -> &'static str {
    if code == "??" {
        return "untracked";
    }
    if code.contains('D') {
        return "deleted";
    }
    if code.contains('A') {
        return "added";
    }
    if code.contains('R') {
        return "renamed";
    }
    if code.contains('U') {
        return "conflicted";
    }
    "modified"
}

fn count_file_lines(path: PathBuf) -> u32 {
    std::fs::read_to_string(path)
        .map(|contents| contents.lines().count().min(u32::MAX as usize) as u32)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{
        git_status_label, normalize_git_change_path, normalize_git_status_path,
        parse_git_numstat_line,
    };

    #[test]
    fn parse_git_numstat_line_projects_file_changes() {
        let change = parse_git_numstat_line("12\t3\tsrc/lib.rs").expect("numstat line parses");
        assert_eq!(change.path, "src/lib.rs");
        assert_eq!(change.status, "modified");
        assert_eq!(change.additions, 12);
        assert_eq!(change.deletions, 3);

        let binary = parse_git_numstat_line("-\t-\tasset.bin").expect("binary numstat parses");
        assert_eq!(binary.additions, 0);
        assert_eq!(binary.deletions, 0);
    }

    #[test]
    fn normalize_git_change_path_uses_rename_destination() {
        assert_eq!(normalize_git_change_path("old.rs => new.rs"), "new.rs");
        assert_eq!(normalize_git_change_path("src/lib.rs"), "src/lib.rs");
    }

    #[test]
    fn git_status_labels_preserve_existing_policy() {
        assert_eq!(git_status_label("??"), "untracked");
        assert_eq!(git_status_label(" D"), "deleted");
        assert_eq!(git_status_label("A "), "added");
        assert_eq!(git_status_label("R "), "renamed");
        assert_eq!(git_status_label("UU"), "conflicted");
        assert_eq!(git_status_label(" M"), "modified");
    }

    #[test]
    fn normalize_git_status_path_uses_rename_destination() {
        assert_eq!(normalize_git_status_path("old.rs -> new.rs"), "new.rs");
        assert_eq!(normalize_git_status_path("src/lib.rs"), "src/lib.rs");
    }
}
