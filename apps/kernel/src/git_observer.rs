use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, MutexGuard};

use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::error::DaemonError;
use crate::history::{
    HistoryAttributionConfidence, HistoryEvent, HistoryEventKind, HistoryEventRole,
    HistoryEventTurnContext, OperationalHistoryStore,
};
use crate::transport::relay_peer::RemoteGitObservation;

#[derive(Debug, Clone)]
pub(crate) struct GitTurnContext {
    pub session_id: String,
    pub agent_id: String,
    pub provider: String,
    pub model: String,
    pub provider_run_id: String,
    pub provider_session_id: Option<String>,
    pub prompt_id: String,
    pub turn_id: String,
    pub worktree_path: PathBuf,
    pub workspace_live_sync_tracked: bool,
    pub machine_id: Option<String>,
    pub prompt_summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct GitTurnSnapshot {
    pub session_id: String,
    pub agent_id: String,
    pub provider: String,
    pub model: String,
    pub provider_run_id: String,
    pub provider_session_id: Option<String>,
    pub prompt_id: String,
    pub turn_id: String,
    pub machine_id: Option<String>,
    pub prompt_summary: String,
    pub repo_root: String,
    pub worktree_path: String,
    pub branch: Option<String>,
    pub head_sha: Option<String>,
    pub upstream_ref: Option<String>,
    pub ahead_count: Option<u32>,
    pub status_fingerprint: String,
    pub is_dirty: bool,
    pub workspace_live_sync_tracked: bool,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct GitTurnSnapshotStore {
    inner: Arc<Mutex<BTreeMap<String, GitTurnSnapshot>>>,
}

impl GitTurnSnapshotStore {
    fn read(&self) -> MutexGuard<'_, BTreeMap<String, GitTurnSnapshot>> {
        self.inner.lock().expect("git turn snapshot mutex poisoned")
    }

    fn key(provider_run_id: &str, prompt_id: &str) -> String {
        format!("{provider_run_id}:{prompt_id}")
    }

    pub(crate) fn insert(&self, snapshot: GitTurnSnapshot) {
        self.inner
            .lock()
            .expect("git turn snapshot mutex poisoned")
            .insert(
                Self::key(&snapshot.provider_run_id, &snapshot.prompt_id),
                snapshot,
            );
    }

    pub(crate) fn remove(&self, provider_run_id: &str, prompt_id: &str) -> Option<GitTurnSnapshot> {
        self.inner
            .lock()
            .expect("git turn snapshot mutex poisoned")
            .remove(&Self::key(provider_run_id, prompt_id))
    }

    pub(crate) fn remove_for_provider_run(&self, provider_run_id: &str) -> Option<GitTurnSnapshot> {
        let mut guard = self.inner.lock().expect("git turn snapshot mutex poisoned");
        let key = guard
            .keys()
            .find(|key| key.starts_with(&format!("{provider_run_id}:")))
            .cloned()?;
        guard.remove(&key)
    }

    pub(crate) fn candidates_for(&self, snapshot: &GitTurnSnapshot) -> GitAttributionCandidates {
        let mut agent_ids = BTreeSet::new();
        let mut prompt_ids = BTreeSet::new();
        let mut turn_ids = BTreeSet::new();
        for candidate in self.read().values() {
            if candidate.repo_root == snapshot.repo_root
                && candidate.worktree_path == snapshot.worktree_path
                && candidate.head_sha == snapshot.head_sha
            {
                agent_ids.insert(candidate.agent_id.clone());
                prompt_ids.insert(candidate.prompt_id.clone());
                turn_ids.insert(candidate.turn_id.clone());
            }
        }
        agent_ids.insert(snapshot.agent_id.clone());
        prompt_ids.insert(snapshot.prompt_id.clone());
        turn_ids.insert(snapshot.turn_id.clone());
        GitAttributionCandidates {
            agent_ids: agent_ids.into_iter().collect(),
            prompt_ids: prompt_ids.into_iter().collect(),
            turn_ids: turn_ids.into_iter().collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackedWorkspaceLiveSyncTurnChange {
    pub session_id: String,
    pub agent_id: String,
    pub provider_run_id: String,
    pub prompt_id: String,
    pub repo_root: String,
    pub worktree_path: String,
    pub branch: Option<String>,
    pub changed_paths: Vec<String>,
    pub file_changes: Vec<TrackedWorkspaceLiveSyncFileChange>,
    pub status_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackedWorkspaceLiveSyncFileChange {
    pub path: String,
    #[serde(default)]
    pub previous_path: Option<String>,
    pub kind: TrackedWorkspaceLiveSyncFileChangeKind,
    #[serde(default)]
    pub before_content_base64: Option<String>,
    #[serde(default)]
    pub after_content_base64: Option<String>,
    pub binary: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackedWorkspaceLiveSyncFileChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct TrackedWorkspaceLiveSyncJournal {
    inner: Arc<Mutex<TrackedWorkspaceLiveSyncJournalState>>,
}

#[derive(Debug, Clone, Default)]
struct TrackedWorkspaceLiveSyncJournalState {
    entries: Vec<TrackedWorkspaceLiveSyncTurnChange>,
    target_results: Vec<TrackedWorkspaceLiveSyncTargetResult>,
}

impl TrackedWorkspaceLiveSyncJournal {
    pub(crate) fn append(&self, change: TrackedWorkspaceLiveSyncTurnChange) {
        self.inner
            .lock()
            .expect("tracked workspace live sync journal mutex poisoned")
            .entries
            .push(change);
    }

    pub(crate) fn record_target_results(&self, results: Vec<TrackedWorkspaceLiveSyncTargetResult>) {
        if results.is_empty() {
            return;
        }
        self.inner
            .lock()
            .expect("tracked workspace live sync journal mutex poisoned")
            .target_results
            .extend(results);
    }

    pub(crate) fn target_results_for_session(
        &self,
        session_id: &str,
    ) -> Vec<TrackedWorkspaceLiveSyncTargetResult> {
        self.inner
            .lock()
            .expect("tracked workspace live sync journal mutex poisoned")
            .target_results
            .iter()
            .filter(|result| result.session_id == session_id)
            .cloned()
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackedWorkspaceLiveSyncTargetResult {
    pub session_id: String,
    pub link_id: String,
    pub link_name: String,
    pub source_agent_id: String,
    pub source_worktree_path: String,
    pub target_user_id: String,
    pub target_machine_id: String,
    pub target_kernel_id: String,
    pub target_repo_root: String,
    pub path_results: Vec<TrackedWorkspaceLiveSyncPathApplyResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackedWorkspaceLiveSyncPathApplyResult {
    pub path: String,
    pub status: TrackedWorkspaceLiveSyncApplyStatus,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackedWorkspaceLiveSyncApplyStatus {
    Applied,
    Rebased,
    SkippedConflict,
    FailedIo,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct GitAttributionCandidates {
    pub agent_ids: Vec<String>,
    pub prompt_ids: Vec<String>,
    pub turn_ids: Vec<String>,
}

impl GitAttributionCandidates {
    fn confidence(&self) -> HistoryAttributionConfidence {
        if self.agent_ids.len() > 1 || self.prompt_ids.len() > 1 {
            HistoryAttributionConfidence::Ambiguous
        } else {
            HistoryAttributionConfidence::Definite
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GitCommitSummary {
    sha: String,
    subject: String,
    author_name: Option<String>,
    author_email: Option<String>,
    commit_timestamp_s: Option<i64>,
    changed_paths: Vec<String>,
}

pub(crate) fn capture_turn_snapshot(context: GitTurnContext) -> Option<GitTurnSnapshot> {
    let repo_root = git_output(&context.worktree_path, &["rev-parse", "--show-toplevel"])?;
    let repo_root = normalize_path(repo_root.trim());
    let status_output =
        git_output(&context.worktree_path, &["status", "--porcelain=v1"]).unwrap_or_default();
    let status_fingerprint = status_output
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n");
    Some(GitTurnSnapshot {
        session_id: context.session_id,
        agent_id: context.agent_id,
        provider: context.provider,
        model: context.model,
        provider_run_id: context.provider_run_id,
        provider_session_id: context.provider_session_id,
        prompt_id: context.prompt_id,
        turn_id: context.turn_id,
        machine_id: context.machine_id,
        prompt_summary: truncate_for_metadata(&context.prompt_summary, 500),
        repo_root,
        worktree_path: normalize_path(&context.worktree_path),
        branch: git_output(
            &context.worktree_path,
            &["rev-parse", "--abbrev-ref", "HEAD"],
        )
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty()),
        head_sha: git_output(&context.worktree_path, &["rev-parse", "--verify", "HEAD"])
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        upstream_ref: git_output(
            &context.worktree_path,
            &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
        )
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty()),
        ahead_count: git_output(
            &context.worktree_path,
            &["rev-list", "--count", "@{u}..HEAD"],
        )
        .and_then(|value| value.trim().parse::<u32>().ok()),
        is_dirty: !status_fingerprint.is_empty(),
        status_fingerprint,
        workspace_live_sync_tracked: context.workspace_live_sync_tracked,
    })
}

pub(crate) fn observe_after_turn(
    before: GitTurnSnapshot,
    after: GitTurnSnapshot,
    candidates: GitAttributionCandidates,
    history: &OperationalHistoryStore,
) -> Result<Vec<HistoryEvent>, DaemonError> {
    append_observations(history, observations_after_turn(before, after, candidates))
}

pub(crate) fn observations_after_turn(
    before: GitTurnSnapshot,
    after: GitTurnSnapshot,
    candidates: GitAttributionCandidates,
) -> Vec<RemoteGitObservation> {
    let mut events = Vec::new();
    for commit in commits_between(&before, &after) {
        events.push(git_observation(
            HistoryEventKind::GitCommitDetected,
            Some(git_commit_content(&commit)),
            git_commit_metadata(&before, &after, &commit),
            &before,
            &candidates,
        ));
    }

    if before.status_fingerprint != after.status_fingerprint {
        events.push(git_observation(
            HistoryEventKind::GitWorktreeChanged,
            Some("Git worktree changed during agent turn.".to_string()),
            git_worktree_metadata(&before, &after),
            &before,
            &candidates,
        ));
    }
    if !before.is_dirty && after.is_dirty {
        events.push(git_observation(
            HistoryEventKind::GitWorktreeDirty,
            Some("Git worktree became dirty during agent turn.".to_string()),
            git_worktree_metadata(&before, &after),
            &before,
            &candidates,
        ));
    } else if before.is_dirty && !after.is_dirty {
        events.push(git_observation(
            HistoryEventKind::GitWorktreeClean,
            Some("Git worktree became clean during agent turn.".to_string()),
            git_worktree_metadata(&before, &after),
            &before,
            &candidates,
        ));
    }
    if before.ahead_count.unwrap_or_default() > 0
        && after.ahead_count == Some(0)
        && before.upstream_ref == after.upstream_ref
    {
        events.push(git_observation(
            HistoryEventKind::GitPushDetected,
            Some("Git push detected during agent turn.".to_string()),
            git_worktree_metadata(&before, &after),
            &before,
            &candidates,
        ));
    }
    events
}

pub(crate) fn tracked_workspace_live_sync_change_after_turn(
    before: &GitTurnSnapshot,
    after: &GitTurnSnapshot,
) -> Option<TrackedWorkspaceLiveSyncTurnChange> {
    if before.is_dirty || before.status_fingerprint == after.status_fingerprint {
        return None;
    }
    if before.repo_root != after.repo_root || before.worktree_path != after.worktree_path {
        return None;
    }
    let path_changes = tracked_workspace_live_sync_path_changes(&after.status_fingerprint);
    let changed_paths = path_changes
        .iter()
        .map(|change| change.path.clone())
        .collect::<Vec<_>>();
    if changed_paths.is_empty() {
        return None;
    }
    let file_changes =
        tracked_workspace_live_sync_file_changes(before, &path_changes).unwrap_or_default();
    Some(TrackedWorkspaceLiveSyncTurnChange {
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
struct TrackedWorkspaceLiveSyncPathChange {
    path: String,
    previous_path: Option<String>,
    kind: TrackedWorkspaceLiveSyncFileChangeKind,
}

fn tracked_workspace_live_sync_path_changes(
    status_fingerprint: &str,
) -> Vec<TrackedWorkspaceLiveSyncPathChange> {
    let mut paths = BTreeMap::new();
    for line in status_fingerprint.lines() {
        let line = line.trim_end();
        if line.len() < 4 {
            continue;
        }
        let status = &line[..2];
        let raw_path = line[3..].trim();
        let (previous_path, path) = if let Some((previous, next)) = raw_path.split_once(" -> ") {
            (
                Some(tracked_workspace_live_sync_unquote_path(previous)),
                tracked_workspace_live_sync_unquote_path(next),
            )
        } else {
            (None, tracked_workspace_live_sync_unquote_path(raw_path))
        };
        if path.is_empty() || tracked_workspace_live_sync_force_excluded_path(&path) {
            continue;
        }
        let kind = if status == "??" {
            TrackedWorkspaceLiveSyncFileChangeKind::Added
        } else if status.contains('R') {
            TrackedWorkspaceLiveSyncFileChangeKind::Renamed
        } else if status.contains('D') {
            TrackedWorkspaceLiveSyncFileChangeKind::Deleted
        } else {
            TrackedWorkspaceLiveSyncFileChangeKind::Modified
        };
        paths.insert(
            path.clone(),
            TrackedWorkspaceLiveSyncPathChange {
                path,
                previous_path,
                kind,
            },
        );
    }
    paths.into_values().collect()
}

fn tracked_workspace_live_sync_file_changes(
    before: &GitTurnSnapshot,
    path_changes: &[TrackedWorkspaceLiveSyncPathChange],
) -> Option<Vec<TrackedWorkspaceLiveSyncFileChange>> {
    let repo_root = PathBuf::from(&before.repo_root);
    let worktree_path = PathBuf::from(&before.worktree_path);
    let revision = before.head_sha.as_deref().unwrap_or("HEAD");
    Some(
        path_changes
            .iter()
            .map(|change| {
                let before_path = change.previous_path.as_deref().unwrap_or(&change.path);
                let before_snapshot = match change.kind {
                    TrackedWorkspaceLiveSyncFileChangeKind::Added => None,
                    TrackedWorkspaceLiveSyncFileChangeKind::Modified
                    | TrackedWorkspaceLiveSyncFileChangeKind::Deleted
                    | TrackedWorkspaceLiveSyncFileChangeKind::Renamed => {
                        tracked_workspace_live_sync_git_blob_snapshot(
                            &repo_root,
                            revision,
                            before_path,
                        )
                    }
                };
                let after_snapshot = match change.kind {
                    TrackedWorkspaceLiveSyncFileChangeKind::Deleted => None,
                    TrackedWorkspaceLiveSyncFileChangeKind::Added
                    | TrackedWorkspaceLiveSyncFileChangeKind::Modified
                    | TrackedWorkspaceLiveSyncFileChangeKind::Renamed => {
                        tracked_workspace_live_sync_worktree_snapshot(&worktree_path, &change.path)
                    }
                };
                let binary = before_snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.binary)
                    || after_snapshot
                        .as_ref()
                        .is_some_and(|snapshot| snapshot.binary);
                TrackedWorkspaceLiveSyncFileChange {
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
struct TrackedWorkspaceLiveSyncContentSnapshot {
    content_base64: String,
    binary: bool,
}

fn tracked_workspace_live_sync_git_blob_snapshot(
    repo_root: &Path,
    revision: &str,
    path: &str,
) -> Option<TrackedWorkspaceLiveSyncContentSnapshot> {
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
        .then(|| tracked_workspace_live_sync_content_snapshot(output.stdout))
}

fn tracked_workspace_live_sync_worktree_snapshot(
    worktree_path: &Path,
    path: &str,
) -> Option<TrackedWorkspaceLiveSyncContentSnapshot> {
    std::fs::read(worktree_path.join(path))
        .ok()
        .map(tracked_workspace_live_sync_content_snapshot)
}

fn tracked_workspace_live_sync_content_snapshot(
    bytes: Vec<u8>,
) -> TrackedWorkspaceLiveSyncContentSnapshot {
    TrackedWorkspaceLiveSyncContentSnapshot {
        binary: bytes.contains(&0),
        content_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
    }
}

fn tracked_workspace_live_sync_unquote_path(path: &str) -> String {
    path.trim().trim_matches('"').to_string()
}

pub(crate) fn apply_tracked_workspace_live_sync_change_to_target(
    change: &TrackedWorkspaceLiveSyncTurnChange,
    target_root: &Path,
) -> Vec<TrackedWorkspaceLiveSyncPathApplyResult> {
    change
        .file_changes
        .iter()
        .map(|file_change| {
            apply_tracked_workspace_live_sync_file_change_to_target(file_change, target_root)
        })
        .collect()
}

fn apply_tracked_workspace_live_sync_file_change_to_target(
    file_change: &TrackedWorkspaceLiveSyncFileChange,
    target_root: &Path,
) -> TrackedWorkspaceLiveSyncPathApplyResult {
    let path = file_change.path.clone();
    let target_path = match tracked_workspace_live_sync_target_path(target_root, &file_change.path)
    {
        Some(path) => path,
        None => {
            return TrackedWorkspaceLiveSyncPathApplyResult {
                path,
                status: TrackedWorkspaceLiveSyncApplyStatus::FailedIo,
                message: "workspace live sync path must be relative and cannot contain `..`"
                    .to_string(),
            };
        }
    };
    let previous_target_path = file_change
        .previous_path
        .as_deref()
        .and_then(|path| tracked_workspace_live_sync_target_path(target_root, path));
    let before_bytes = match tracked_workspace_live_sync_decode_optional(
        file_change.before_content_base64.as_deref(),
    ) {
        Ok(bytes) => bytes,
        Err(message) => {
            return TrackedWorkspaceLiveSyncPathApplyResult {
                path,
                status: TrackedWorkspaceLiveSyncApplyStatus::FailedIo,
                message,
            };
        }
    };
    let after_bytes = match tracked_workspace_live_sync_decode_optional(
        file_change.after_content_base64.as_deref(),
    ) {
        Ok(bytes) => bytes,
        Err(message) => {
            return TrackedWorkspaceLiveSyncPathApplyResult {
                path,
                status: TrackedWorkspaceLiveSyncApplyStatus::FailedIo,
                message,
            };
        }
    };
    match file_change.kind {
        TrackedWorkspaceLiveSyncFileChangeKind::Added => {
            tracked_workspace_live_sync_apply_add(&path, &target_path, after_bytes)
        }
        TrackedWorkspaceLiveSyncFileChangeKind::Modified => {
            tracked_workspace_live_sync_apply_modify(&path, &target_path, before_bytes, after_bytes)
        }
        TrackedWorkspaceLiveSyncFileChangeKind::Deleted => {
            tracked_workspace_live_sync_apply_delete(&path, &target_path, before_bytes)
        }
        TrackedWorkspaceLiveSyncFileChangeKind::Renamed => {
            tracked_workspace_live_sync_apply_rename(
                &path,
                previous_target_path.as_deref(),
                &target_path,
                before_bytes,
                after_bytes,
            )
        }
    }
}

fn tracked_workspace_live_sync_apply_add(
    path: &str,
    target_path: &Path,
    after_bytes: Option<Vec<u8>>,
) -> TrackedWorkspaceLiveSyncPathApplyResult {
    let Some(after_bytes) = after_bytes else {
        return tracked_workspace_live_sync_failed(path, "tracked add has no after content");
    };
    if target_path.exists() {
        return tracked_workspace_live_sync_conflict(path, "target path already exists");
    }
    tracked_workspace_live_sync_write_file(path, target_path, &after_bytes)
}

fn tracked_workspace_live_sync_apply_modify(
    path: &str,
    target_path: &Path,
    before_bytes: Option<Vec<u8>>,
    after_bytes: Option<Vec<u8>>,
) -> TrackedWorkspaceLiveSyncPathApplyResult {
    let (Some(before_bytes), Some(after_bytes)) = (before_bytes, after_bytes) else {
        return tracked_workspace_live_sync_failed(path, "tracked modify is missing content");
    };
    match std::fs::read(target_path) {
        Ok(current) if current == before_bytes => {
            tracked_workspace_live_sync_write_file(path, target_path, &after_bytes)
        }
        Ok(current) => tracked_workspace_live_sync_rebase_modify(
            path,
            target_path,
            &before_bytes,
            &after_bytes,
            &current,
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            tracked_workspace_live_sync_conflict(path, "target path is missing")
        }
        Err(error) => tracked_workspace_live_sync_failed(
            path,
            format!("failed to read target path before apply: {error}"),
        ),
    }
}

fn tracked_workspace_live_sync_rebase_modify(
    path: &str,
    target_path: &Path,
    before_bytes: &[u8],
    after_bytes: &[u8],
    current_bytes: &[u8],
) -> TrackedWorkspaceLiveSyncPathApplyResult {
    let Ok(before) = std::str::from_utf8(before_bytes) else {
        return tracked_workspace_live_sync_conflict(
            path,
            "binary target content changed before apply",
        );
    };
    let Ok(after) = std::str::from_utf8(after_bytes) else {
        return tracked_workspace_live_sync_conflict(
            path,
            "binary source content cannot be rebased",
        );
    };
    let Ok(current) = std::str::from_utf8(current_bytes) else {
        return tracked_workspace_live_sync_conflict(
            path,
            "binary target content cannot be rebased",
        );
    };
    let Some(rebased) = tracked_workspace_live_sync_rebase_text(before, after, current) else {
        return tracked_workspace_live_sync_conflict(
            path,
            "target content changed in an overlapping area before apply",
        );
    };
    if let Some(parent) = target_path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            return tracked_workspace_live_sync_failed(
                path,
                format!("failed to create target directory: {error}"),
            );
        }
    }
    match std::fs::write(target_path, rebased.as_bytes()) {
        Ok(()) => {
            tracked_workspace_live_sync_rebased(path, "rebased over non-overlapping target change")
        }
        Err(error) => tracked_workspace_live_sync_failed(
            path,
            format!("failed to write rebased target content: {error}"),
        ),
    }
}

fn tracked_workspace_live_sync_rebase_text(
    before: &str,
    after: &str,
    current: &str,
) -> Option<String> {
    let before_lines = tracked_workspace_live_sync_lines(before);
    let after_lines = tracked_workspace_live_sync_lines(after);
    let current_lines = tracked_workspace_live_sync_lines(current);
    let source = tracked_workspace_live_sync_changed_range(&before_lines, &after_lines);
    let target = tracked_workspace_live_sync_changed_range(&before_lines, &current_lines);
    if tracked_workspace_live_sync_ranges_overlap(
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
struct TrackedWorkspaceLiveSyncTextChangeRange {
    before_start: usize,
    before_end: usize,
    changed_start: usize,
    changed_end: usize,
}

fn tracked_workspace_live_sync_changed_range(
    before: &[String],
    changed: &[String],
) -> TrackedWorkspaceLiveSyncTextChangeRange {
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
    TrackedWorkspaceLiveSyncTextChangeRange {
        before_start: prefix,
        before_end: before.len() - suffix,
        changed_start: prefix,
        changed_end: changed.len() - suffix,
    }
}

fn tracked_workspace_live_sync_ranges_overlap(
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

fn tracked_workspace_live_sync_lines(value: &str) -> Vec<String> {
    if value.is_empty() {
        return Vec::new();
    }
    value.split_inclusive('\n').map(str::to_string).collect()
}

fn tracked_workspace_live_sync_apply_delete(
    path: &str,
    target_path: &Path,
    before_bytes: Option<Vec<u8>>,
) -> TrackedWorkspaceLiveSyncPathApplyResult {
    let Some(before_bytes) = before_bytes else {
        return tracked_workspace_live_sync_failed(path, "tracked delete has no before content");
    };
    match std::fs::read(target_path) {
        Ok(current) if current == before_bytes => match std::fs::remove_file(target_path) {
            Ok(()) => tracked_workspace_live_sync_applied(path, "deleted target path"),
            Err(error) => tracked_workspace_live_sync_failed(
                path,
                format!("failed to delete target path: {error}"),
            ),
        },
        Ok(_) => tracked_workspace_live_sync_conflict(path, "target content changed before delete"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            tracked_workspace_live_sync_conflict(path, "target path is already missing")
        }
        Err(error) => tracked_workspace_live_sync_failed(
            path,
            format!("failed to read target path before delete: {error}"),
        ),
    }
}

fn tracked_workspace_live_sync_apply_rename(
    path: &str,
    previous_target_path: Option<&Path>,
    target_path: &Path,
    before_bytes: Option<Vec<u8>>,
    after_bytes: Option<Vec<u8>>,
) -> TrackedWorkspaceLiveSyncPathApplyResult {
    let (Some(previous_target_path), Some(before_bytes), Some(after_bytes)) =
        (previous_target_path, before_bytes, after_bytes)
    else {
        return tracked_workspace_live_sync_failed(path, "tracked rename is missing content");
    };
    if target_path.exists() {
        return tracked_workspace_live_sync_conflict(path, "rename target path already exists");
    }
    match std::fs::read(previous_target_path) {
        Ok(current) if current == before_bytes => {}
        Ok(_) => {
            return tracked_workspace_live_sync_conflict(
                path,
                "rename source content changed before apply",
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return tracked_workspace_live_sync_conflict(path, "rename source path is missing");
        }
        Err(error) => {
            return tracked_workspace_live_sync_failed(
                path,
                format!("failed to read rename source before apply: {error}"),
            );
        }
    }
    if let Some(parent) = target_path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            return tracked_workspace_live_sync_failed(
                path,
                format!("failed to create target directory: {error}"),
            );
        }
    }
    if let Err(error) = std::fs::write(target_path, after_bytes) {
        return tracked_workspace_live_sync_failed(
            path,
            format!("failed to write target: {error}"),
        );
    }
    match std::fs::remove_file(previous_target_path) {
        Ok(()) => tracked_workspace_live_sync_applied(path, "renamed target path"),
        Err(error) => tracked_workspace_live_sync_failed(
            path,
            format!("failed to remove rename source after write: {error}"),
        ),
    }
}

fn tracked_workspace_live_sync_write_file(
    path: &str,
    target_path: &Path,
    bytes: &[u8],
) -> TrackedWorkspaceLiveSyncPathApplyResult {
    if let Some(parent) = target_path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            return tracked_workspace_live_sync_failed(
                path,
                format!("failed to create target directory: {error}"),
            );
        }
    }
    match std::fs::write(target_path, bytes) {
        Ok(()) => tracked_workspace_live_sync_applied(path, "applied target content"),
        Err(error) => tracked_workspace_live_sync_failed(
            path,
            format!("failed to write target content: {error}"),
        ),
    }
}

fn tracked_workspace_live_sync_decode_optional(
    value: Option<&str>,
) -> Result<Option<Vec<u8>>, String> {
    value
        .map(|value| {
            base64::engine::general_purpose::STANDARD
                .decode(value)
                .map_err(|error| format!("tracked content is not valid base64: {error}"))
        })
        .transpose()
}

fn tracked_workspace_live_sync_target_path(target_root: &Path, path: &str) -> Option<PathBuf> {
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

fn tracked_workspace_live_sync_applied(
    path: &str,
    message: impl Into<String>,
) -> TrackedWorkspaceLiveSyncPathApplyResult {
    TrackedWorkspaceLiveSyncPathApplyResult {
        path: path.to_string(),
        status: TrackedWorkspaceLiveSyncApplyStatus::Applied,
        message: message.into(),
    }
}

fn tracked_workspace_live_sync_rebased(
    path: &str,
    message: impl Into<String>,
) -> TrackedWorkspaceLiveSyncPathApplyResult {
    TrackedWorkspaceLiveSyncPathApplyResult {
        path: path.to_string(),
        status: TrackedWorkspaceLiveSyncApplyStatus::Rebased,
        message: message.into(),
    }
}

fn tracked_workspace_live_sync_conflict(
    path: &str,
    message: impl Into<String>,
) -> TrackedWorkspaceLiveSyncPathApplyResult {
    TrackedWorkspaceLiveSyncPathApplyResult {
        path: path.to_string(),
        status: TrackedWorkspaceLiveSyncApplyStatus::SkippedConflict,
        message: message.into(),
    }
}

fn tracked_workspace_live_sync_failed(
    path: &str,
    message: impl Into<String>,
) -> TrackedWorkspaceLiveSyncPathApplyResult {
    TrackedWorkspaceLiveSyncPathApplyResult {
        path: path.to_string(),
        status: TrackedWorkspaceLiveSyncApplyStatus::FailedIo,
        message: message.into(),
    }
}

fn tracked_workspace_live_sync_force_excluded_path(path: &str) -> bool {
    if path == ".arrobaignore"
        || path == ".git"
        || path.starts_with(".git/")
        || path == ".arroba"
        || path.starts_with(".arroba/")
        || path == ".env"
        || path.starts_with(".env.")
    {
        return true;
    }
    let forced_dirs = [
        "node_modules",
        "target",
        ".next",
        "dist",
        "build",
        ".venv",
        "venv",
        "__pycache__",
        ".pytest_cache",
    ];
    path.split('/')
        .any(|part| forced_dirs.iter().any(|excluded| part == *excluded))
}

pub(crate) fn append_observations(
    history: &OperationalHistoryStore,
    observations: Vec<RemoteGitObservation>,
) -> Result<Vec<HistoryEvent>, DaemonError> {
    let mut events = Vec::new();
    for observation in observations {
        let sequence = history.reserve_sequence();
        let mut event = HistoryEvent::operational(
            sequence,
            observation.kind,
            Some(HistoryEventRole::System),
            observation.content,
            observation.metadata,
            observation.context,
        );
        event.candidate_agent_ids = observation.candidate_agent_ids;
        event.candidate_prompt_ids = observation.candidate_prompt_ids;
        event.candidate_turn_ids = observation.candidate_turn_ids;
        event.attribution_confidence = observation.attribution_confidence;
        history.append(&event)?;
        events.push(event);
    }
    Ok(events)
}

fn git_observation(
    kind: HistoryEventKind,
    content: Option<String>,
    metadata: BTreeMap<String, serde_json::Value>,
    before: &GitTurnSnapshot,
    candidates: &GitAttributionCandidates,
) -> RemoteGitObservation {
    let context = HistoryEventTurnContext {
        session_id: Some(before.session_id.clone()),
        agent_id: Some(before.agent_id.clone()),
        provider: Some(before.provider.clone()),
        model: Some(before.model.clone()),
        turn_id: Some(before.turn_id.clone()),
        prompt_id: Some(before.prompt_id.clone()),
        provider_run_id: Some(before.provider_run_id.clone()),
        provider_session_id: before.provider_session_id.clone(),
        machine_id: before.machine_id.clone(),
        repo_root: Some(before.repo_root.clone()),
        worktree_path: Some(before.worktree_path.clone()),
        ..HistoryEventTurnContext::default()
    };
    RemoteGitObservation {
        kind,
        content,
        metadata,
        context,
        candidate_agent_ids: candidates.agent_ids.clone(),
        candidate_prompt_ids: candidates.prompt_ids.clone(),
        candidate_turn_ids: candidates.turn_ids.clone(),
        attribution_confidence: Some(candidates.confidence()),
    }
}

fn commits_between(before: &GitTurnSnapshot, after: &GitTurnSnapshot) -> Vec<GitCommitSummary> {
    let Some(after_head) = after.head_sha.as_deref() else {
        return Vec::new();
    };
    if before.head_sha.as_deref() == Some(after_head) {
        return Vec::new();
    }
    let rev_range = before
        .head_sha
        .as_ref()
        .map(|head| format!("{head}..{after_head}"))
        .unwrap_or_else(|| after_head.to_string());
    let Some(output) = git_output(
        Path::new(&after.worktree_path),
        &["rev-list", "--reverse", &rev_range],
    ) else {
        return Vec::new();
    };
    output
        .lines()
        .filter_map(|sha| commit_summary(Path::new(&after.worktree_path), sha.trim()))
        .collect()
}

fn commit_summary(worktree: &Path, sha: &str) -> Option<GitCommitSummary> {
    if sha.is_empty() {
        return None;
    }
    let details = git_output(
        worktree,
        &["show", "-s", "--format=%H%x00%s%x00%an%x00%ae%x00%ct", sha],
    )?;
    let mut parts = details.trim_end_matches('\n').split('\0');
    let full_sha = parts.next()?.to_string();
    let subject = parts.next().unwrap_or_default().to_string();
    let author_name = parts
        .next()
        .map(str::to_string)
        .filter(|value| !value.is_empty());
    let author_email = parts
        .next()
        .map(str::to_string)
        .filter(|value| !value.is_empty());
    let commit_timestamp_s = parts.next().and_then(|value| value.parse::<i64>().ok());
    let changed_paths = git_output(
        worktree,
        &["diff-tree", "--no-commit-id", "--name-only", "-r", sha],
    )
    .unwrap_or_default()
    .lines()
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .map(str::to_string)
    .collect::<Vec<_>>();
    Some(GitCommitSummary {
        sha: full_sha,
        subject,
        author_name,
        author_email,
        commit_timestamp_s,
        changed_paths,
    })
}

fn git_commit_content(commit: &GitCommitSummary) -> String {
    let mut lines = vec![format!("{} {}", commit.sha, commit.subject)];
    lines.extend(commit.changed_paths.iter().cloned());
    lines.join("\n")
}

fn git_commit_metadata(
    before: &GitTurnSnapshot,
    after: &GitTurnSnapshot,
    commit: &GitCommitSummary,
) -> BTreeMap<String, serde_json::Value> {
    let mut metadata = git_worktree_metadata(before, after);
    metadata.insert(
        "commit_sha".to_string(),
        serde_json::Value::String(commit.sha.clone()),
    );
    metadata.insert(
        "commit_subject".to_string(),
        serde_json::Value::String(commit.subject.clone()),
    );
    if let Some(author_name) = commit.author_name.clone() {
        metadata.insert(
            "commit_author_name".to_string(),
            serde_json::Value::String(author_name),
        );
    }
    if let Some(author_email) = commit.author_email.clone() {
        metadata.insert(
            "commit_author_email".to_string(),
            serde_json::Value::String(author_email),
        );
    }
    if let Some(timestamp) = commit.commit_timestamp_s {
        metadata.insert(
            "commit_timestamp_s".to_string(),
            serde_json::Value::Number(timestamp.into()),
        );
    }
    metadata.insert(
        "changed_paths".to_string(),
        serde_json::Value::Array(
            commit
                .changed_paths
                .iter()
                .cloned()
                .map(serde_json::Value::String)
                .collect(),
        ),
    );
    metadata
}

fn git_worktree_metadata(
    before: &GitTurnSnapshot,
    after: &GitTurnSnapshot,
) -> BTreeMap<String, serde_json::Value> {
    let mut metadata = BTreeMap::new();
    metadata.insert(
        "repo_root".to_string(),
        serde_json::Value::String(before.repo_root.clone()),
    );
    metadata.insert(
        "worktree_path".to_string(),
        serde_json::Value::String(before.worktree_path.clone()),
    );
    if let Some(branch) = before.branch.clone().or_else(|| after.branch.clone()) {
        metadata.insert("branch".to_string(), serde_json::Value::String(branch));
    }
    if let Some(before_head) = before.head_sha.clone() {
        metadata.insert(
            "before_head_sha".to_string(),
            serde_json::Value::String(before_head),
        );
    }
    if let Some(after_head) = after.head_sha.clone() {
        metadata.insert(
            "after_head_sha".to_string(),
            serde_json::Value::String(after_head),
        );
    }
    if let Some(upstream_ref) = before
        .upstream_ref
        .clone()
        .or_else(|| after.upstream_ref.clone())
    {
        metadata.insert(
            "upstream_ref".to_string(),
            serde_json::Value::String(upstream_ref),
        );
    }
    if let Some(ahead_count) = before.ahead_count {
        metadata.insert(
            "before_ahead_count".to_string(),
            serde_json::Value::Number(ahead_count.into()),
        );
    }
    if let Some(ahead_count) = after.ahead_count {
        metadata.insert(
            "after_ahead_count".to_string(),
            serde_json::Value::Number(ahead_count.into()),
        );
    }
    metadata.insert(
        "before_dirty".to_string(),
        serde_json::Value::Bool(before.is_dirty),
    );
    metadata.insert(
        "after_dirty".to_string(),
        serde_json::Value::Bool(after.is_dirty),
    );
    metadata.insert(
        "prompt_summary".to_string(),
        serde_json::Value::String(before.prompt_summary.clone()),
    );
    metadata
}

fn git_output(worktree: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(worktree)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

fn normalize_path(path: impl AsRef<Path>) -> String {
    path.as_ref()
        .canonicalize()
        .unwrap_or_else(|_| path.as_ref().to_path_buf())
        .display()
        .to_string()
}

fn truncate_for_metadata(value: &str, limit: usize) -> String {
    let mut output = String::new();
    for ch in value.chars().take(limit) {
        output.push(ch);
    }
    output
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use base64::Engine as _;

    use crate::history::{HistoryEventKind, HistoryEventQuery, OperationalHistoryStore};

    use super::{
        apply_tracked_workspace_live_sync_change_to_target, capture_turn_snapshot,
        observe_after_turn, tracked_workspace_live_sync_change_after_turn, GitTurnContext,
        GitTurnSnapshot, GitTurnSnapshotStore, TrackedWorkspaceLiveSyncApplyStatus,
        TrackedWorkspaceLiveSyncFileChange, TrackedWorkspaceLiveSyncFileChangeKind,
        TrackedWorkspaceLiveSyncTurnChange,
    };

    #[test]
    fn observes_commit_and_indexes_searchable_metadata() {
        let root = std::env::temp_dir().join(format!(
            "arroba-git-observer-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let history_path = root.join("history.db");
        std::fs::create_dir_all(&root).expect("temp repo should be created");
        run_git(&root, &["init"]);
        run_git(&root, &["config", "user.email", "agent@example.com"]);
        run_git(&root, &["config", "user.name", "Agent"]);
        std::fs::write(root.join("README.md"), "seed\n").expect("seed file should write");
        run_git(&root, &["add", "README.md"]);
        run_git(&root, &["commit", "-m", "seed commit"]);

        let before = capture_turn_snapshot(test_context(&root, "prompt-1"))
            .expect("git snapshot should be captured");
        std::fs::write(root.join("feature.txt"), "hello\n").expect("feature file should write");
        run_git(&root, &["add", "feature.txt"]);
        run_git(&root, &["commit", "-m", "add searchable feature"]);
        let after = capture_turn_snapshot(test_context(&root, "prompt-1"))
            .expect("git post snapshot should be captured");

        let snapshots = GitTurnSnapshotStore::default();
        snapshots.insert(before.clone());
        let candidates = snapshots.candidates_for(&before);
        let history =
            OperationalHistoryStore::open(history_path.clone()).expect("history store should open");
        let events = observe_after_turn(before, after, candidates, &history)
            .expect("observation should append");
        assert!(events
            .iter()
            .any(|event| event.kind == HistoryEventKind::GitCommitDetected));

        let subject_matches = history
            .query_events(HistoryEventQuery {
                provider: Some("dev-stub".to_string()),
                model: Some("dev-git".to_string()),
                text: Some("add searchable feature".to_string()),
                limit: Some(10),
                ..HistoryEventQuery::default()
            })
            .expect("subject query should work");
        assert_eq!(subject_matches.len(), 1);
        assert_eq!(subject_matches[0].prompt_id.as_deref(), Some("prompt-1"));
        let branch = subject_matches[0]
            .metadata
            .get("branch")
            .and_then(|value| value.as_str());
        assert!(matches!(branch, Some("master" | "main")));

        let path_matches = history
            .query_events(HistoryEventQuery {
                text: Some("feature.txt".to_string()),
                limit: Some(10),
                ..HistoryEventQuery::default()
            })
            .expect("path query should work");
        assert!(path_matches
            .iter()
            .any(|event| event.kind == HistoryEventKind::GitCommitDetected));

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(history_path.with_extension("db-wal"));
        let _ = std::fs::remove_file(history_path.with_extension("db-shm"));
    }

    fn test_context(root: &std::path::Path, prompt_id: &str) -> GitTurnContext {
        GitTurnContext {
            session_id: "session-1".to_string(),
            agent_id: "agent-1".to_string(),
            provider: "dev-stub".to_string(),
            model: "dev-git".to_string(),
            provider_run_id: "provider-run-1".to_string(),
            provider_session_id: Some("provider-session-1".to_string()),
            prompt_id: prompt_id.to_string(),
            turn_id: prompt_id.to_string(),
            worktree_path: root.to_path_buf(),
            workspace_live_sync_tracked: false,
            machine_id: None,
            prompt_summary: "make a searchable feature".to_string(),
        }
    }

    #[test]
    fn tracked_workspace_live_sync_change_records_clean_turn_paths() {
        let before = tracked_snapshot(false, "");
        let after = tracked_snapshot(true, " M src/lib.rs\n?? new.txt");

        let change = tracked_workspace_live_sync_change_after_turn(&before, &after)
            .expect("clean tracked turn should journal changed paths");

        assert_eq!(change.changed_paths, vec!["new.txt", "src/lib.rs"]);
        assert_eq!(change.status_fingerprint, " M src/lib.rs\n?? new.txt");
    }

    #[test]
    fn tracked_workspace_live_sync_change_skips_dirty_start() {
        let before = tracked_snapshot(true, " M src/lib.rs");
        let after = tracked_snapshot(true, " M src/lib.rs\n M src/other.rs");

        assert!(tracked_workspace_live_sync_change_after_turn(&before, &after).is_none());
    }

    #[test]
    fn tracked_workspace_live_sync_change_filters_forced_exclusions() {
        let before = tracked_snapshot(false, "");
        let after = tracked_snapshot(
            true,
            " M .env\n M .arroba/state.json\n M node_modules/pkg/index.js\n M src/lib.rs",
        );

        let change = tracked_workspace_live_sync_change_after_turn(&before, &after)
            .expect("allowed tracked path should remain");

        assert_eq!(change.changed_paths, vec!["src/lib.rs"]);
    }

    #[test]
    fn tracked_workspace_live_sync_change_captures_file_snapshots() {
        let root = std::env::temp_dir().join(format!(
            "arroba-tracked-sync-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(root.join("src")).expect("temp repo should be created");
        run_git(&root, &["init"]);
        run_git(&root, &["config", "user.email", "agent@example.com"]);
        run_git(&root, &["config", "user.name", "Agent"]);
        std::fs::write(root.join("src/lib.rs"), "pub fn old() {}\n")
            .expect("tracked file should write");
        std::fs::write(root.join("README.md"), "delete me\n").expect("delete file should write");
        run_git(&root, &["add", "."]);
        run_git(&root, &["commit", "-m", "seed commit"]);

        let before = capture_turn_snapshot(GitTurnContext {
            workspace_live_sync_tracked: true,
            ..test_context(&root, "prompt-1")
        })
        .expect("pre-turn snapshot should capture");
        std::fs::write(root.join("src/lib.rs"), "pub fn new() {}\n")
            .expect("tracked file should update");
        std::fs::write(root.join("src/new.rs"), "pub fn added() {}\n")
            .expect("new file should write");
        std::fs::remove_file(root.join("README.md")).expect("tracked file should delete");
        let after = capture_turn_snapshot(GitTurnContext {
            workspace_live_sync_tracked: true,
            ..test_context(&root, "prompt-1")
        })
        .expect("post-turn snapshot should capture");

        let change = tracked_workspace_live_sync_change_after_turn(&before, &after)
            .expect("tracked turn should produce file changes");

        assert_eq!(
            change.changed_paths,
            vec!["README.md", "src/lib.rs", "src/new.rs"]
        );
        assert_eq!(change.file_changes.len(), 3);
        let modified = change
            .file_changes
            .iter()
            .find(|change| change.path == "src/lib.rs")
            .expect("modified path should be present");
        let old_base64 = base64::engine::general_purpose::STANDARD.encode("pub fn old() {}\n");
        let new_base64 = base64::engine::general_purpose::STANDARD.encode("pub fn new() {}\n");
        assert_eq!(
            modified.before_content_base64.as_deref(),
            Some(old_base64.as_str())
        );
        assert_eq!(
            modified.after_content_base64.as_deref(),
            Some(new_base64.as_str())
        );
        let added = change
            .file_changes
            .iter()
            .find(|change| change.path == "src/new.rs")
            .expect("added path should be present");
        assert_eq!(added.before_content_base64, None);
        assert!(added.after_content_base64.is_some());
        let deleted = change
            .file_changes
            .iter()
            .find(|change| change.path == "README.md")
            .expect("deleted path should be present");
        assert!(deleted.before_content_base64.is_some());
        assert_eq!(deleted.after_content_base64, None);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn tracked_workspace_live_sync_apply_target_applies_exact_base_changes() {
        let source = std::env::temp_dir().join(format!(
            "arroba-tracked-sync-source-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let target = std::env::temp_dir().join(format!(
            "arroba-tracked-sync-target-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(source.join("src")).expect("source should be created");
        std::fs::create_dir_all(target.join("src")).expect("target should be created");
        run_git(&source, &["init"]);
        run_git(&source, &["config", "user.email", "agent@example.com"]);
        run_git(&source, &["config", "user.name", "Agent"]);
        std::fs::write(source.join("src/lib.rs"), "old\n").expect("source should write");
        std::fs::write(source.join("remove.txt"), "remove\n").expect("source should write");
        run_git(&source, &["add", "."]);
        run_git(&source, &["commit", "-m", "seed commit"]);
        std::fs::write(target.join("src/lib.rs"), "old\n").expect("target should write");
        std::fs::write(target.join("remove.txt"), "remove\n").expect("target should write");

        let before = capture_turn_snapshot(GitTurnContext {
            workspace_live_sync_tracked: true,
            ..test_context(&source, "prompt-1")
        })
        .expect("pre-turn snapshot should capture");
        std::fs::write(source.join("src/lib.rs"), "new\n").expect("source should update");
        std::fs::write(source.join("src/new.rs"), "added\n").expect("source should add");
        std::fs::remove_file(source.join("remove.txt")).expect("source should delete");
        let after = capture_turn_snapshot(GitTurnContext {
            workspace_live_sync_tracked: true,
            ..test_context(&source, "prompt-1")
        })
        .expect("post-turn snapshot should capture");
        let change = tracked_workspace_live_sync_change_after_turn(&before, &after)
            .expect("tracked turn should produce a change");

        let results = apply_tracked_workspace_live_sync_change_to_target(&change, &target);

        assert!(results
            .iter()
            .all(|result| result.status == TrackedWorkspaceLiveSyncApplyStatus::Applied));
        assert_eq!(
            std::fs::read_to_string(target.join("src/lib.rs")).expect("target should read"),
            "new\n"
        );
        assert_eq!(
            std::fs::read_to_string(target.join("src/new.rs")).expect("target should read"),
            "added\n"
        );
        assert!(!target.join("remove.txt").exists());

        let _ = std::fs::remove_dir_all(&source);
        let _ = std::fs::remove_dir_all(&target);
    }

    #[test]
    fn tracked_workspace_live_sync_apply_target_skips_conflicting_target() {
        let source = std::env::temp_dir().join(format!(
            "arroba-tracked-sync-conflict-source-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let target = std::env::temp_dir().join(format!(
            "arroba-tracked-sync-conflict-target-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(source.join("src")).expect("source should be created");
        std::fs::create_dir_all(target.join("src")).expect("target should be created");
        run_git(&source, &["init"]);
        run_git(&source, &["config", "user.email", "agent@example.com"]);
        run_git(&source, &["config", "user.name", "Agent"]);
        std::fs::write(source.join("src/lib.rs"), "old\n").expect("source should write");
        run_git(&source, &["add", "."]);
        run_git(&source, &["commit", "-m", "seed commit"]);
        std::fs::write(target.join("src/lib.rs"), "target local edit\n")
            .expect("target should write");

        let before = capture_turn_snapshot(GitTurnContext {
            workspace_live_sync_tracked: true,
            ..test_context(&source, "prompt-1")
        })
        .expect("pre-turn snapshot should capture");
        std::fs::write(source.join("src/lib.rs"), "new\n").expect("source should update");
        let after = capture_turn_snapshot(GitTurnContext {
            workspace_live_sync_tracked: true,
            ..test_context(&source, "prompt-1")
        })
        .expect("post-turn snapshot should capture");
        let change = tracked_workspace_live_sync_change_after_turn(&before, &after)
            .expect("tracked turn should produce a change");

        let results = apply_tracked_workspace_live_sync_change_to_target(&change, &target);

        assert_eq!(
            results[0].status,
            TrackedWorkspaceLiveSyncApplyStatus::SkippedConflict
        );
        assert_eq!(
            std::fs::read_to_string(target.join("src/lib.rs")).expect("target should read"),
            "target local edit\n"
        );

        let _ = std::fs::remove_dir_all(&source);
        let _ = std::fs::remove_dir_all(&target);
    }

    #[test]
    fn tracked_workspace_live_sync_apply_target_rebases_non_overlapping_text_changes() {
        let target = std::env::temp_dir().join(format!(
            "arroba-tracked-sync-rebase-target-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(target.join("src")).expect("target should be created");
        std::fs::write(target.join("src/lib.rs"), "a\nlocal\nb\nc\n").expect("target should write");
        let encode =
            |value: &str| base64::engine::general_purpose::STANDARD.encode(value.as_bytes());
        let change = TrackedWorkspaceLiveSyncTurnChange {
            session_id: "session-1".to_string(),
            agent_id: "agent-1".to_string(),
            provider_run_id: "provider-run-1".to_string(),
            prompt_id: "prompt-1".to_string(),
            repo_root: "/source".to_string(),
            worktree_path: "/source".to_string(),
            branch: Some("main".to_string()),
            changed_paths: vec!["src/lib.rs".to_string()],
            file_changes: vec![TrackedWorkspaceLiveSyncFileChange {
                path: "src/lib.rs".to_string(),
                previous_path: None,
                kind: TrackedWorkspaceLiveSyncFileChangeKind::Modified,
                before_content_base64: Some(encode("a\nb\nc\n")),
                after_content_base64: Some(encode("a\nb\nsource\nc\n")),
                binary: false,
            }],
            status_fingerprint: "fingerprint".to_string(),
        };

        let results = apply_tracked_workspace_live_sync_change_to_target(&change, &target);

        assert_eq!(
            results[0].status,
            TrackedWorkspaceLiveSyncApplyStatus::Rebased
        );
        assert_eq!(
            std::fs::read_to_string(target.join("src/lib.rs")).expect("target should read"),
            "a\nlocal\nb\nsource\nc\n"
        );

        let _ = std::fs::remove_dir_all(&target);
    }

    fn tracked_snapshot(is_dirty: bool, status_fingerprint: &str) -> GitTurnSnapshot {
        GitTurnSnapshot {
            session_id: "session-1".to_string(),
            agent_id: "agent-1".to_string(),
            provider: "dev-stub".to_string(),
            model: "dev-git".to_string(),
            provider_run_id: "provider-run-1".to_string(),
            provider_session_id: Some("provider-session-1".to_string()),
            prompt_id: "prompt-1".to_string(),
            turn_id: "prompt-1".to_string(),
            machine_id: None,
            prompt_summary: "make a searchable feature".to_string(),
            repo_root: "/tmp/repo".to_string(),
            worktree_path: "/tmp/repo".to_string(),
            branch: Some("main".to_string()),
            head_sha: Some("abc123".to_string()),
            upstream_ref: None,
            ahead_count: None,
            status_fingerprint: status_fingerprint.to_string(),
            is_dirty,
            workspace_live_sync_tracked: true,
        }
    }

    fn run_git(cwd: &std::path::Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("git should run");
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
