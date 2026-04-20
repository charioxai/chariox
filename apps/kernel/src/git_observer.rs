use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use crate::error::DaemonError;
use crate::history::{
    HistoryAttributionConfidence, HistoryEvent, HistoryEventKind, HistoryEventRole,
    HistoryEventTurnContext, OperationalHistoryStore,
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
    pub worktree_path: PathBuf,
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
    })
}

pub(crate) fn observe_after_turn(
    before: GitTurnSnapshot,
    after: GitTurnSnapshot,
    candidates: GitAttributionCandidates,
    history: &OperationalHistoryStore,
) -> Result<Vec<HistoryEvent>, DaemonError> {
    let mut events = Vec::new();
    for commit in commits_between(&before, &after) {
        let mut event = append_git_event(
            history,
            HistoryEventKind::GitCommitDetected,
            Some(git_commit_content(&commit)),
            git_commit_metadata(&before, &after, &commit),
            &before,
            &candidates,
        )?;
        event.caused_by_event_id = None;
        events.push(event);
    }

    if before.status_fingerprint != after.status_fingerprint {
        events.push(append_git_event(
            history,
            HistoryEventKind::GitWorktreeChanged,
            Some("Git worktree changed during agent turn.".to_string()),
            git_worktree_metadata(&before, &after),
            &before,
            &candidates,
        )?);
    }
    if !before.is_dirty && after.is_dirty {
        events.push(append_git_event(
            history,
            HistoryEventKind::GitWorktreeDirty,
            Some("Git worktree became dirty during agent turn.".to_string()),
            git_worktree_metadata(&before, &after),
            &before,
            &candidates,
        )?);
    } else if before.is_dirty && !after.is_dirty {
        events.push(append_git_event(
            history,
            HistoryEventKind::GitWorktreeClean,
            Some("Git worktree became clean during agent turn.".to_string()),
            git_worktree_metadata(&before, &after),
            &before,
            &candidates,
        )?);
    }
    if before.ahead_count.unwrap_or_default() > 0
        && after.ahead_count == Some(0)
        && before.upstream_ref == after.upstream_ref
    {
        events.push(append_git_event(
            history,
            HistoryEventKind::GitPushDetected,
            Some("Git push detected during agent turn.".to_string()),
            git_worktree_metadata(&before, &after),
            &before,
            &candidates,
        )?);
    }
    Ok(events)
}

fn append_git_event(
    history: &OperationalHistoryStore,
    kind: HistoryEventKind,
    content: Option<String>,
    metadata: BTreeMap<String, serde_json::Value>,
    before: &GitTurnSnapshot,
    candidates: &GitAttributionCandidates,
) -> Result<HistoryEvent, DaemonError> {
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
    let sequence = history.reserve_sequence();
    let mut event = HistoryEvent::operational(
        sequence,
        kind,
        Some(HistoryEventRole::System),
        content,
        metadata,
        context,
    );
    event.candidate_agent_ids = candidates.agent_ids.clone();
    event.candidate_prompt_ids = candidates.prompt_ids.clone();
    event.candidate_turn_ids = candidates.turn_ids.clone();
    event.attribution_confidence = Some(candidates.confidence());
    history.append(&event)?;
    Ok(event)
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

    use crate::history::{HistoryEventKind, HistoryEventQuery, OperationalHistoryStore};

    use super::{capture_turn_snapshot, observe_after_turn, GitTurnContext, GitTurnSnapshotStore};

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
            machine_id: None,
            prompt_summary: "make a searchable feature".to_string(),
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
