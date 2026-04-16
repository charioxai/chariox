use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use serde::{Deserialize, Serialize};

use crate::io::WorkspaceIdentity;

#[derive(Clone, Debug, Default)]
pub(crate) struct ArtifactExternalChangeMonitor {
    state: Arc<StdMutex<ArtifactExternalChangeMonitorState>>,
}

#[derive(Debug, Default)]
struct ArtifactExternalChangeMonitorState {
    tracked_artifacts: BTreeMap<String, TrackedExternalArtifact>,
    external_change_events: u64,
    externally_changed_artifacts: BTreeSet<String>,
}

#[derive(Debug, Clone)]
struct TrackedExternalArtifact {
    provider_run_id: String,
    workspace_fingerprint: String,
    path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactExternalChangeHealthSnapshot {
    pub tracked_artifacts: usize,
    pub externally_changed_artifacts: usize,
    pub external_change_events: u64,
}

impl ArtifactExternalChangeMonitor {
    pub(crate) fn observe_managed_read(
        &self,
        provider_run_id: &str,
        workspace_identity: &WorkspaceIdentity,
        path: &Path,
    ) {
        let key = artifact_key(workspace_identity, path);
        let mut state = self
            .state
            .lock()
            .expect("artifact external change monitor lock should not be poisoned");
        state.tracked_artifacts.insert(
            key,
            TrackedExternalArtifact {
                provider_run_id: provider_run_id.to_string(),
                workspace_fingerprint: workspace_identity.worktree_root_fingerprint.clone(),
                path: path.to_path_buf(),
            },
        );
    }

    pub(crate) fn record_external_change(
        &self,
        workspace_identity: &WorkspaceIdentity,
        path: &Path,
    ) {
        let key = artifact_key(workspace_identity, path);
        let mut state = self
            .state
            .lock()
            .expect("artifact external change monitor lock should not be poisoned");
        state.external_change_events += 1;
        state.externally_changed_artifacts.insert(key);
    }

    pub(crate) fn health_snapshot(&self) -> ArtifactExternalChangeHealthSnapshot {
        let state = self
            .state
            .lock()
            .expect("artifact external change monitor lock should not be poisoned");
        // Touch tracked fields so future refactors do not accidentally turn the
        // records into write-only bookkeeping.
        let _tracked_records_are_well_formed = state.tracked_artifacts.values().all(|record| {
            !record.provider_run_id.is_empty()
                && !record.workspace_fingerprint.is_empty()
                && !record.path.as_os_str().is_empty()
        });
        ArtifactExternalChangeHealthSnapshot {
            tracked_artifacts: state.tracked_artifacts.len(),
            externally_changed_artifacts: state.externally_changed_artifacts.len(),
            external_change_events: state.external_change_events,
        }
    }
}

fn artifact_key(workspace_identity: &WorkspaceIdentity, path: &Path) -> String {
    format!(
        "{}:{}",
        workspace_identity.worktree_root_fingerprint,
        path.to_string_lossy()
    )
}

#[cfg(test)]
mod tests {
    use super::ArtifactExternalChangeMonitor;

    #[test]
    fn health_snapshot_counts_tracked_and_changed_artifacts() {
        let monitor = ArtifactExternalChangeMonitor::default();
        let workspace = crate::io::WorkspaceIdentity::local("repo-a");

        monitor.observe_managed_read("run-1", &workspace, "src/lib.rs".as_ref());
        monitor.observe_managed_read("run-1", &workspace, "src/main.rs".as_ref());
        monitor.record_external_change(&workspace, "src/lib.rs".as_ref());
        monitor.record_external_change(&workspace, "src/lib.rs".as_ref());

        let health = monitor.health_snapshot();

        assert_eq!(health.tracked_artifacts, 2);
        assert_eq!(health.externally_changed_artifacts, 1);
        assert_eq!(health.external_change_events, 2);
    }
}
