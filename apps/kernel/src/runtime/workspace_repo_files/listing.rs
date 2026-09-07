//! Workspace repository file listing projection.

use std::collections::{BTreeMap, HashMap};

use crate::error::DaemonError;
use crate::local::{WorkspaceRepoFileEntry, WorkspaceRepoFileListing};
use crate::runtime::workspace_git_changes::workspace_git_file_changes;
use crate::runtime::workspace_git_common::{detect_git_branch, workspace_default_compare_ref};

use super::shared::{contained_directory, current_unix_ms, file_error, workspace_file_root};

pub(crate) fn list_workspace_repo_files(
    workspace_id: &str,
    worktree_id: &str,
    path_prefix: Option<&str>,
    requested_compare_ref: Option<&str>,
    limit: Option<u32>,
) -> Result<WorkspaceRepoFileListing, DaemonError> {
    let worktree_path = worktree_id.trim();
    if worktree_path.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "list workspace files",
            message: "worktree_id is required".to_string(),
        });
    }
    let (repo_root, has_git) = workspace_file_root(worktree_path)?;
    if !has_git {
        return list_directory_files(workspace_id, worktree_id, &repo_root, path_prefix, limit);
    }
    let repo_root_string = repo_root.display().to_string();
    let prefix = normalize_repo_file_prefix(path_prefix.unwrap_or_default());
    let branch = detect_git_branch(worktree_path).ok();
    let compare_ref = requested_compare_ref
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| workspace_default_compare_ref(&repo_root_string, branch.as_deref()));
    let change_map = workspace_git_file_changes(worktree_path, &compare_ref)?
        .into_iter()
        .map(|file| (file.path.clone(), file))
        .collect::<HashMap<_, _>>();
    let mut paths = workspace_git_tracked_files(worktree_path)?;
    paths.extend(change_map.keys().cloned());
    paths.sort();
    paths.dedup();

    let mut entries = BTreeMap::<String, WorkspaceRepoFileEntry>::new();
    for path in paths {
        let normalized_path = normalize_repo_file_prefix(&path);
        if normalized_path.is_empty() {
            continue;
        }
        let Some(remainder) = repo_file_remainder_for_prefix(&normalized_path, &prefix) else {
            continue;
        };
        if remainder.is_empty() {
            continue;
        }
        let mut parts = remainder.split('/');
        let Some(name) = parts.next().filter(|value| !value.is_empty()) else {
            continue;
        };
        let is_directory = parts.next().is_some();
        let entry_path = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}/{name}")
        };
        let change = change_map.get(&normalized_path);
        let entry = entries
            .entry(entry_path.clone())
            .or_insert_with(|| WorkspaceRepoFileEntry {
                path: entry_path.clone(),
                name: name.to_string(),
                kind: if is_directory { "directory" } else { "file" }.to_string(),
                changed: false,
                status: None,
                additions: 0,
                deletions: 0,
            });
        if is_directory {
            entry.kind = "directory".to_string();
        }
        if let Some(change) = change {
            entry.changed = true;
            entry.additions = entry.additions.saturating_add(change.additions);
            entry.deletions = entry.deletions.saturating_add(change.deletions);
            if !is_directory {
                entry.status = Some(change.status.clone());
            } else if entry.status.is_none() {
                entry.status = Some("changed".to_string());
            }
        }
    }
    let mut entries = entries.into_values().collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        let left_rank = if left.kind == "directory" { 0 } else { 1 };
        let right_rank = if right.kind == "directory" { 0 } else { 1 };
        left_rank
            .cmp(&right_rank)
            .then_with(|| left.name.cmp(&right.name))
    });
    let limit = limit.unwrap_or(400).clamp(1, 1000) as usize;
    let total_entries = entries.len();
    let truncated = total_entries > limit;
    entries.truncate(limit);
    Ok(WorkspaceRepoFileListing {
        workspace_id: workspace_id.to_string(),
        worktree_id: worktree_id.to_string(),
        path_prefix: prefix,
        compare_ref,
        total_entries: total_entries.min(u32::MAX as usize) as u32,
        truncated,
        entries,
        generated_at_ms: current_unix_ms(),
    })
}

