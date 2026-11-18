use crate::runtime::bounded_process::BoundedCommandExt as _;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::error::DaemonError;
use crate::local::WorkspaceGitFileChange;

/// What `git status` says about one changed path.
pub(crate) struct StatusEntry {
    pub(crate) status: String,
    /// The path a staged rename came from: a commit of the new path holds both.
    pub(crate) renamed_from: Option<String>,
}

pub(crate) fn workspace_git_file_changes(
    worktree_path: &str,
    compare_ref: &str,
) -> Result<Vec<WorkspaceGitFileChange>, DaemonError> {
    let status_by_path = workspace_git_status_by_path(worktree_path)?;
    let mut files = workspace_git_numstat(worktree_path, compare_ref).unwrap_or_default();
    for file in &mut files {
        if let Some(entry) = status_by_path.get(&file.path) {
            file.status = entry.status.clone();
        }
    }
    let mut known = files
        .iter()
        .map(|file| file.path.clone())
        .collect::<HashSet<_>>();
    for (path, StatusEntry { status, .. }) in status_by_path {
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
    let output = crate::process_spawn::command("git")
        .args([
            "diff",
            "--no-ext-diff",
            "--no-color",
            "--find-renames",
            compare_ref,
            "--",
        ])
        .current_dir(worktree_path)
        .bounded_output()
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
) -> Result<HashMap<String, StatusEntry>, DaemonError> {
    let output = crate::process_spawn::command("git")
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
        .current_dir(worktree_path)
        .bounded_output()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "workspace git status",
            message: error.to_string(),
        })?;
    if !output.status.success() {
        return Ok(HashMap::new());
    }
    Ok(parse_status_records(&output.stdout))
}

/// `-z` records are `XY path`, and a rename adds the source path as the next
/// record. Paths are the real names, never git's quoted display form.
fn parse_status_records(stdout: &[u8]) -> HashMap<String, StatusEntry> {
    let mut statuses = HashMap::new();
    let mut records = stdout
        .split(|byte| *byte == 0)
        .map(|record| String::from_utf8_lossy(record).into_owned());
    while let Some(record) = records.next() {
        let (Some(code), Some(path)) = (record.get(..2), record.get(3..)) else {
            continue;
        };
        let renamed = code.contains('R');
        let copied = code.contains('C');
        let source = (renamed || copied).then(|| records.next()).flatten();
        if path.is_empty() {
            continue;
        }
        statuses.insert(
            path.to_string(),
            StatusEntry {
                status: git_status_label(code).to_string(),
                renamed_from: source.filter(|_| renamed),
            },
        );
    }
    statuses
}

fn workspace_git_numstat(
    worktree_path: &str,
    compare_ref: &str,
) -> Result<Vec<WorkspaceGitFileChange>, DaemonError> {
    let output = crate::process_spawn::command("git")
        .args([
            "diff",
            "--numstat",
            "-z",
            "--find-renames",
            compare_ref,
            "--",
        ])
        .current_dir(worktree_path)
        .bounded_output()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "workspace git diff",
            message: error.to_string(),
        })?;
    if !output.status.success() {
        return Ok(Vec::new());
    }
    Ok(parse_numstat_records(&output.stdout))
}

/// `-z` records are `added<TAB>deleted<TAB>path`; a rename leaves the path
/// empty and follows with its source and destination as two records.
fn parse_numstat_records(stdout: &[u8]) -> Vec<WorkspaceGitFileChange> {
    let mut changes = Vec::new();
    let mut records = stdout
        .split(|byte| *byte == 0)
        .map(|record| String::from_utf8_lossy(record).into_owned());
    while let Some(record) = records.next() {
        let mut parts = record.splitn(3, '\t');
        let (Some(additions), Some(deletions), Some(path)) = (
            parts.next().and_then(parse_git_numstat_count),
            parts.next().and_then(parse_git_numstat_count),
            parts.next(),
        ) else {
            continue;
        };
        let path = if path.is_empty() {
            records.next();
            records.next().unwrap_or_default()
        } else {
            path.to_string()
        };
        if path.is_empty() {
            continue;
        }
        changes.push(WorkspaceGitFileChange {
            path,
            status: "modified".to_string(),
            additions,
            deletions,
        });
    }
    changes
}

fn parse_git_numstat_count(value: &str) -> Option<u32> {
    if value == "-" {
        return Some(0);
    }
    value.parse().ok()
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
    use super::{git_status_label, parse_numstat_records, parse_status_records};

    #[test]
    fn numstat_records_project_file_changes_with_real_paths() {
        let changes = parse_numstat_records(
            b"12\t3\tsrc/lib.rs\0-\t-\tasset.bin\x001\t0\thello world.txt\0".as_slice(),
        );
        let seen: Vec<_> = changes
            .iter()
            .map(|change| (change.path.as_str(), change.additions, change.deletions))
            .collect();
        assert_eq!(
            seen,
            [
                ("src/lib.rs", 12, 3),
                ("asset.bin", 0, 0),
                ("hello world.txt", 1, 0)
            ]
        );
    }

    #[test]
    fn a_numstat_rename_reports_its_destination() {
        let changes =
            parse_numstat_records(b"0\t0\t\0old name.rs\0new name.rs\x002\t1\tother.rs\0");
        let paths: Vec<_> = changes.iter().map(|change| change.path.as_str()).collect();
        assert_eq!(paths, ["new name.rs", "other.rs"]);
    }

    #[test]
    fn status_records_keep_real_paths_and_a_rename_source() {
        let statuses = parse_status_records(
            "R  new é.txt\0old é.txt\0 M hello world.txt\0?? plain.txt\0".as_bytes(),
        );
        let renamed = &statuses["new é.txt"];
        assert_eq!(renamed.status, "renamed");
        assert_eq!(renamed.renamed_from.as_deref(), Some("old é.txt"));
        assert_eq!(statuses["hello world.txt"].status, "modified");
        assert_eq!(statuses["plain.txt"].status, "untracked");
        assert!(statuses["plain.txt"].renamed_from.is_none());
        assert_eq!(statuses.len(), 3);
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
}
