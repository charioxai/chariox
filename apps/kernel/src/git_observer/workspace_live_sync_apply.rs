use std::path::{Path, PathBuf};

use base64::Engine as _;

use super::{
    WorkspaceLiveSyncApplyStatus, WorkspaceLiveSyncChange, WorkspaceLiveSyncFileChange,
    WorkspaceLiveSyncFileChangeKind, WorkspaceLiveSyncPathApplyResult,
};

pub(crate) fn apply_workspace_live_sync_change_to_target(
    change: &WorkspaceLiveSyncChange,
    target_root: &Path,
) -> Vec<WorkspaceLiveSyncPathApplyResult> {
    change
        .file_changes
        .iter()
        .map(|file_change| {
            apply_workspace_live_sync_file_change_to_target(file_change, target_root)
        })
        .collect()
}

fn apply_workspace_live_sync_file_change_to_target(
    file_change: &WorkspaceLiveSyncFileChange,
    target_root: &Path,
) -> WorkspaceLiveSyncPathApplyResult {
    let path = file_change.path.clone();
    let target_path = match workspace_live_sync_target_path(target_root, &file_change.path) {
        Some(path) => path,
        None => {
            return WorkspaceLiveSyncPathApplyResult {
                path,
                status: WorkspaceLiveSyncApplyStatus::FailedIo,
                message: "workspace live sync path must be relative and cannot contain `..`"
                    .to_string(),
            };
        }
    };
    let previous_target_path = file_change
        .previous_path
        .as_deref()
        .and_then(|path| workspace_live_sync_target_path(target_root, path));
    let ignore_patterns = workspace_live_sync_ignore_patterns(target_root);
    if workspace_live_sync_excluded_path(&file_change.path, &ignore_patterns)
        || file_change
            .previous_path
            .as_deref()
            .is_some_and(|path| workspace_live_sync_excluded_path(path, &ignore_patterns))
    {
        return workspace_live_sync_conflict(
            &path,
            "path is ignored by target workspace live sync rules",
        );
    }
    let before_bytes =
        match workspace_live_sync_decode_optional(file_change.before_content_base64.as_deref()) {
            Ok(bytes) => bytes,
            Err(message) => {
                return WorkspaceLiveSyncPathApplyResult {
                    path,
                    status: WorkspaceLiveSyncApplyStatus::FailedIo,
                    message,
                };
            }
        };
    let after_bytes =
        match workspace_live_sync_decode_optional(file_change.after_content_base64.as_deref()) {
            Ok(bytes) => bytes,
            Err(message) => {
                return WorkspaceLiveSyncPathApplyResult {
                    path,
                    status: WorkspaceLiveSyncApplyStatus::FailedIo,
                    message,
                };
            }
        };
    match file_change.kind {
        WorkspaceLiveSyncFileChangeKind::Added => {
            workspace_live_sync_apply_add(&path, &target_path, after_bytes)
        }
        WorkspaceLiveSyncFileChangeKind::Modified => {
            workspace_live_sync_apply_modify(&path, &target_path, before_bytes, after_bytes)
        }
        WorkspaceLiveSyncFileChangeKind::Deleted => {
            workspace_live_sync_apply_delete(&path, &target_path, before_bytes)
        }
        WorkspaceLiveSyncFileChangeKind::Renamed => workspace_live_sync_apply_rename(
            &path,
            previous_target_path.as_deref(),
            &target_path,
            before_bytes,
            after_bytes,
        ),
    }
}

fn workspace_live_sync_apply_add(
    path: &str,
    target_path: &Path,
    after_bytes: Option<Vec<u8>>,
) -> WorkspaceLiveSyncPathApplyResult {
    let Some(after_bytes) = after_bytes else {
        return workspace_live_sync_failed(path, "workspace live sync add has no after content");
    };
    match std::fs::read(target_path) {
        Ok(current) if current == after_bytes => {
            return workspace_live_sync_applied(
                path,
                "target path already contains source content",
            );
        }
        Ok(_) => return workspace_live_sync_conflict(path, "target path already exists"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return workspace_live_sync_failed(
                path,
                format!("failed to read target path before add: {error}"),
            );
        }
    }
    workspace_live_sync_write_file(path, target_path, &after_bytes)
}

