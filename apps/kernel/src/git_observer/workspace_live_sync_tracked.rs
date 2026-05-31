use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use base64::Engine as _;

use super::{
    GitTurnSnapshot, WorkspaceLiveSyncChange, WorkspaceLiveSyncFileChange,
    WorkspaceLiveSyncFileChangeKind, WorkspaceLiveSyncTrackedFileSnapshot,
};

pub(super) fn change_after_turn(
    before: &GitTurnSnapshot,
    after: &GitTurnSnapshot,
) -> Option<WorkspaceLiveSyncChange> {
    if before.repo_root != after.repo_root || before.worktree_path != after.worktree_path {
        return None;
    }
    if before.is_dirty {
        return dirty_change_after_turn(before, after);
    }
    if before.status_fingerprint == after.status_fingerprint {
        return None;
    }
    let worktree_path = PathBuf::from(&before.worktree_path);
    let changed_status_fingerprint =
        status_delta(&before.status_fingerprint, &after.status_fingerprint);
    let path_changes = path_changes(&changed_status_fingerprint, &worktree_path);
    let changed_paths = path_changes
        .iter()
        .map(|change| change.path.clone())
        .collect::<Vec<_>>();
    if changed_paths.is_empty() {
        return None;
    }
    let file_changes = file_changes(before, &path_changes).unwrap_or_default();
    Some(WorkspaceLiveSyncChange {
        session_id: before.session_id.clone(),
        agent_id: before.agent_id.clone(),
        provider_run_id: before.provider_run_id.clone(),
        prompt_id: before.prompt_id.clone(),
        repo_root: before.repo_root.clone(),
        worktree_path: before.worktree_path.clone(),
        branch: before.branch.clone().or_else(|| after.branch.clone()),
        changed_paths,
        file_changes,
        status_fingerprint: after.status_fingerprint.clone(),
    })
}

