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
use crate::session::{PromptOrigin, PromptQueueItem};
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
    pub source_attachment_id: Option<String>,
    pub prompt_origin: Option<PromptOrigin>,
    pub external_provider: Option<String>,
    pub external_provider_session_id: Option<String>,
    pub external_provider_turn_id: Option<String>,
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
    pub source_attachment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_origin: Option<PromptOrigin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_provider_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_provider_turn_id: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_attachment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_origin: Option<PromptOrigin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_provider_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_provider_turn_id: Option<String>,
    pub completed_at_ms: u64,
    #[serde(default)]
    pub settlement_status: CompletedTurnSettlementStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub changed_paths: Vec<String>,
    #[serde(default)]
    pub undo_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo_unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletedTurnSettlementStatus {
    #[default]
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CompletedGitTurnSnapshotStore {
    inner: Arc<Mutex<BTreeMap<String, VecDeque<CompletedGitTurnSnapshot>>>>,
    settled_turns: Arc<Mutex<BTreeMap<String, CompletedGitTurnActionProjection>>>,
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

    pub(crate) fn record_prompt_settlement(
        &self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
        prompt: &PromptQueueItem,
        completed_at_ms: u64,
        started_at_ms: Option<u64>,
        settlement_status: CompletedTurnSettlementStatus,
    ) {
        let mut projection = CompletedGitTurnActionProjection {
            turn_id: prompt.id().to_string(),
            prompt_id: prompt.id().to_string(),
            provider_run_id: provider_run_id.to_string(),
            agent_id: agent_id.to_string(),
            source_attachment_id: Some(prompt.source_attachment_id().to_string()),
            prompt_origin: Some(prompt.prompt_origin()),
            external_provider: prompt.external_provider().map(str::to_string),
            external_provider_session_id: prompt.external_provider_session_id().map(str::to_string),
            external_provider_turn_id: prompt.external_provider_turn_id().map(str::to_string),
            completed_at_ms,
            settlement_status,
            duration_ms: started_at_ms
                .map(|started_at_ms| completed_at_ms.saturating_sub(started_at_ms)),
            changed_paths: Vec::new(),
            undo_available: false,
            undo_unavailable_reason: Some(
                "workspace change observation is not available for this turn".to_string(),
            ),
        };
        let key = completed_turn_agent_key(session_id, agent_id);
        let mut settled_turns = self
            .settled_turns
            .lock()
            .expect("settled prompt turn mutex poisoned");
        if settled_turns.get(&key).is_some_and(|existing| {
            existing.turn_id == projection.turn_id
                && existing.settlement_status == CompletedTurnSettlementStatus::Cancelled
        }) {
            projection.settlement_status = CompletedTurnSettlementStatus::Cancelled;
        }
        let replace = settled_turns
            .get(&key)
            .is_none_or(|existing| completed_turn_projection_is_newer(&projection, existing));
        if replace {
            settled_turns.insert(key, projection);
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
        let observed = self
            .latest_for_agent(session_id, agent_id)
            .map(|turn| turn.action_projection());
        let settled = self
            .settled_turns
            .lock()
            .expect("settled prompt turn mutex poisoned")
            .get(&completed_turn_agent_key(session_id, agent_id))
            .cloned();
        match (observed, settled) {
            (Some(observed), Some(settled)) if observed.turn_id == settled.turn_id => {
                Some(CompletedGitTurnActionProjection {
                    settlement_status: settled.settlement_status,
                    ..observed
                })
            }
            (Some(observed), Some(settled)) => {
                Some(if completed_turn_projection_is_newer(&settled, &observed) {
                    settled
                } else {
                    observed
                })
            }
            (Some(observed), None) => Some(observed),
            (None, Some(settled)) => Some(settled),
            (None, None) => None,
        }
    }
}

fn completed_turn_projection_is_newer(
    incoming: &CompletedGitTurnActionProjection,
    existing: &CompletedGitTurnActionProjection,
) -> bool {
    let incoming_started_at_ms = incoming
        .duration_ms
        .map(|duration_ms| incoming.completed_at_ms.saturating_sub(duration_ms))
        .unwrap_or(incoming.completed_at_ms);
    let existing_started_at_ms = existing
        .duration_ms
        .map(|duration_ms| existing.completed_at_ms.saturating_sub(duration_ms))
        .unwrap_or(existing.completed_at_ms);
    incoming_started_at_ms > existing_started_at_ms
        || (incoming_started_at_ms == existing_started_at_ms
            && incoming.completed_at_ms >= existing.completed_at_ms)
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
            source_attachment_id: self.before.source_attachment_id.clone(),
            prompt_origin: self.before.prompt_origin,
            external_provider: self.before.external_provider.clone(),
            external_provider_session_id: self.before.external_provider_session_id.clone(),
            external_provider_turn_id: self.before.external_provider_turn_id.clone(),
            completed_at_ms: self.completed_at_ms,
            settlement_status: CompletedTurnSettlementStatus::Completed,
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
        source_attachment_id: context.source_attachment_id,
        prompt_origin: context.prompt_origin,
        external_provider: context.external_provider,
        external_provider_session_id: context.external_provider_session_id,
        external_provider_turn_id: context.external_provider_turn_id,
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
mod tests;