fn workspace_live_sync_apply_modify(
    path: &str,
    target_path: &Path,
    before_bytes: Option<Vec<u8>>,
    after_bytes: Option<Vec<u8>>,
) -> WorkspaceLiveSyncPathApplyResult {
    let (Some(before_bytes), Some(after_bytes)) = (before_bytes, after_bytes) else {
        return workspace_live_sync_failed(path, "workspace live sync modify is missing content");
    };
    match std::fs::read(target_path) {
        Ok(current) if current == before_bytes => {
            workspace_live_sync_write_file(path, target_path, &after_bytes)
        }
        Ok(current) if current == after_bytes => {
            workspace_live_sync_applied(path, "target path already contains source content")
        }
        Ok(current) => workspace_live_sync_rebase_modify(
            path,
            target_path,
            &before_bytes,
            &after_bytes,
            &current,
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            workspace_live_sync_conflict(path, "target path is missing")
        }
        Err(error) => workspace_live_sync_failed(
            path,
            format!("failed to read target path before apply: {error}"),
        ),
    }
}

fn workspace_live_sync_rebase_modify(
    path: &str,
    target_path: &Path,
    before_bytes: &[u8],
    after_bytes: &[u8],
    current_bytes: &[u8],
) -> WorkspaceLiveSyncPathApplyResult {
    let Ok(before) = std::str::from_utf8(before_bytes) else {
        return workspace_live_sync_conflict(path, "binary target content changed before apply");
    };
    let Ok(after) = std::str::from_utf8(after_bytes) else {
        return workspace_live_sync_conflict(path, "binary source content cannot be rebased");
    };
    let Ok(current) = std::str::from_utf8(current_bytes) else {
        return workspace_live_sync_conflict(path, "binary target content cannot be rebased");
    };
    let Some(rebased) = workspace_live_sync_rebase_text(before, after, current) else {
        return workspace_live_sync_conflict(
            path,
            "target content changed in an overlapping area before apply",
        );
    };
    if let Some(parent) = target_path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            return workspace_live_sync_failed(
                path,
                format!("failed to create target directory: {error}"),
            );
        }
    }
    match std::fs::write(target_path, rebased.as_bytes()) {
        Ok(()) => workspace_live_sync_rebased(path, "rebased over non-overlapping target change"),
        Err(error) => workspace_live_sync_failed(
            path,
            format!("failed to write rebased target content: {error}"),
        ),
    }
}

fn workspace_live_sync_rebase_text(before: &str, after: &str, current: &str) -> Option<String> {
    let before_lines = workspace_live_sync_lines(before);
    let after_lines = workspace_live_sync_lines(after);
    let current_lines = workspace_live_sync_lines(current);
    let source = workspace_live_sync_changed_range(&before_lines, &after_lines);
    let target = workspace_live_sync_changed_range(&before_lines, &current_lines);
    if workspace_live_sync_ranges_overlap(
        source.before_start,
        source.before_end,
        target.before_start,
        target.before_end,
    ) {
        return None;
    }
    let target_delta = target.changed_end as isize
        - target.before_end as isize
        - (target.changed_start as isize - target.before_start as isize);
    let offset = if target.before_end <= source.before_start {
        target_delta
    } else {
        0
    };
    let current_start = (source.before_start as isize + offset).try_into().ok()?;
    let current_end = (source.before_end as isize + offset).try_into().ok()?;
    if current_start > current_end || current_end > current_lines.len() {
        return None;
    }
    let mut rebased = Vec::new();
    rebased.extend_from_slice(&current_lines[..current_start]);
    rebased.extend_from_slice(&after_lines[source.changed_start..source.changed_end]);
    rebased.extend_from_slice(&current_lines[current_end..]);
    Some(rebased.concat())
}

