use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use serde::{Deserialize, Serialize};

use crate::io::WorkspaceIdentity;

#[derive(Clone, Debug, Default)]
pub(crate) struct WorkspaceIdentityMonitor {
    state: Arc<StdMutex<WorkspaceIdentityMonitorState>>,
}

#[derive(Debug, Default)]
struct WorkspaceIdentityMonitorState {
    provider_runs: BTreeMap<String, ProviderWorkspaceRecord>,
}

#[derive(Debug, Clone)]
struct ProviderWorkspaceRecord {
    root: PathBuf,
    baseline_identity: WorkspaceIdentity,
    current_identity: WorkspaceIdentity,
    generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceIdentitySnapshot {
    pub root: PathBuf,
    pub baseline_identity: WorkspaceIdentity,
    pub current_identity: WorkspaceIdentity,
    pub generation: u64,
    pub identity_changed: bool,
    pub valid: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceIdentityMonitorHealthSnapshot {
    pub tracked_provider_runs: usize,
    pub identity_changed_provider_runs: usize,
    pub invalid_provider_runs: usize,
    pub current_generation_total: u64,
    pub issues: Vec<WorkspaceIdentityIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceIdentityIssue {
    pub provider_run_id: String,
    pub root: String,
    pub generation: u64,
    pub valid: bool,
    pub baseline_fingerprint: String,
    pub current_fingerprint: String,
    pub baseline_branch: Option<String>,
    pub current_branch: Option<String>,
    pub baseline_head_commit: Option<String>,
    pub current_head_commit: Option<String>,
    pub baseline_repo_url: Option<String>,
    pub current_repo_url: Option<String>,
}

impl WorkspaceIdentityMonitor {
    pub(crate) fn observe_provider_run(
        &self,
        provider_run_id: impl Into<String>,
        root: PathBuf,
        current_identity: WorkspaceIdentity,
    ) -> WorkspaceIdentitySnapshot {
        let provider_run_id = provider_run_id.into();
        let mut state = self
            .state
            .lock()
            .expect("workspace identity monitor lock should not be poisoned");
        let record = state
            .provider_runs
            .entry(provider_run_id)
            .or_insert_with(|| ProviderWorkspaceRecord {
                root: root.clone(),
                baseline_identity: current_identity.clone(),
                current_identity: current_identity.clone(),
                generation: 0,
            });
        if record.root != root || record.current_identity != current_identity {
            record.root = root.clone();
            record.current_identity = current_identity.clone();
            record.generation += 1;
        }
        WorkspaceIdentitySnapshot {
            root: root.clone(),
            baseline_identity: record.baseline_identity.clone(),
            current_identity,
            generation: record.generation,
            identity_changed: record.generation > 0,
            valid: record.root == root && record.current_identity == record.baseline_identity,
        }
    }

    pub(crate) fn remove_provider_run(&self, provider_run_id: &str) {
        self.state
            .lock()
            .expect("workspace identity monitor lock should not be poisoned")
            .provider_runs
            .remove(provider_run_id);
    }

    pub(crate) fn health_snapshot(&self) -> WorkspaceIdentityMonitorHealthSnapshot {
        let state = self
            .state
            .lock()
            .expect("workspace identity monitor lock should not be poisoned");
        let mut identity_changed_provider_runs = 0usize;
        let mut invalid_provider_runs = 0usize;
        let mut current_generation_total = 0u64;
        let mut issues = Vec::new();
        for (provider_run_id, record) in &state.provider_runs {
            current_generation_total += record.generation;
            if record.generation > 0 {
                identity_changed_provider_runs += 1;
            }
            let valid = record.current_identity == record.baseline_identity;
            if !valid {
                invalid_provider_runs += 1;
            }
            if record.generation > 0 || !valid {
                issues.push(WorkspaceIdentityIssue {
                    provider_run_id: provider_run_id.clone(),
                    root: record.root.to_string_lossy().to_string(),
                    generation: record.generation,
                    valid,
                    baseline_fingerprint: record
                        .baseline_identity
                        .worktree_root_fingerprint
                        .clone(),
                    current_fingerprint: record.current_identity.worktree_root_fingerprint.clone(),
                    baseline_branch: record.baseline_identity.branch.clone(),
                    current_branch: record.current_identity.branch.clone(),
                    baseline_head_commit: record.baseline_identity.head_commit.clone(),
                    current_head_commit: record.current_identity.head_commit.clone(),
                    baseline_repo_url: record.baseline_identity.repo_url.clone(),
                    current_repo_url: record.current_identity.repo_url.clone(),
                });
            }
        }
        WorkspaceIdentityMonitorHealthSnapshot {
            tracked_provider_runs: state.provider_runs.len(),
            identity_changed_provider_runs,
            invalid_provider_runs,
            current_generation_total,
            issues,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::WorkspaceIdentityMonitor;

    #[test]
    fn provider_workspace_starts_valid() {
        let monitor = WorkspaceIdentityMonitor::default();
        let identity = crate::io::WorkspaceIdentity::local("root-a");

        let snapshot = monitor.observe_provider_run("run-1", "/repo".into(), identity.clone());

        assert!(snapshot.valid);
        assert!(!snapshot.identity_changed);
        assert_eq!(snapshot.current_identity, identity);
        assert_eq!(snapshot.generation, 0);
    }

    #[test]
    fn provider_workspace_invalidates_after_identity_change() {
        let monitor = WorkspaceIdentityMonitor::default();
        let first = crate::io::WorkspaceIdentity::local("root-a");
        let second = crate::io::WorkspaceIdentity::local("root-b");

        monitor.observe_provider_run("run-1", "/repo".into(), first.clone());
        let snapshot = monitor.observe_provider_run("run-1", "/repo".into(), second.clone());

        assert!(!snapshot.valid);
        assert!(snapshot.identity_changed);
        assert_eq!(snapshot.baseline_identity, first);
        assert_eq!(snapshot.current_identity, second);
        assert_eq!(snapshot.generation, 1);
    }

    #[test]
    fn provider_workspace_can_revalidate_after_returning_to_baseline() {
        let monitor = WorkspaceIdentityMonitor::default();
        let first = crate::io::WorkspaceIdentity::local("root-a");
        let second = crate::io::WorkspaceIdentity::local("root-b");

        monitor.observe_provider_run("run-1", "/repo".into(), first.clone());
        monitor.observe_provider_run("run-1", "/repo".into(), second);
        let snapshot = monitor.observe_provider_run("run-1", "/repo".into(), first);

        assert!(snapshot.valid);
        assert!(snapshot.identity_changed);
        assert_eq!(snapshot.generation, 2);
    }

    #[test]
    fn health_snapshot_counts_changed_and_invalid_provider_runs() {
        let monitor = WorkspaceIdentityMonitor::default();
        let first = crate::io::WorkspaceIdentity::local("root-a");
        let second = crate::io::WorkspaceIdentity::local("root-b");

        monitor.observe_provider_run("run-1", "/repo".into(), first.clone());
        monitor.observe_provider_run("run-1", "/repo".into(), second.clone());
        monitor.observe_provider_run("run-2", "/repo".into(), first);

        let health = monitor.health_snapshot();

        assert_eq!(health.tracked_provider_runs, 2);
        assert_eq!(health.identity_changed_provider_runs, 1);
        assert_eq!(health.invalid_provider_runs, 1);
        assert_eq!(health.current_generation_total, 1);
        assert_eq!(health.issues.len(), 1);
        assert_eq!(health.issues[0].provider_run_id, "run-1");
        assert_eq!(health.issues[0].root, "/repo");
        assert_eq!(health.issues[0].baseline_fingerprint, "root-a");
        assert_eq!(health.issues[0].current_fingerprint, "root-b");
        assert!(!health.issues[0].valid);
    }
}
