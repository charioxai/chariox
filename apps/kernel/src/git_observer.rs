use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use crate::error::DaemonError;
use crate::history::{
    HistoryAttributionConfidence, HistoryEvent, HistoryEventKind, HistoryEventRole,
    HistoryEventTurnContext, OperationalHistoryStore,
};
use crate::transport::relay_peer::RemoteGitObservation;
pub use crate::workspace_live_sync_journal::{
    WorkspaceLiveSyncApplyStatus, WorkspaceLiveSyncPathApplyResult, WorkspaceLiveSyncTargetResult,
};
pub(crate) use crate::workspace_live_sync_journal::{
    WorkspaceLiveSyncJournal, WorkspaceLiveSyncJournalEntry,
};

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
    pub started_at_ms: Option<u64>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<u64>,
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
    #[serde(default)]
    pub workspace_live_sync_file_snapshots: BTreeMap<String, WorkspaceLiveSyncTrackedFileSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct WorkspaceLiveSyncTrackedFileSnapshot {
    #[serde(default)]
    pub content_base64: Option<String>,
    #[serde(default)]
    pub binary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CompletedGitTurnSnapshot {
    pub before: GitTurnSnapshot,
    pub after: GitTurnSnapshot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<WorkspaceLiveSyncChange>,
    pub completed_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub undone: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletedGitTurnActionProjection {
    pub turn_id: String,
    pub prompt_id: String,
    pub provider_run_id: String,
    pub agent_id: String,
    pub completed_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub changed_paths: Vec<String>,
    #[serde(default)]
    pub undo_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo_unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CompletedGitTurnSnapshotStore {
    inner: Arc<Mutex<BTreeMap<String, VecDeque<CompletedGitTurnSnapshot>>>>,
}

impl CompletedGitTurnSnapshotStore {
    const MAX_TURNS_PER_AGENT: usize = 20;

    pub(crate) fn record(&self, snapshot: CompletedGitTurnSnapshot) {
        let key = completed_turn_agent_key(&snapshot.before.session_id, &snapshot.before.agent_id);
        let mut guard = self
            .inner
            .lock()
            .expect("completed git turn snapshot mutex poisoned");
        let turns = guard.entry(key).or_default();
        turns.retain(|existing| existing.before.turn_id != snapshot.before.turn_id);
        turns.push_back(snapshot);
        while turns.len() > Self::MAX_TURNS_PER_AGENT {
            turns.pop_front();
        }
    }

    pub(crate) fn latest_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Option<CompletedGitTurnSnapshot> {
        self.inner
            .lock()
            .expect("completed git turn snapshot mutex poisoned")
            .get(&completed_turn_agent_key(session_id, agent_id))
            .and_then(|turns| turns.back().cloned())
    }

    pub(crate) fn resolve(
        &self,
        session_id: &str,
        agent_id: &str,
        turn_ref: Option<&str>,
    ) -> Option<CompletedGitTurnSnapshot> {
        let guard = self
            .inner
            .lock()
            .expect("completed git turn snapshot mutex poisoned");
        let turns = guard.get(&completed_turn_agent_key(session_id, agent_id))?;
        match turn_ref.map(str::trim).filter(|value| !value.is_empty()) {
            Some(reference) => turns
                .iter()
                .rev()
                .find(|turn| {
                    turn.before.turn_id == reference
                        || turn.before.prompt_id == reference
                        || turn.before.turn_id.starts_with(reference)
                        || turn.before.prompt_id.starts_with(reference)
                })
                .cloned(),
            None => turns.back().cloned(),
        }
    }

    pub(crate) fn mark_undone(&self, session_id: &str, agent_id: &str, turn_id: &str) {
        let mut guard = self
            .inner
            .lock()
            .expect("completed git turn snapshot mutex poisoned");
        if let Some(turns) = guard.get_mut(&completed_turn_agent_key(session_id, agent_id)) {
            if let Some(turn) = turns.iter_mut().find(|turn| turn.before.turn_id == turn_id) {
                turn.undone = true;
            }
        }
    }

    pub(crate) fn latest_projection_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Option<CompletedGitTurnActionProjection> {
        self.latest_for_agent(session_id, agent_id)
            .map(|turn| turn.action_projection())
    }
}

impl CompletedGitTurnSnapshot {
    pub(crate) fn new(
        before: GitTurnSnapshot,
        after: GitTurnSnapshot,
        change: Option<WorkspaceLiveSyncChange>,
        completed_at_ms: u64,
    ) -> Self {
        let duration_ms = before
            .started_at_ms
            .map(|started_at_ms| completed_at_ms.saturating_sub(started_at_ms));
        Self {
            before,
            after,
            change,
            completed_at_ms,
            duration_ms,
            undone: false,
        }
    }

    pub(crate) fn action_projection(&self) -> CompletedGitTurnActionProjection {
        let undo_unavailable_reason = if self.undone {
            Some("turn already undone".to_string())
        } else {
            None
        };
        CompletedGitTurnActionProjection {
            turn_id: self.before.turn_id.clone(),
            prompt_id: self.before.prompt_id.clone(),
            provider_run_id: self.before.provider_run_id.clone(),
            agent_id: self.before.agent_id.clone(),
            completed_at_ms: self.completed_at_ms,
            duration_ms: self.duration_ms,
            changed_paths: self
                .change
                .as_ref()
                .map(|change| change.changed_paths.clone())
                .unwrap_or_default(),
            undo_available: undo_unavailable_reason.is_none(),
            undo_unavailable_reason,
        }
    }
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
        let mut guard = self.inner.lock().expect("git turn snapshot mutex poisoned");
        let provider_run_prefix = format!("{}:", snapshot.provider_run_id);
        guard.retain(|key, _| !key.starts_with(&provider_run_prefix));
        guard.insert(
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

    pub(crate) fn get(&self, provider_run_id: &str, prompt_id: &str) -> Option<GitTurnSnapshot> {
        self.inner
            .lock()
            .expect("git turn snapshot mutex poisoned")
            .get(&Self::key(provider_run_id, prompt_id))
            .cloned()
    }

    pub(crate) fn remove_for_provider_run(&self, provider_run_id: &str) -> Option<GitTurnSnapshot> {
        let mut guard = self.inner.lock().expect("git turn snapshot mutex poisoned");
        let key = guard
            .keys()
            .find(|key| key.starts_with(&format!("{provider_run_id}:")))
            .cloned()?;
        guard.remove(&key)
    }

    pub(crate) fn get_for_provider_run(&self, provider_run_id: &str) -> Option<GitTurnSnapshot> {
        let guard = self.inner.lock().expect("git turn snapshot mutex poisoned");
        let key = guard
            .keys()
            .find(|key| key.starts_with(&format!("{provider_run_id}:")))?;
        guard.get(key).cloned()
    }

    pub(crate) fn get_for_session_agent_prompt(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
    ) -> Option<GitTurnSnapshot> {
        self.inner
            .lock()
            .expect("git turn snapshot mutex poisoned")
            .values()
            .find(|snapshot| {
                snapshot.session_id == session_id
                    && snapshot.agent_id == agent_id
                    && snapshot.prompt_id == prompt_id
            })
            .cloned()
    }

    pub(crate) fn provider_run_ids_for_session(&self, session_id: &str) -> BTreeSet<String> {
        self.read()
            .values()
            .filter(|snapshot| snapshot.session_id == session_id)
            .map(|snapshot| snapshot.provider_run_id.clone())
            .collect()
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
pub struct WorkspaceLiveSyncChange {
    pub session_id: String,
    pub agent_id: String,
    pub provider_run_id: String,
    pub prompt_id: String,
    pub repo_root: String,
    pub worktree_path: String,
    pub branch: Option<String>,
    pub changed_paths: Vec<String>,
    pub file_changes: Vec<WorkspaceLiveSyncFileChange>,
    pub status_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceLiveSyncFileChange {
    pub path: String,
    #[serde(default)]
    pub previous_path: Option<String>,
    pub kind: WorkspaceLiveSyncFileChangeKind,
    #[serde(default)]
    pub before_content_base64: Option<String>,
    #[serde(default)]
    pub after_content_base64: Option<String>,
    pub binary: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceLiveSyncFileChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
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
    let workspace_live_sync_file_snapshots = workspace_live_sync_tracked::dirty_file_snapshots(
        &context.worktree_path,
        &status_fingerprint,
    );
    Some(GitTurnSnapshot {
        session_id: context.session_id,
        agent_id: context.agent_id,
        provider: context.provider,
        model: context.model,
        provider_run_id: context.provider_run_id,
        provider_session_id: context.provider_session_id,
        prompt_id: context.prompt_id,
        turn_id: context.turn_id,
        started_at_ms: context.started_at_ms,
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
        workspace_live_sync_file_snapshots,
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
) -> Option<WorkspaceLiveSyncChange> {
    workspace_live_sync_tracked::change_after_turn(before, after)
}

pub(crate) use workspace_live_sync_apply::apply_workspace_live_sync_change_to_target;
pub(crate) use workspace_live_sync_apply::apply_workspace_live_sync_undo_to_target;

mod workspace_live_sync_apply;
mod workspace_live_sync_tracked;

fn completed_turn_agent_key(session_id: &str, agent_id: &str) -> String {
    format!("{session_id}\n{agent_id}")
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

pub(crate) fn workspace_live_sync_git_branch(worktree: &Path) -> Option<String> {
    let branch = git_output(worktree, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let branch = branch.trim();
    if branch.is_empty() || branch == "HEAD" {
        None
    } else {
        Some(branch.to_string())
    }
}

pub(crate) fn workspace_live_sync_repo_fingerprint(worktree: &Path) -> Option<String> {
    git_output(worktree, &["config", "--get", "remote.origin.url"])
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            git_output(worktree, &["rev-parse", "--git-common-dir"]).map(|value| {
                let git_dir = value.trim();
                let path = Path::new(git_dir);
                let absolute = if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    worktree.join(path)
                };
                normalize_path(absolute)
            })
        })
}

pub(crate) fn workspace_live_sync_identity_conflict(
    target_root: &Path,
    expected_branch: Option<&str>,
    expected_repo_fingerprint: Option<&str>,
) -> Option<String> {
    if let Some(expected_branch) = expected_branch {
        match workspace_live_sync_git_branch(target_root) {
            Some(current_branch) if current_branch == expected_branch => {}
            Some(current_branch) => {
                return Some(format!(
                    "target branch changed from `{expected_branch}` to `{current_branch}`"
                ));
            }
            None => {
                return Some(format!(
                    "target branch `{expected_branch}` could not be verified"
                ));
            }
        }
    }
    if let Some(expected_repo_fingerprint) = expected_repo_fingerprint {
        match workspace_live_sync_repo_fingerprint(target_root) {
            Some(current_fingerprint) if current_fingerprint == expected_repo_fingerprint => {}
            Some(current_fingerprint) => {
                return Some(format!(
                    "target repo identity changed from `{expected_repo_fingerprint}` to `{current_fingerprint}`"
                ));
            }
            None => {
                return Some(format!(
                    "target repo identity `{expected_repo_fingerprint}` could not be verified"
                ));
            }
        }
    }
    None
}

pub(crate) fn git_output(worktree: &Path, args: &[&str]) -> Option<String> {
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
    use std::collections::BTreeMap;
    use std::process::Command;

    use base64::Engine as _;

    use crate::history::{HistoryEventKind, HistoryEventQuery, OperationalHistoryStore};

    use super::{
        apply_workspace_live_sync_change_to_target, capture_turn_snapshot, git_output,
        observe_after_turn, tracked_workspace_live_sync_change_after_turn,
        workspace_live_sync_git_branch, workspace_live_sync_identity_conflict,
        workspace_live_sync_repo_fingerprint, GitTurnContext, GitTurnSnapshot,
        GitTurnSnapshotStore, WorkspaceLiveSyncApplyStatus, WorkspaceLiveSyncChange,
        WorkspaceLiveSyncFileChange, WorkspaceLiveSyncFileChangeKind, WorkspaceLiveSyncJournal,
        WorkspaceLiveSyncJournalEntry, WorkspaceLiveSyncPathApplyResult,
        WorkspaceLiveSyncTargetResult,
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
            started_at_ms: None,
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
    fn tracked_workspace_live_sync_change_records_dirty_to_dirty_content_delta() {
        let root = std::env::temp_dir().join(format!(
            "arroba-tracked-sync-dirty-delta-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(root.join("outputs")).expect("temp repo should be created");
        run_git(&root, &["init"]);
        run_git(&root, &["config", "user.email", "agent@example.com"]);
        run_git(&root, &["config", "user.name", "Agent"]);
        std::fs::write(root.join("outputs/conflict.txt"), "one\ntarget\nthree\n")
            .expect("tracked file should write");
        run_git(&root, &["add", "."]);
        run_git(&root, &["commit", "-m", "seed commit"]);
        std::fs::write(root.join("outputs/conflict.txt"), "one\nsource\nthree\n")
            .expect("dirty file should write");

        let before = capture_turn_snapshot(GitTurnContext {
            workspace_live_sync_tracked: true,
            ..test_context(&root, "prompt-1")
        })
        .expect("dirty pre-turn snapshot should capture");
        std::fs::write(root.join("outputs/conflict.txt"), "one\nresolved\nthree\n")
            .expect("dirty file should update again");
        let after = capture_turn_snapshot(GitTurnContext {
            workspace_live_sync_tracked: true,
            ..test_context(&root, "prompt-1")
        })
        .expect("dirty post-turn snapshot should capture");

        let change = tracked_workspace_live_sync_change_after_turn(&before, &after)
            .expect("dirty tracked turn should produce a content delta");

        assert_eq!(change.changed_paths, vec!["outputs/conflict.txt"]);
        assert_eq!(change.file_changes.len(), 1);
        assert_eq!(
            change.file_changes[0].kind,
            WorkspaceLiveSyncFileChangeKind::Modified
        );
        let before_content = base64::engine::general_purpose::STANDARD
            .decode(
                change.file_changes[0]
                    .before_content_base64
                    .as_deref()
                    .expect("before content should be present"),
            )
            .expect("before content should decode");
        let after_content = base64::engine::general_purpose::STANDARD
            .decode(
                change.file_changes[0]
                    .after_content_base64
                    .as_deref()
                    .expect("after content should be present"),
            )
            .expect("after content should decode");
        assert_eq!(before_content, b"one\nsource\nthree\n");
        assert_eq!(after_content, b"one\nresolved\nthree\n");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn tracked_workspace_live_sync_change_filters_forced_exclusions() {
        let before = tracked_snapshot(false, "");
        let after = tracked_snapshot(
            true,
            " M .env\n M .envrc\n M config/.env.local\n M .arroba/state.json\n M .codex/session.json\n M .opencode/state.json\n M .claude/settings.json\n M .cursor/index.json\n M daemon.sock\n M daemon.socket\n M .tmp-arroba/socket\n M .tmp-live-workspace-live-sync-drill/state.json\n M .tmp-live-remote-workspace-live-sync-drill/state.json\n M history/session.jsonl\n M session-history/session.jsonl\n M operational-history/events.db\n M operational-history-1.db\n M node_modules/pkg/index.js\n M target/debug/app\n M .cache/tool/output.json\n M .turbo/cache.json\n M .next/cache/app\n M dist/app.js\n M build/app.js\n M .venv/pyvenv.cfg\n M venv/pyvenv.cfg\n M __pycache__/mod.pyc\n M .pytest_cache/v/cache/nodeids\n M .mypy_cache/module.json\n M .ruff_cache/module.json\n M .gradle/caches/module.bin\n M .m2/repository/artifact.jar\n M .pnpm-store/v3/files/index\n M src/lib.rs",
        );

        let change = tracked_workspace_live_sync_change_after_turn(&before, &after)
            .expect("allowed tracked path should remain");

        assert_eq!(change.changed_paths, vec!["src/lib.rs"]);
    }

    #[test]
    fn tracked_workspace_live_sync_change_filters_arrobaignore_patterns() {
        let root = std::env::temp_dir().join(format!(
            "arroba-tracked-sync-ignore-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(root.join("ignored")).expect("temp worktree should be created");
        std::fs::write(root.join(".gitignore"), "ignored/\n*.secret\n")
            .expect("gitignore should write");
        let mut before = tracked_snapshot(false, "");
        before.repo_root = root.display().to_string();
        before.worktree_path = root.display().to_string();
        let mut after =
            tracked_snapshot(true, " M ignored/file.txt\n M src/lib.rs\n?? token.secret");
        after.repo_root = root.display().to_string();
        after.worktree_path = root.display().to_string();

        let change = tracked_workspace_live_sync_change_after_turn(&before, &after)
            .expect("allowed tracked path should remain");

        assert_eq!(change.changed_paths, vec!["src/lib.rs"]);
        assert_eq!(
            std::fs::read_to_string(root.join(".arrobaignore"))
                .expect(".arrobaignore should initialize"),
            "ignored/\n*.secret\n"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn tracked_workspace_live_sync_change_initializes_empty_arrobaignore_without_gitignore() {
        let root = std::env::temp_dir().join(format!(
            "arroba-tracked-sync-empty-ignore-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&root).expect("temp worktree should be created");
        let mut before = tracked_snapshot(false, "");
        before.repo_root = root.display().to_string();
        before.worktree_path = root.display().to_string();
        let mut after = tracked_snapshot(true, " M src/lib.rs\n?? token.secret");
        after.repo_root = root.display().to_string();
        after.worktree_path = root.display().to_string();

        let change = tracked_workspace_live_sync_change_after_turn(&before, &after)
            .expect("tracked paths should remain when no ignore file exists");

        assert_eq!(change.changed_paths, vec!["src/lib.rs", "token.secret"]);
        assert_eq!(
            std::fs::read_to_string(root.join(".arrobaignore"))
                .expect(".arrobaignore should initialize"),
            ""
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn tracked_workspace_live_sync_change_filters_renames_from_ignored_paths() {
        let root = std::env::temp_dir().join(format!(
            "arroba-tracked-sync-ignore-rename-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&root).expect("temp worktree should be created");
        std::fs::write(root.join(".arrobaignore"), "ignored/\n").expect("ignore should write");
        let mut before = tracked_snapshot(false, "");
        before.repo_root = root.display().to_string();
        before.worktree_path = root.display().to_string();
        let mut after = tracked_snapshot(true, "R  ignored/old.txt -> src/new.txt\n M src/lib.rs");
        after.repo_root = root.display().to_string();
        after.worktree_path = root.display().to_string();

        let change = tracked_workspace_live_sync_change_after_turn(&before, &after)
            .expect("allowed tracked path should remain");

        assert_eq!(change.changed_paths, vec!["src/lib.rs"]);

        let _ = std::fs::remove_dir_all(&root);
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
    fn tracked_workspace_live_sync_change_skips_already_synced_status_lines() {
        let root = std::env::temp_dir().join(format!(
            "arroba-tracked-sync-delta-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(root.join("outputs")).expect("temp worktree should be created");
        let mut before = tracked_snapshot(false, " M tracked.txt\n?? outputs/existing.txt");
        before.repo_root = root.display().to_string();
        before.worktree_path = root.display().to_string();
        let mut after = tracked_snapshot(
            true,
            " M tracked.txt\n?? .arrobaignore\n?? outputs/existing.txt",
        );
        after.repo_root = root.display().to_string();
        after.worktree_path = root.display().to_string();

        assert!(tracked_workspace_live_sync_change_after_turn(&before, &after).is_none());

        let mut after_new = tracked_snapshot(
            true,
            " M tracked.txt\n?? outputs/existing.txt\n?? outputs/new.txt",
        );
        after_new.repo_root = root.display().to_string();
        after_new.worktree_path = root.display().to_string();

        let change = tracked_workspace_live_sync_change_after_turn(&before, &after_new)
            .expect("new status lines should still fan out");
        assert_eq!(change.changed_paths, vec!["outputs/new.txt"]);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn workspace_live_sync_apply_target_applies_exact_base_changes() {
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
        run_git(&target, &["init"]);
        run_git(&target, &["config", "user.email", "agent@example.com"]);
        run_git(&target, &["config", "user.name", "Agent"]);
        run_git(&target, &["add", "."]);
        run_git(&target, &["commit", "-m", "target seed"]);
        let target_head_before =
            git_output(&target, &["rev-parse", "HEAD"]).expect("target head should be readable");

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

        let results = apply_workspace_live_sync_change_to_target(&change, &target);

        assert!(results
            .iter()
            .all(|result| result.status == WorkspaceLiveSyncApplyStatus::Applied));
        assert_eq!(
            std::fs::read_to_string(target.join("src/lib.rs")).expect("target should read"),
            "new\n"
        );
        assert_eq!(
            std::fs::read_to_string(target.join("src/new.rs")).expect("target should read"),
            "added\n"
        );
        assert!(!target.join("remove.txt").exists());
        assert_eq!(
            git_output(&target, &["rev-parse", "HEAD"]).expect("target head should be readable"),
            target_head_before
        );
        assert!(git_output(&target, &["status", "--porcelain"])
            .expect("target status should be readable")
            .contains("src/lib.rs"));

        let _ = std::fs::remove_dir_all(&source);
        let _ = std::fs::remove_dir_all(&target);
    }

    #[test]
    fn workspace_live_sync_apply_target_skips_conflicting_target() {
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

        let results = apply_workspace_live_sync_change_to_target(&change, &target);

        assert_eq!(
            results[0].status,
            WorkspaceLiveSyncApplyStatus::SkippedConflict
        );
        assert_eq!(
            std::fs::read_to_string(target.join("src/lib.rs")).expect("target should read"),
            "target local edit\n"
        );

        let _ = std::fs::remove_dir_all(&source);
        let _ = std::fs::remove_dir_all(&target);
    }

    #[test]
    fn workspace_live_sync_apply_target_treats_already_applied_paths_as_applied() {
        let target = std::env::temp_dir().join(format!(
            "arroba-tracked-sync-idempotent-target-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(target.join("src")).expect("target should be created");
        std::fs::write(target.join("src/added.rs"), "added\n").expect("target should write");
        std::fs::write(target.join("src/lib.rs"), "new\n").expect("target should write");
        std::fs::write(target.join("src/new_name.rs"), "moved\n").expect("target should write");
        let encode =
            |value: &str| base64::engine::general_purpose::STANDARD.encode(value.as_bytes());
        let change = WorkspaceLiveSyncChange {
            session_id: "session-1".to_string(),
            agent_id: "agent-1".to_string(),
            provider_run_id: "provider-run-1".to_string(),
            prompt_id: "prompt-1".to_string(),
            repo_root: "/source".to_string(),
            worktree_path: "/source".to_string(),
            branch: Some("main".to_string()),
            changed_paths: vec![
                "src/added.rs".to_string(),
                "src/lib.rs".to_string(),
                "src/remove.rs".to_string(),
                "src/new_name.rs".to_string(),
            ],
            file_changes: vec![
                WorkspaceLiveSyncFileChange {
                    path: "src/added.rs".to_string(),
                    previous_path: None,
                    kind: WorkspaceLiveSyncFileChangeKind::Added,
                    before_content_base64: None,
                    after_content_base64: Some(encode("added\n")),
                    binary: false,
                },
                WorkspaceLiveSyncFileChange {
                    path: "src/lib.rs".to_string(),
                    previous_path: None,
                    kind: WorkspaceLiveSyncFileChangeKind::Modified,
                    before_content_base64: Some(encode("old\n")),
                    after_content_base64: Some(encode("new\n")),
                    binary: false,
                },
                WorkspaceLiveSyncFileChange {
                    path: "src/remove.rs".to_string(),
                    previous_path: None,
                    kind: WorkspaceLiveSyncFileChangeKind::Deleted,
                    before_content_base64: Some(encode("remove\n")),
                    after_content_base64: None,
                    binary: false,
                },
                WorkspaceLiveSyncFileChange {
                    path: "src/new_name.rs".to_string(),
                    previous_path: Some("src/old_name.rs".to_string()),
                    kind: WorkspaceLiveSyncFileChangeKind::Renamed,
                    before_content_base64: Some(encode("old\n")),
                    after_content_base64: Some(encode("moved\n")),
                    binary: false,
                },
            ],
            status_fingerprint: "fingerprint".to_string(),
        };

        let results = apply_workspace_live_sync_change_to_target(&change, &target);

        assert!(results
            .iter()
            .all(|result| result.status == WorkspaceLiveSyncApplyStatus::Applied));
        assert_eq!(
            std::fs::read_to_string(target.join("src/added.rs")).expect("target should read"),
            "added\n"
        );
        assert_eq!(
            std::fs::read_to_string(target.join("src/lib.rs")).expect("target should read"),
            "new\n"
        );
        assert!(!target.join("src/remove.rs").exists());
        assert_eq!(
            std::fs::read_to_string(target.join("src/new_name.rs")).expect("target should read"),
            "moved\n"
        );

        let _ = std::fs::remove_dir_all(&target);
    }

    #[test]
    fn workspace_live_sync_apply_target_continues_after_path_failure() {
        let target = std::env::temp_dir().join(format!(
            "arroba-tracked-sync-partial-failure-target-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&target).expect("target should be created");
        let encode =
            |value: &str| base64::engine::general_purpose::STANDARD.encode(value.as_bytes());
        let change = WorkspaceLiveSyncChange {
            session_id: "session-1".to_string(),
            agent_id: "agent-1".to_string(),
            provider_run_id: "provider-run-1".to_string(),
            prompt_id: "prompt-1".to_string(),
            repo_root: "/source".to_string(),
            worktree_path: "/source".to_string(),
            branch: Some("main".to_string()),
            changed_paths: vec!["../escape.txt".to_string(), "src/applied.rs".to_string()],
            file_changes: vec![
                WorkspaceLiveSyncFileChange {
                    path: "../escape.txt".to_string(),
                    previous_path: None,
                    kind: WorkspaceLiveSyncFileChangeKind::Added,
                    before_content_base64: None,
                    after_content_base64: Some(encode("blocked\n")),
                    binary: false,
                },
                WorkspaceLiveSyncFileChange {
                    path: "src/applied.rs".to_string(),
                    previous_path: None,
                    kind: WorkspaceLiveSyncFileChangeKind::Added,
                    before_content_base64: None,
                    after_content_base64: Some(encode("applied\n")),
                    binary: false,
                },
            ],
            status_fingerprint: "fingerprint".to_string(),
        };

        let results = apply_workspace_live_sync_change_to_target(&change, &target);

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].status, WorkspaceLiveSyncApplyStatus::FailedIo);
        assert!(results[0].message.contains("must be relative"));
        assert_eq!(results[1].status, WorkspaceLiveSyncApplyStatus::Applied);
        assert_eq!(
            std::fs::read_to_string(target.join("src/applied.rs")).expect("target should read"),
            "applied\n"
        );

        let _ = std::fs::remove_dir_all(&target);
    }

    #[test]
    fn workspace_live_sync_apply_target_skips_ignored_target_path() {
        let target = std::env::temp_dir().join(format!(
            "arroba-tracked-sync-ignore-target-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(target.join("ignored")).expect("target should be created");
        std::fs::write(target.join(".arrobaignore"), "ignored/\n").expect("ignore should write");
        let encode = |value: &str| base64::engine::general_purpose::STANDARD.encode(value);
        let change = WorkspaceLiveSyncChange {
            session_id: "session-1".to_string(),
            agent_id: "agent-1".to_string(),
            provider_run_id: "provider-run-1".to_string(),
            prompt_id: "prompt-1".to_string(),
            repo_root: "/source".to_string(),
            worktree_path: "/source".to_string(),
            branch: Some("main".to_string()),
            changed_paths: vec!["ignored/file.txt".to_string()],
            file_changes: vec![WorkspaceLiveSyncFileChange {
                path: "ignored/file.txt".to_string(),
                previous_path: None,
                kind: WorkspaceLiveSyncFileChangeKind::Added,
                before_content_base64: None,
                after_content_base64: Some(encode("secret\n")),
                binary: false,
            }],
            status_fingerprint: "fingerprint".to_string(),
        };

        let results = apply_workspace_live_sync_change_to_target(&change, &target);

        assert_eq!(
            results[0].status,
            WorkspaceLiveSyncApplyStatus::SkippedConflict
        );
        assert!(results[0].message.contains("ignored"));
        assert!(!target.join("ignored/file.txt").exists());

        let _ = std::fs::remove_dir_all(&target);
    }

    #[test]
    fn workspace_live_sync_apply_target_skips_forced_excluded_path() {
        let target = std::env::temp_dir().join(format!(
            "arroba-tracked-sync-force-exclude-target-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&target).expect("target should be created");
        let encode = |value: &str| base64::engine::general_purpose::STANDARD.encode(value);
        let change = WorkspaceLiveSyncChange {
            session_id: "session-1".to_string(),
            agent_id: "agent-1".to_string(),
            provider_run_id: "provider-run-1".to_string(),
            prompt_id: "prompt-1".to_string(),
            repo_root: "/source".to_string(),
            worktree_path: "/source".to_string(),
            branch: Some("main".to_string()),
            changed_paths: vec![".env.local".to_string()],
            file_changes: vec![WorkspaceLiveSyncFileChange {
                path: ".env.local".to_string(),
                previous_path: None,
                kind: WorkspaceLiveSyncFileChangeKind::Added,
                before_content_base64: None,
                after_content_base64: Some(encode("TOKEN=secret\n")),
                binary: false,
            }],
            status_fingerprint: "fingerprint".to_string(),
        };

        let results = apply_workspace_live_sync_change_to_target(&change, &target);

        assert_eq!(
            results[0].status,
            WorkspaceLiveSyncApplyStatus::SkippedConflict
        );
        assert!(!target.join(".env.local").exists());

        let _ = std::fs::remove_dir_all(&target);
    }

    #[test]
    fn workspace_live_sync_apply_target_conflicts_on_binary_mismatch() {
        let target = std::env::temp_dir().join(format!(
            "arroba-tracked-sync-binary-conflict-target-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&target).expect("target should be created");
        std::fs::write(target.join("image.bin"), [0xff, 1, 9, 3]).expect("target should write");
        let encode = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
        let change = WorkspaceLiveSyncChange {
            session_id: "session-1".to_string(),
            agent_id: "agent-1".to_string(),
            provider_run_id: "provider-run-1".to_string(),
            prompt_id: "prompt-1".to_string(),
            repo_root: "/source".to_string(),
            worktree_path: "/source".to_string(),
            branch: Some("main".to_string()),
            changed_paths: vec!["image.bin".to_string()],
            file_changes: vec![WorkspaceLiveSyncFileChange {
                path: "image.bin".to_string(),
                previous_path: None,
                kind: WorkspaceLiveSyncFileChangeKind::Modified,
                before_content_base64: Some(encode(&[0xff, 1, 2, 3])),
                after_content_base64: Some(encode(&[0xff, 1, 2, 4])),
                binary: true,
            }],
            status_fingerprint: "fingerprint".to_string(),
        };

        let results = apply_workspace_live_sync_change_to_target(&change, &target);

        assert_eq!(
            results[0].status,
            WorkspaceLiveSyncApplyStatus::SkippedConflict
        );
        assert!(results[0].message.contains("binary"));
        assert_eq!(
            std::fs::read(target.join("image.bin")).expect("target should read"),
            vec![0xff, 1, 9, 3]
        );

        let _ = std::fs::remove_dir_all(&target);
    }

    #[test]
    fn workspace_live_sync_apply_target_conflicts_on_incompatible_rename() {
        let target = std::env::temp_dir().join(format!(
            "arroba-tracked-sync-rename-conflict-target-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&target).expect("target should be created");
        std::fs::write(target.join("old.txt"), "old\n").expect("old target should write");
        std::fs::write(target.join("new.txt"), "already here\n").expect("new target should write");
        let encode =
            |value: &str| base64::engine::general_purpose::STANDARD.encode(value.as_bytes());
        let change = WorkspaceLiveSyncChange {
            session_id: "session-1".to_string(),
            agent_id: "agent-1".to_string(),
            provider_run_id: "provider-run-1".to_string(),
            prompt_id: "prompt-1".to_string(),
            repo_root: "/source".to_string(),
            worktree_path: "/source".to_string(),
            branch: Some("main".to_string()),
            changed_paths: vec!["new.txt".to_string()],
            file_changes: vec![WorkspaceLiveSyncFileChange {
                path: "new.txt".to_string(),
                previous_path: Some("old.txt".to_string()),
                kind: WorkspaceLiveSyncFileChangeKind::Renamed,
                before_content_base64: Some(encode("old\n")),
                after_content_base64: Some(encode("moved\n")),
                binary: false,
            }],
            status_fingerprint: "fingerprint".to_string(),
        };

        let results = apply_workspace_live_sync_change_to_target(&change, &target);

        assert_eq!(
            results[0].status,
            WorkspaceLiveSyncApplyStatus::SkippedConflict
        );
        assert!(results[0].message.contains("already exists"));
        assert_eq!(
            std::fs::read_to_string(target.join("old.txt")).expect("old target should read"),
            "old\n"
        );
        assert_eq!(
            std::fs::read_to_string(target.join("new.txt")).expect("new target should read"),
            "already here\n"
        );

        let _ = std::fs::remove_dir_all(&target);
    }

    #[test]
    fn workspace_live_sync_identity_conflict_detects_branch_drift() {
        let target = std::env::temp_dir().join(format!(
            "arroba-tracked-sync-identity-target-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&target).expect("target should be created");
        run_git(&target, &["init"]);
        run_git(&target, &["config", "user.email", "agent@example.com"]);
        run_git(&target, &["config", "user.name", "Agent"]);
        std::fs::write(target.join("README.md"), "seed\n").expect("target should write");
        run_git(&target, &["add", "."]);
        run_git(&target, &["commit", "-m", "seed"]);
        run_git(&target, &["checkout", "-b", "sync-main"]);
        let fingerprint =
            workspace_live_sync_repo_fingerprint(&target).expect("fingerprint should resolve");

        assert_eq!(
            workspace_live_sync_git_branch(&target).as_deref(),
            Some("sync-main")
        );
        assert!(workspace_live_sync_identity_conflict(
            &target,
            Some("sync-main"),
            Some(fingerprint.as_str()),
        )
        .is_none());

        run_git(&target, &["checkout", "-b", "other"]);

        let conflict = workspace_live_sync_identity_conflict(
            &target,
            Some("sync-main"),
            Some(fingerprint.as_str()),
        )
        .expect("branch drift should conflict");
        assert!(conflict.contains("target branch changed"));

        let _ = std::fs::remove_dir_all(&target);
    }

    #[test]
    fn workspace_live_sync_apply_target_rebases_non_overlapping_text_changes() {
        let target = std::env::temp_dir().join(format!(
            "arroba-tracked-sync-rebase-target-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(target.join("src")).expect("target should be created");
        std::fs::write(target.join("src/lib.rs"), "a\nlocal\nb\nc\n").expect("target should write");
        let encode =
            |value: &str| base64::engine::general_purpose::STANDARD.encode(value.as_bytes());
        let change = WorkspaceLiveSyncChange {
            session_id: "session-1".to_string(),
            agent_id: "agent-1".to_string(),
            provider_run_id: "provider-run-1".to_string(),
            prompt_id: "prompt-1".to_string(),
            repo_root: "/source".to_string(),
            worktree_path: "/source".to_string(),
            branch: Some("main".to_string()),
            changed_paths: vec!["src/lib.rs".to_string()],
            file_changes: vec![WorkspaceLiveSyncFileChange {
                path: "src/lib.rs".to_string(),
                previous_path: None,
                kind: WorkspaceLiveSyncFileChangeKind::Modified,
                before_content_base64: Some(encode("a\nb\nc\n")),
                after_content_base64: Some(encode("a\nb\nsource\nc\n")),
                binary: false,
            }],
            status_fingerprint: "fingerprint".to_string(),
        };

        let results = apply_workspace_live_sync_change_to_target(&change, &target);

        assert_eq!(results[0].status, WorkspaceLiveSyncApplyStatus::Rebased);
        assert_eq!(
            std::fs::read_to_string(target.join("src/lib.rs")).expect("target should read"),
            "a\nlocal\nb\nsource\nc\n"
        );

        let _ = std::fs::remove_dir_all(&target);
    }

    #[test]
    fn workspace_live_sync_journal_assigns_ordered_sequences_per_link() {
        let journal = WorkspaceLiveSyncJournal::default();
        let change = || workspace_live_sync_test_change("session-1");

        let first = journal.append_for_link("link-a", "shared-a", change());
        let second = journal.append_for_link("link-a", "shared-a", change());
        let other_link = journal.append_for_link("link-b", "shared-b", change());

        assert_eq!(first.sequence, 1);
        assert_eq!(second.sequence, 2);
        assert_eq!(other_link.sequence, 1);
        let entries = journal.entries_for_session("session-1");
        assert_eq!(entries.len(), 3);
        assert_eq!(
            entries
                .iter()
                .map(|entry| (entry.link_id.as_str(), entry.sequence))
                .collect::<Vec<_>>(),
            vec![("link-a", 1), ("link-a", 2), ("link-b", 1)]
        );
    }

    #[test]
    fn workspace_live_sync_journal_restores_durable_events_and_next_sequence() {
        let path = std::env::temp_dir().join(format!(
            "arroba-workspace-live-sync-journal-{}-{}.db",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
        let store =
            crate::durable_state::DurableKernelStateStore::open(path.clone()).expect("open store");

        let entry = WorkspaceLiveSyncJournalEntry {
            sequence: 1,
            link_id: "link-a".to_string(),
            link_name: "shared-a".to_string(),
            change: workspace_live_sync_test_change("session-1"),
        };
        let target_result = WorkspaceLiveSyncTargetResult {
            session_id: "session-1".to_string(),
            link_id: "link-a".to_string(),
            link_name: "shared-a".to_string(),
            source_agent_id: "agent-1".to_string(),
            source_worktree_path: "/repo".to_string(),
            target_user_id: "user-2".to_string(),
            target_machine_id: "machine-2".to_string(),
            target_kernel_id: "kernel-2".to_string(),
            target_repo_root: "/target".to_string(),
            path_results: vec![WorkspaceLiveSyncPathApplyResult {
                path: "src/lib.rs".to_string(),
                status: WorkspaceLiveSyncApplyStatus::Applied,
                message: "applied cleanly".to_string(),
            }],
        };
        store
            .append_event(
                "workspace_live_sync.change_recorded",
                Some("session-1".to_string()),
                serde_json::json!({ "entry": entry }),
            )
            .expect("change event should append");
        store
            .append_event(
                "workspace_live_sync.target_results_recorded",
                Some("session-1".to_string()),
                serde_json::json!({ "target_results": [target_result] }),
            )
            .expect("target result event should append");

        let journal =
            WorkspaceLiveSyncJournal::restore_from_durable_state(&store).expect("restore journal");

        assert_eq!(journal.entries_for_session("session-1").len(), 1);
        assert_eq!(journal.target_results_for_session("session-1").len(), 1);
        let next = journal.append_for_link(
            "link-a",
            "shared-a",
            workspace_live_sync_test_change("session-1"),
        );
        assert_eq!(next.sequence, 2);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn pending_turn_snapshot_lookup_does_not_consume_snapshot() {
        let snapshots = GitTurnSnapshotStore::default();
        let snapshot = tracked_snapshot(false, "");
        snapshots.insert(snapshot.clone());

        assert_eq!(
            snapshots.get("provider-run-1", "prompt-1"),
            Some(snapshot.clone())
        );
        assert_eq!(
            snapshots.get_for_provider_run("provider-run-1"),
            Some(snapshot.clone())
        );
        assert_eq!(
            snapshots.remove("provider-run-1", "prompt-1"),
            Some(snapshot)
        );
        assert_eq!(snapshots.get_for_provider_run("provider-run-1"), None);
    }

    fn workspace_live_sync_test_change(session_id: &str) -> WorkspaceLiveSyncChange {
        WorkspaceLiveSyncChange {
            session_id: session_id.to_string(),
            agent_id: "agent-1".to_string(),
            provider_run_id: "provider-run-1".to_string(),
            prompt_id: "prompt-1".to_string(),
            repo_root: "/repo".to_string(),
            worktree_path: "/repo".to_string(),
            branch: Some("main".to_string()),
            changed_paths: vec!["src/lib.rs".to_string()],
            file_changes: Vec::new(),
            status_fingerprint: "managed_workspace_live_sync".to_string(),
        }
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
            started_at_ms: None,
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
            workspace_live_sync_file_snapshots: BTreeMap::new(),
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