fn list_directory_files(
    workspace_id: &str,
    worktree_id: &str,
    root: &std::path::Path,
    path_prefix: Option<&str>,
    limit: Option<u32>,
) -> Result<WorkspaceRepoFileListing, DaemonError> {
    // Validate before normalization so an absolute path is not silently made relative.
    let raw_prefix = path_prefix.unwrap_or_default().trim();
    let directory = contained_directory(root, raw_prefix)?;
    let prefix = normalize_repo_file_prefix(raw_prefix);
    let limit = limit.unwrap_or(400).clamp(1, 1000) as usize;
    let mut total_entries = 0_u32;
    let mut entries = BTreeMap::new();
    for item in std::fs::read_dir(directory).map_err(file_error)? {
        let item = item.map_err(file_error)?;
        let name = item.file_name();
        let Some(name) = name.to_str() else { continue };
        if name == ".git" {
            continue;
        }
        let resolved = match std::fs::canonicalize(item.path()) {
            Ok(path) if path.starts_with(root) => path,
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(file_error(error)),
        };
        let metadata = std::fs::metadata(resolved).map_err(file_error)?;
        if !metadata.is_dir() && !metadata.is_file() {
            continue;
        }
        let entry = WorkspaceRepoFileEntry {
            path: if prefix.is_empty() {
                name.to_string()
            } else {
                format!("{prefix}/{name}")
            },
            name: name.to_string(),
            kind: if metadata.is_dir() {
                "directory"
            } else {
                "file"
            }
            .to_string(),
            changed: false,
            status: None,
            additions: 0,
            deletions: 0,
        };
        total_entries = total_entries.saturating_add(1);
        entries.insert((!metadata.is_dir(), name.to_string()), entry);
        // Keep memory bounded while retaining deterministic directory-first ordering.
        if entries.len() > limit {
            entries.pop_last();
        }
    }
    Ok(WorkspaceRepoFileListing {
        workspace_id: workspace_id.to_string(),
        worktree_id: worktree_id.to_string(),
        path_prefix: prefix,
        compare_ref: String::new(),
        total_entries,
        truncated: total_entries as usize > entries.len(),
        entries: entries.into_values().collect(),
        generated_at_ms: current_unix_ms(),
    })
}

fn workspace_git_tracked_files(worktree_path: &str) -> Result<Vec<String>, DaemonError> {
    let output = std::process::Command::new("git")
        .args(["ls-files", "-z", "-c", "-o", "--exclude-standard"])
        .current_dir(worktree_path)
        .output()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "workspace repo files",
            message: error.to_string(),
        })?;
    if !output.status.success() {
        return Ok(Vec::new());
    }
    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .filter_map(|part| {
            let value = String::from_utf8_lossy(part).trim().to_string();
            (!value.is_empty()).then_some(value)
        })
        .collect())
}

fn normalize_repo_file_prefix(path: &str) -> String {
    path.trim()
        .trim_matches('/')
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect::<Vec<_>>()
        .join("/")
}

fn repo_file_remainder_for_prefix<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    if prefix.is_empty() {
        return Some(path);
    }
    if path == prefix {
        return Some("");
    }
    path.strip_prefix(prefix)
        .and_then(|value| value.strip_prefix('/'))
}

#[cfg(test)]
mod tests {
    use super::{normalize_repo_file_prefix, repo_file_remainder_for_prefix};

    #[test]
    fn repo_file_prefixes_normalize_for_listing_projection() {
        assert_eq!(normalize_repo_file_prefix(" /src//./app/ "), "src/app");
        assert_eq!(normalize_repo_file_prefix("./"), "");
        assert_eq!(
            repo_file_remainder_for_prefix("src/app/main.rs", "src"),
            Some("app/main.rs")
        );
        assert_eq!(repo_file_remainder_for_prefix("src", "src"), Some(""));
        assert_eq!(
            repo_file_remainder_for_prefix("src-other/main.rs", "src"),
            None
        );
    }
}