#[derive(Debug, Clone, Copy)]
struct WorkspaceLiveSyncTextChangeRange {
    before_start: usize,
    before_end: usize,
    changed_start: usize,
    changed_end: usize,
}

fn workspace_live_sync_changed_range(
    before: &[String],
    changed: &[String],
) -> WorkspaceLiveSyncTextChangeRange {
    let mut prefix = 0usize;
    while prefix < before.len() && prefix < changed.len() && before[prefix] == changed[prefix] {
        prefix += 1;
    }
    let mut suffix = 0usize;
    while suffix + prefix < before.len()
        && suffix + prefix < changed.len()
        && before[before.len() - 1 - suffix] == changed[changed.len() - 1 - suffix]
    {
        suffix += 1;
    }
    WorkspaceLiveSyncTextChangeRange {
        before_start: prefix,
        before_end: before.len() - suffix,
        changed_start: prefix,
        changed_end: changed.len() - suffix,
    }
}

fn workspace_live_sync_ranges_overlap(
    left_start: usize,
    left_end: usize,
    right_start: usize,
    right_end: usize,
) -> bool {
    if left_start == left_end {
        return right_start <= left_start && left_start <= right_end;
    }
    if right_start == right_end {
        return left_start <= right_start && right_start <= left_end;
    }
    left_start < right_end && right_start < left_end
}

fn workspace_live_sync_lines(value: &str) -> Vec<String> {
    if value.is_empty() {
        return Vec::new();
    }
    value.split_inclusive('\n').map(str::to_string).collect()
}

fn workspace_live_sync_apply_delete(
    path: &str,
    target_path: &Path,
    before_bytes: Option<Vec<u8>>,
) -> WorkspaceLiveSyncPathApplyResult {
    let Some(before_bytes) = before_bytes else {
        return workspace_live_sync_failed(
            path,
            "workspace live sync delete has no before content",
        );
    };
    match std::fs::read(target_path) {
        Ok(current) if current == before_bytes => match std::fs::remove_file(target_path) {
            Ok(()) => workspace_live_sync_applied(path, "deleted target path"),
            Err(error) => {
                workspace_live_sync_failed(path, format!("failed to delete target path: {error}"))
            }
        },
        Ok(_) => workspace_live_sync_conflict(path, "target content changed before delete"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            workspace_live_sync_applied(path, "target path is already missing")
        }
        Err(error) => workspace_live_sync_failed(
            path,
            format!("failed to read target path before delete: {error}"),
        ),
    }
}

fn workspace_live_sync_apply_rename(
    path: &str,
    previous_target_path: Option<&Path>,
    target_path: &Path,
    before_bytes: Option<Vec<u8>>,
    after_bytes: Option<Vec<u8>>,
) -> WorkspaceLiveSyncPathApplyResult {
    let (Some(previous_target_path), Some(before_bytes), Some(after_bytes)) =
        (previous_target_path, before_bytes, after_bytes)
    else {
        return workspace_live_sync_failed(path, "workspace live sync rename is missing content");
    };
    match std::fs::read(target_path) {
        Ok(current) if current == after_bytes => match std::fs::read(previous_target_path) {
            Ok(previous_current) if previous_current == before_bytes => {
                match std::fs::remove_file(previous_target_path) {
                    Ok(()) => {
                        return workspace_live_sync_applied(
                            path,
                            "completed already-written rename target",
                        );
                    }
                    Err(error) => {
                        return workspace_live_sync_failed(
                            path,
                            format!(
                                "failed to remove rename source after idempotent write: {error}"
                            ),
                        )
                    }
                }
            }
            Ok(_) => {
                return workspace_live_sync_conflict(
                    path,
                    "rename source content changed before apply",
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return workspace_live_sync_applied(
                    path,
                    "rename target already contains source content",
                );
            }
            Err(error) => {
                return workspace_live_sync_failed(
                    path,
                    format!("failed to read rename source before idempotent apply: {error}"),
                );
            }
        },
        Ok(_) => return workspace_live_sync_conflict(path, "rename target path already exists"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return workspace_live_sync_failed(
                path,
                format!("failed to read rename target before apply: {error}"),
            );
        }
    }
    match std::fs::read(previous_target_path) {
        Ok(current) if current == before_bytes => {}
        Ok(_) => {
            return workspace_live_sync_conflict(
                path,
                "rename source content changed before apply",
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return workspace_live_sync_conflict(path, "rename source path is missing");
        }
        Err(error) => {
            return workspace_live_sync_failed(
                path,
                format!("failed to read rename source before apply: {error}"),
            );
        }
    }
    if let Some(parent) = target_path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            return workspace_live_sync_failed(
                path,
                format!("failed to create target directory: {error}"),
            );
        }
    }
    if let Err(error) = std::fs::write(target_path, after_bytes) {
        return workspace_live_sync_failed(path, format!("failed to write target: {error}"));
    }
    match std::fs::remove_file(previous_target_path) {
        Ok(()) => workspace_live_sync_applied(path, "renamed target path"),
        Err(error) => workspace_live_sync_failed(
            path,
            format!("failed to remove rename source after write: {error}"),
        ),
    }
}

