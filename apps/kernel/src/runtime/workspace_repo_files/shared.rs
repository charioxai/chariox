//! Shared workspace file response helpers.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::DaemonError;
use crate::runtime::workspace_git_common::resolve_repo_root;

/// Git metadata enriches file browsing but is not required for a Workspace.
/// Do not hide Git failures when the directory actually belongs to a repository.
pub(super) fn workspace_file_root(path: &str) -> Result<(PathBuf, bool), DaemonError> {
    let root = std::fs::canonicalize(path).map_err(file_error)?;
    if !root.is_dir() {
        return Err(file_error("workspace root is not a directory"));
    }
    for ancestor in root.ancestors() {
        match std::fs::symlink_metadata(ancestor.join(".git")) {
            Ok(_) => return resolve_repo_root(path).map(|root| (root, true)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(file_error(error)),
        }
    }
    Ok((root, false))
}

pub(super) fn contained_directory(root: &Path, prefix: &str) -> Result<PathBuf, DaemonError> {
    if Path::new(prefix).is_absolute() || prefix.split('/').any(|part| part == "..") {
        return Err(file_error("directory path must stay within the workspace"));
    }
    let directory = std::fs::canonicalize(root.join(prefix)).map_err(file_error)?;
    if !directory.starts_with(root) || !directory.is_dir() {
        return Err(file_error("directory path must stay within the workspace"));
    }
    Ok(directory)
}

pub(super) fn file_error(error: impl std::fmt::Display) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "workspace files",
        message: error.to_string(),
    }
}

pub(super) fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
