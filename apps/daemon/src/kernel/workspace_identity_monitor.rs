use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

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
}
