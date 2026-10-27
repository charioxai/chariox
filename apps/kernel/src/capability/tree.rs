use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::DaemonError;

use super::common::{path_stays_within_root, resolve_worktree_scoped_path};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadDirectoryTreeRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub worktree_root: PathBuf,
    pub path: Option<PathBuf>,
    pub max_depth: usize,
}

impl ReadDirectoryTreeRequest {
    pub fn new(
        session_id: impl Into<String>,
        attachment_id: impl Into<String>,
        worktree_root: PathBuf,
        path: Option<PathBuf>,
        max_depth: usize,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            attachment_id: attachment_id.into(),
            worktree_root,
            path,
            max_depth,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectoryEntry {
    pub relative_path: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadDirectoryTreeResult {
    pub session_id: String,
    pub root_path: PathBuf,
    pub entries: Vec<DirectoryEntry>,
}

#[derive(Debug, Clone, Default)]
pub struct DirectoryTreeService;

impl DirectoryTreeService {
    pub fn new() -> Self {
        Self
    }

    pub fn read_tree(
        &self,
        request: ReadDirectoryTreeRequest,
    ) -> Result<ReadDirectoryTreeResult, DaemonError> {
        let root_path = resolve_worktree_scoped_path(
            &request.session_id,
            &request.worktree_root,
            request.path.as_deref(),
        )?;
        let mut entries = Vec::new();
        visit_tree(
            &request.session_id,
            &root_path,
            &root_path,
            request.max_depth,
            0,
            &mut entries,
        )?;

        Ok(ReadDirectoryTreeResult {
            session_id: request.session_id,
            root_path,
            entries,
        })
    }
}

fn visit_tree(
    session_id: &str,
    base: &Path,
    current: &Path,
    max_depth: usize,
    depth: usize,
    entries: &mut Vec<DirectoryEntry>,
) -> Result<(), DaemonError> {
    if depth > max_depth {
        return Ok(());
    }

    let mut children = std::fs::read_dir(current)
        .map_err(|error| DaemonError::FilesystemCapabilityFailed {
            session_id: session_id.to_string(),
            path: current.display().to_string(),
            message: error.to_string(),
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| DaemonError::FilesystemCapabilityFailed {
            session_id: session_id.to_string(),
            path: current.display().to_string(),
            message: error.to_string(),
        })?;
    children.sort_by_key(|entry| entry.path());

    for child in children {
        let path = child.path();
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
            DaemonError::FilesystemCapabilityFailed {
                session_id: session_id.to_string(),
                path: path.display().to_string(),
                message: error.to_string(),
            }
        })?;
        let child_depth = depth + 1;
        let is_symlink = metadata.file_type().is_symlink();
        let is_dir = metadata.is_dir();
        let include_entry = if is_dir {
            child_depth <= max_depth
        } else {
            child_depth <= max_depth + 1
        };
        if !include_entry {
            continue;
        }
        let kind = if is_symlink {
            "symlink"
        } else if is_dir {
            "directory"
        } else {
            "file"
        };
        let relative = path
            .strip_prefix(base)
            .unwrap_or(&path)
            .display()
            .to_string();
        entries.push(DirectoryEntry {
            relative_path: relative,
            kind: kind.to_string(),
        });

        if is_dir {
            visit_tree(session_id, base, &path, max_depth, child_depth, entries)?;
        } else if is_symlink && path_stays_within_root(base, &path) {
            let target_metadata = std::fs::metadata(&path).map_err(|error| {
                DaemonError::FilesystemCapabilityFailed {
                    session_id: session_id.to_string(),
                    path: path.display().to_string(),
                    message: error.to_string(),
                }
            })?;

            if target_metadata.is_dir() && child_depth <= max_depth {
                visit_tree(session_id, base, &path, max_depth, child_depth, entries)?;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs as unix_fs;

    use super::{DirectoryTreeService, ReadDirectoryTreeRequest};

    #[test]
    fn reads_directory_tree_with_depth_limit() {
        let root = std::env::temp_dir().join("chariox-tree-capability-test");
        let nested = root.join("src/nested");
        fs::create_dir_all(&nested).expect("nested dirs should exist");
        fs::write(root.join("README.md"), "hello").expect("file should exist");
        fs::write(root.join("src/lib.rs"), "pub fn test() {}").expect("file should exist");

        let result = DirectoryTreeService::new()
            .read_tree(ReadDirectoryTreeRequest::new(
                "session-1",
                "attachment-1",
                root,
                None,
                1,
            ))
            .expect("tree read should succeed");

        assert!(result
            .entries
            .iter()
            .any(|entry| entry.relative_path == "README.md"));
        assert!(result
            .entries
            .iter()
            .any(|entry| entry.relative_path == "src"));
        assert!(result
            .entries
            .iter()
            .any(|entry| entry.relative_path == "src/lib.rs"));
        assert!(!result
            .entries
            .iter()
            .any(|entry| entry.relative_path == "src/nested"));
    }

    #[cfg(unix)]
    #[test]
    fn does_not_follow_symlinked_directory_outside_worktree() {
        let root = std::env::temp_dir().join("chariox-tree-capability-symlink-test");
        let outside = std::env::temp_dir().join("chariox-tree-capability-outside");
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&outside);
        fs::create_dir_all(&root).expect("root should exist");
        fs::create_dir_all(&outside).expect("outside should exist");
        fs::write(outside.join("secret.txt"), "secret").expect("secret file should exist");
        unix_fs::symlink(&outside, root.join("outside-link")).expect("symlink should exist");

        let result = DirectoryTreeService::new()
            .read_tree(ReadDirectoryTreeRequest::new(
                "session-1",
                "attachment-1",
                root,
                None,
                3,
            ))
            .expect("tree read should succeed");

        assert!(result
            .entries
            .iter()
            .any(|entry| entry.relative_path == "outside-link" && entry.kind == "symlink"));
        assert!(!result
            .entries
            .iter()
            .any(|entry| entry.relative_path.contains("secret.txt")));
    }
}
