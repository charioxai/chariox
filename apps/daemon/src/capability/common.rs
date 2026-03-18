use std::path::{Component, Path, PathBuf};

use crate::error::DaemonError;

pub fn resolve_worktree_scoped_path(
    session_id: &str,
    worktree_root: &Path,
    requested_path: Option<&Path>,
) -> Result<PathBuf, DaemonError> {
    let root = std::fs::canonicalize(worktree_root).map_err(|error| {
        DaemonError::FilesystemCapabilityFailed {
            session_id: session_id.to_string(),
            path: worktree_root.display().to_string(),
            message: error.to_string(),
        }
    })?;

    let candidate = requested_path
        .map(PathBuf::from)
        .unwrap_or_else(|| root.clone());
    let candidate = if candidate.is_absolute() {
        candidate
    } else {
        root.join(candidate)
    };
    let normalized = normalize_path(&candidate);

    let boundary_checked = canonicalize_with_virtual_tail(&normalized).map_err(|error| {
        DaemonError::FilesystemCapabilityFailed {
            session_id: session_id.to_string(),
            path: normalized.display().to_string(),
            message: error.to_string(),
        }
    })?;

    if !boundary_checked.starts_with(&root) {
        return Err(DaemonError::PathOutsideWorktree {
            session_id: session_id.to_string(),
            path: boundary_checked.display().to_string(),
            worktree_root: root.display().to_string(),
        });
    }

    Ok(boundary_checked)
}

pub fn path_stays_within_root(root: &Path, candidate: &Path) -> bool {
    std::fs::canonicalize(candidate)
        .map(|resolved| resolved.starts_with(root))
        .unwrap_or(false)
}

fn canonicalize_with_virtual_tail(path: &Path) -> std::io::Result<PathBuf> {
    if path.exists() {
        return std::fs::canonicalize(path);
    }

    let mut missing_tail = Vec::new();
    let mut current = path;

    while !current.exists() {
        let Some(name) = current.file_name() else {
            break;
        };
        missing_tail.push(PathBuf::from(name));
        let Some(parent) = current.parent() else {
            break;
        };
        current = parent;
    }

    let mut resolved = std::fs::canonicalize(current)?;
    for component in missing_tail.iter().rev() {
        resolved.push(component);
    }

    Ok(resolved)
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }

    normalized
}