fn workspace_live_sync_write_file(
    path: &str,
    target_path: &Path,
    bytes: &[u8],
) -> WorkspaceLiveSyncPathApplyResult {
    if let Some(parent) = target_path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            return workspace_live_sync_failed(
                path,
                format!("failed to create target directory: {error}"),
            );
        }
    }
    match std::fs::write(target_path, bytes) {
        Ok(()) => workspace_live_sync_applied(path, "applied target content"),
        Err(error) => {
            workspace_live_sync_failed(path, format!("failed to write target content: {error}"))
        }
    }
}

fn workspace_live_sync_decode_optional(value: Option<&str>) -> Result<Option<Vec<u8>>, String> {
    value
        .map(|value| {
            base64::engine::general_purpose::STANDARD
                .decode(value)
                .map_err(|error| {
                    format!("workspace live sync content is not valid base64: {error}")
                })
        })
        .transpose()
}

fn workspace_live_sync_target_path(target_root: &Path, path: &str) -> Option<PathBuf> {
    let relative = Path::new(path);
    if relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::Prefix(_)
            )
        })
    {
        return None;
    }
    Some(target_root.join(relative))
}

fn workspace_live_sync_applied(
    path: &str,
    message: impl Into<String>,
) -> WorkspaceLiveSyncPathApplyResult {
    WorkspaceLiveSyncPathApplyResult {
        path: path.to_string(),
        status: WorkspaceLiveSyncApplyStatus::Applied,
        message: message.into(),
    }
}

fn workspace_live_sync_rebased(
    path: &str,
    message: impl Into<String>,
) -> WorkspaceLiveSyncPathApplyResult {
    WorkspaceLiveSyncPathApplyResult {
        path: path.to_string(),
        status: WorkspaceLiveSyncApplyStatus::Rebased,
        message: message.into(),
    }
}

fn workspace_live_sync_conflict(
    path: &str,
    message: impl Into<String>,
) -> WorkspaceLiveSyncPathApplyResult {
    WorkspaceLiveSyncPathApplyResult {
        path: path.to_string(),
        status: WorkspaceLiveSyncApplyStatus::SkippedConflict,
        message: message.into(),
    }
}

fn workspace_live_sync_failed(
    path: &str,
    message: impl Into<String>,
) -> WorkspaceLiveSyncPathApplyResult {
    WorkspaceLiveSyncPathApplyResult {
        path: path.to_string(),
        status: WorkspaceLiveSyncApplyStatus::FailedIo,
        message: message.into(),
    }
}

fn workspace_live_sync_excluded_path(path: &str, ignore_patterns: &[String]) -> bool {
    crate::workspace_live_sync_ignore::workspace_live_sync_ignored_path(path, ignore_patterns)
}

fn workspace_live_sync_ignore_patterns(worktree_path: &Path) -> Vec<String> {
    crate::workspace_live_sync_ignore::workspace_live_sync_user_ignore_patterns(worktree_path)
}
