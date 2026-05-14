use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::error::DaemonError;
use crate::local::WaitingRoomLaunchTarget;

pub(crate) fn search_workspace_directories(
    query: &str,
    limit: usize,
    launch_target: Option<WaitingRoomLaunchTarget>,
) -> Result<Vec<String>, DaemonError> {
    let mut results = Vec::new();
    let mut seen = HashSet::new();
    let roots = workspace_search_roots();
    let trimmed_query = query.trim();
    let normalized_query = trimmed_query.to_lowercase();

    if let Some(target) = launch_target {
        push_matching_path(
            &mut results,
            &mut seen,
            target.workspace_id,
            &normalized_query,
            limit,
        );
        push_matching_path(
            &mut results,
            &mut seen,
            target.worktree_id,
            &normalized_query,
            limit,
        );
    }

    if normalized_query.is_empty() {
        for root in &roots {
            push_unique_path(&mut results, &mut seen, root.display().to_string());
            if results.len() >= limit {
                break;
            }
            if let Ok(entries) = std::fs::read_dir(root) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        push_unique_path(&mut results, &mut seen, path.display().to_string());
                        if results.len() >= limit {
                            break;
                        }
                    }
                }
            }
            if results.len() >= limit {
                break;
            }
        }
        results.truncate(limit);
        return Ok(results);
    }

    if looks_like_path_query(trimmed_query) {
        append_directory_completion(&mut results, &mut seen, trimmed_query, limit)?;
        results.truncate(limit);
        return Ok(results);
    }

    for root in roots {
        append_matching_directory_children(
            &mut results,
            &mut seen,
            &root,
            &normalized_query,
            limit,
        )?;
    }
    results.truncate(limit);
    Ok(results)
}

pub(crate) fn create_workspace_directory(path: &str) -> Result<String, DaemonError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "create workspace directory",
            message: "workspace path is required".to_string(),
        });
    }
    let expanded = expand_workspace_query_path(trimmed);
    let directory = if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "create workspace directory",
                message: error.to_string(),
            })?
            .join(expanded)
    };
    if directory.exists() && !directory.is_dir() {
        return Err(DaemonError::LocalTransport {
            operation: "create workspace directory",
            message: format!("{} exists and is not a directory", directory.display()),
        });
    }
    std::fs::create_dir_all(&directory).map_err(|error| DaemonError::LocalTransport {
        operation: "create workspace directory",
        message: error.to_string(),
    })?;
    Ok(directory.display().to_string())
}

fn workspace_search_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut seen = HashSet::new();
    for candidate in [
        std::env::current_dir().ok(),
        std::env::var_os("HOME").map(PathBuf::from),
    ]
    .into_iter()
    .flatten()
    {
        let path = candidate;
        if seen.insert(path.clone()) {
            roots.push(path);
        }
    }
    roots
}

fn push_unique_path(results: &mut Vec<String>, seen: &mut HashSet<String>, value: String) {
    if value.trim().is_empty() {
        return;
    }
    if seen.insert(value.clone()) {
        results.push(value);
    }
}

fn push_matching_path(
    results: &mut Vec<String>,
    seen: &mut HashSet<String>,
    value: String,
    normalized_query: &str,
    limit: usize,
) {
    if results.len() >= limit {
        return;
    }
    if normalized_query.is_empty() || value.to_lowercase().contains(normalized_query) {
        push_unique_path(results, seen, value);
    }
}

fn looks_like_path_query(query: &str) -> bool {
    query.starts_with('/') || query.starts_with("~/") || query == "~" || query.contains('/')
}

fn append_directory_completion(
    results: &mut Vec<String>,
    seen: &mut HashSet<String>,
    query: &str,
    limit: usize,
) -> Result<(), DaemonError> {
    let expanded = expand_workspace_query_path(query);
    if query == "~" {
        if expanded.is_dir() {
            push_unique_path(results, seen, expanded.display().to_string());
            append_matching_directory_children(results, seen, &expanded, "", limit)?;
        }
        return Ok(());
    }
    if query.ends_with('/') {
        return append_matching_directory_children(results, seen, &expanded, "", limit);
    }

    if expanded.is_dir() {
        push_unique_path(results, seen, expanded.display().to_string());
    }
    let prefix = expanded
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("")
        .to_lowercase();
    let parent = expanded
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/"));
    append_matching_directory_children(results, seen, &parent, &prefix, limit)
}

fn append_matching_directory_children(
    results: &mut Vec<String>,
    seen: &mut HashSet<String>,
    parent: &Path,
    normalized_query: &str,
    limit: usize,
) -> Result<(), DaemonError> {
    if results.len() >= limit || !parent.is_dir() {
        return Ok(());
    }
    let entries = std::fs::read_dir(parent).map_err(|error| DaemonError::LocalTransport {
        operation: "search workspace directories",
        message: error.to_string(),
    })?;
    let mut matches = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("")
            .to_lowercase();
        if normalized_query.is_empty() || name.contains(normalized_query) {
            matches.push(path);
        }
    }
    matches.sort_by(|left, right| {
        directory_match_rank(left, normalized_query)
            .cmp(&directory_match_rank(right, normalized_query))
            .then_with(|| directory_sort_name(left).cmp(&directory_sort_name(right)))
    });
    for path in matches {
        push_unique_path(results, seen, path.display().to_string());
        if results.len() >= limit {
            break;
        }
    }
    Ok(())
}

fn directory_match_rank(path: &Path, normalized_query: &str) -> (u8, u8) {
    let name = directory_sort_name(path);
    let query = normalized_query.trim();
    let exact_rank = if !query.is_empty() && name == query {
        0
    } else if query.is_empty() || name.starts_with(query) {
        1
    } else {
        2
    };
    let hidden_rank = if query.starts_with('.') || !name.starts_with('.') {
        0
    } else {
        1
    };
    (exact_rank, hidden_rank)
}

fn directory_sort_name(path: &Path) -> String {
    path.file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("")
        .to_lowercase()
}

pub(crate) fn expand_workspace_query_path(query: &str) -> PathBuf {
    if query == "~" {
        return std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(query));
    }
    if let Some(rest) = query.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(query)
}