fn dirty_change_after_turn(
    before: &GitTurnSnapshot,
    after: &GitTurnSnapshot,
) -> Option<WorkspaceLiveSyncChange> {
    let repo_root = PathBuf::from(&before.repo_root);
    let worktree_path = PathBuf::from(&after.worktree_path);
    let revision = before.head_sha.as_deref().unwrap_or("HEAD");
    let mut paths = before
        .workspace_live_sync_file_snapshots
        .keys()
        .chain(after.workspace_live_sync_file_snapshots.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    for change in path_changes(&after.status_fingerprint, &worktree_path) {
        paths.insert(change.path);
    }
    let mut file_changes = Vec::new();
    for path in paths {
        let before_snapshot = before_file_snapshot(before, &repo_root, revision, &path);
        let after_snapshot = after_file_snapshot(after, &worktree_path, &path);
        if before_snapshot == after_snapshot {
            continue;
        }
        let kind = match (
            before_snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.content_base64.as_ref()),
            after_snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.content_base64.as_ref()),
        ) {
            (None, Some(_)) => WorkspaceLiveSyncFileChangeKind::Added,
            (Some(_), None) => WorkspaceLiveSyncFileChangeKind::Deleted,
            (Some(_), Some(_)) => WorkspaceLiveSyncFileChangeKind::Modified,
            (None, None) => continue,
        };
        let binary = before_snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.binary)
            || after_snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.binary);
        file_changes.push(WorkspaceLiveSyncFileChange {
            path: path.clone(),
            previous_path: None,
            kind,
            binary,
            before_content_base64: before_snapshot.and_then(|snapshot| snapshot.content_base64),
            after_content_base64: after_snapshot.and_then(|snapshot| snapshot.content_base64),
        });
    }
    if file_changes.is_empty() {
        return None;
    }
    let changed_paths = file_changes
        .iter()
        .map(|change| change.path.clone())
        .collect::<Vec<_>>();
    Some(WorkspaceLiveSyncChange {
        session_id: before.session_id.clone(),
        agent_id: before.agent_id.clone(),
        provider_run_id: before.provider_run_id.clone(),
        prompt_id: before.prompt_id.clone(),
        repo_root: before.repo_root.clone(),
        worktree_path: before.worktree_path.clone(),
        branch: before.branch.clone().or_else(|| after.branch.clone()),
        changed_paths,
        file_changes,
        status_fingerprint: after.status_fingerprint.clone(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TrackedPathChange {
    path: String,
    previous_path: Option<String>,
    kind: WorkspaceLiveSyncFileChangeKind,
}

fn status_delta(before_status: &str, after_status: &str) -> String {
    if before_status.is_empty() {
        return after_status.to_string();
    }
    let before_lines = before_status
        .lines()
        .map(str::trim_end)
        .collect::<BTreeSet<_>>();
    after_status
        .lines()
        .map(str::trim_end)
        .filter(|line| !before_lines.contains(line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn path_changes(status_fingerprint: &str, worktree_path: &Path) -> Vec<TrackedPathChange> {
    let ignore_patterns = ignore_patterns(worktree_path);
    let mut paths = BTreeMap::new();
    for line in status_fingerprint.lines() {
        let line = line.trim_end();
        if line.len() < 4 {
            continue;
        }
        let status = &line[..2];
        let raw_path = line[3..].trim();
        let (previous_path, path) = if let Some((previous, next)) = raw_path.split_once(" -> ") {
            (Some(unquote_path(previous)), unquote_path(next))
        } else {
            (None, unquote_path(raw_path))
        };
        if path.is_empty()
            || excluded_path(&path, &ignore_patterns)
            || previous_path
                .as_deref()
                .is_some_and(|path| excluded_path(path, &ignore_patterns))
        {
            continue;
        }
        let kind = if status == "??" {
            WorkspaceLiveSyncFileChangeKind::Added
        } else if status.contains('R') {
            WorkspaceLiveSyncFileChangeKind::Renamed
        } else if status.contains('D') {
            WorkspaceLiveSyncFileChangeKind::Deleted
        } else {
            WorkspaceLiveSyncFileChangeKind::Modified
        };
        paths.insert(
            path.clone(),
            TrackedPathChange {
                path,
                previous_path,
                kind,
            },
        );
    }
    paths.into_values().collect()
}

pub(super) fn dirty_file_snapshots(
    worktree_path: &Path,
    status_fingerprint: &str,
) -> BTreeMap<String, WorkspaceLiveSyncTrackedFileSnapshot> {
    path_changes(status_fingerprint, worktree_path)
        .into_iter()
        .map(|change| {
            let snapshot = worktree_snapshot(worktree_path, &change.path);
            (
                change.path,
                WorkspaceLiveSyncTrackedFileSnapshot::from_content_snapshot(snapshot),
            )
        })
        .collect()
}

fn before_file_snapshot(
    before: &GitTurnSnapshot,
    repo_root: &Path,
    revision: &str,
    path: &str,
) -> Option<WorkspaceLiveSyncTrackedFileSnapshot> {
    before
        .workspace_live_sync_file_snapshots
        .get(path)
        .cloned()
        .or_else(|| {
            git_blob_snapshot(repo_root, revision, path).map(|snapshot| {
                WorkspaceLiveSyncTrackedFileSnapshot::from_content_snapshot(Some(snapshot))
            })
        })
}

fn after_file_snapshot(
    after: &GitTurnSnapshot,
    worktree_path: &Path,
    path: &str,
) -> Option<WorkspaceLiveSyncTrackedFileSnapshot> {
    after
        .workspace_live_sync_file_snapshots
        .get(path)
        .cloned()
        .or_else(|| {
            worktree_snapshot(worktree_path, path).map(|snapshot| {
                WorkspaceLiveSyncTrackedFileSnapshot::from_content_snapshot(Some(snapshot))
            })
        })
}

fn file_changes(
    before: &GitTurnSnapshot,
    path_changes: &[TrackedPathChange],
) -> Option<Vec<WorkspaceLiveSyncFileChange>> {
    let repo_root = PathBuf::from(&before.repo_root);
    let worktree_path = PathBuf::from(&before.worktree_path);
    let revision = before.head_sha.as_deref().unwrap_or("HEAD");
    Some(
        path_changes
            .iter()
            .map(|change| {
                let before_path = change.previous_path.as_deref().unwrap_or(&change.path);
                let before_snapshot = match change.kind {
                    WorkspaceLiveSyncFileChangeKind::Added => None,
                    WorkspaceLiveSyncFileChangeKind::Modified
                    | WorkspaceLiveSyncFileChangeKind::Deleted
                    | WorkspaceLiveSyncFileChangeKind::Renamed => {
                        git_blob_snapshot(&repo_root, revision, before_path)
                    }
                };
                let after_snapshot = match change.kind {
                    WorkspaceLiveSyncFileChangeKind::Deleted => None,
                    WorkspaceLiveSyncFileChangeKind::Added
                    | WorkspaceLiveSyncFileChangeKind::Modified
                    | WorkspaceLiveSyncFileChangeKind::Renamed => {
                        worktree_snapshot(&worktree_path, &change.path)
                    }
                };
                let binary = before_snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.binary)
                    || after_snapshot
                        .as_ref()
                        .is_some_and(|snapshot| snapshot.binary);
                WorkspaceLiveSyncFileChange {
                    path: change.path.clone(),
                    previous_path: change.previous_path.clone(),
                    kind: change.kind,
                    binary,
                    before_content_base64: before_snapshot.map(|snapshot| snapshot.content_base64),
                    after_content_base64: after_snapshot.map(|snapshot| snapshot.content_base64),
                }
            })
            .collect(),
    )
}

#[derive(Debug, Clone)]
struct TrackedContentSnapshot {
    content_base64: String,
    binary: bool,
}

impl WorkspaceLiveSyncTrackedFileSnapshot {
    fn from_content_snapshot(snapshot: Option<TrackedContentSnapshot>) -> Self {
        Self {
            binary: snapshot.as_ref().is_some_and(|snapshot| snapshot.binary),
            content_base64: snapshot.map(|snapshot| snapshot.content_base64),
        }
    }
}

fn git_blob_snapshot(
    repo_root: &Path,
    revision: &str,
    path: &str,
) -> Option<TrackedContentSnapshot> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("show")
        .arg(format!("{revision}:{path}"))
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| content_snapshot(output.stdout))
}

fn worktree_snapshot(worktree_path: &Path, path: &str) -> Option<TrackedContentSnapshot> {
    std::fs::read(worktree_path.join(path))
        .ok()
        .map(content_snapshot)
}

fn content_snapshot(bytes: Vec<u8>) -> TrackedContentSnapshot {
    TrackedContentSnapshot {
        binary: bytes.contains(&0),
        content_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
    }
}

fn unquote_path(path: &str) -> String {
    path.trim().trim_matches('"').to_string()
}

fn excluded_path(path: &str, ignore_patterns: &[String]) -> bool {
    crate::workspace_live_sync_ignore::workspace_live_sync_ignored_path(path, ignore_patterns)
}

fn ignore_patterns(worktree_path: &Path) -> Vec<String> {
    crate::workspace_live_sync_ignore::workspace_live_sync_user_ignore_patterns(worktree_path)
}
