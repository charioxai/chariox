use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, UNIX_EPOCH};

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
    live_watcher_started: bool,
    live_watcher_scans: u64,
    live_watcher_scan_errors: u64,
}

#[derive(Debug, Clone)]
struct TrackedExternalArtifact {
    provider_run_id: String,
    workspace_fingerprint: String,
    workspace_root: PathBuf,
    path: PathBuf,
    last_observed_signature: Option<FileSignature>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileSignature {
    len: u64,
    modified_ms: Option<u128>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactExternalChangeHealthSnapshot {
    pub tracked_artifacts: usize,
    pub externally_changed_artifacts: usize,
    pub external_change_events: u64,
    pub live_watcher_started: bool,
    pub live_watcher_scans: u64,
    pub live_watcher_scan_errors: u64,
    pub issues: Vec<ArtifactExternalChangeIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactExternalChangeIssue {
    pub artifact_key: String,
    pub provider_run_id: Option<String>,
    pub workspace_fingerprint: String,
    pub workspace_root: Option<String>,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactExternalChangeNotice {
    pub path: PathBuf,
    pub message: String,
}

impl ArtifactExternalChangeMonitor {
    pub(crate) fn observe_managed_read(
        &self,
        provider_run_id: &str,
        workspace_identity: &WorkspaceIdentity,
        workspace_root: &Path,
        path: &Path,
    ) {
        self.ensure_live_watcher_started();
        let key = artifact_key(workspace_identity, path);
        let full_path = workspace_root.join(path);
        let signature = file_signature(&full_path);
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.tracked_artifacts.insert(
            key,
            TrackedExternalArtifact {
                provider_run_id: provider_run_id.to_string(),
                workspace_fingerprint: workspace_identity.worktree_root_fingerprint.clone(),
                workspace_root: workspace_root.to_path_buf(),
                path: path.to_path_buf(),
                last_observed_signature: signature,
            },
        );
    }

    pub(crate) fn observe_managed_write(
        &self,
        provider_run_id: &str,
        workspace_identity: &WorkspaceIdentity,
        workspace_root: &Path,
        path: &Path,
    ) {
        let key = artifact_key(workspace_identity, path);
        let full_path = workspace_root.join(path);
        let signature = file_signature(&full_path);
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.externally_changed_artifacts.remove(&key);
        state.tracked_artifacts.insert(
            key,
            TrackedExternalArtifact {
                provider_run_id: provider_run_id.to_string(),
                workspace_fingerprint: workspace_identity.worktree_root_fingerprint.clone(),
                workspace_root: workspace_root.to_path_buf(),
                path: path.to_path_buf(),
                last_observed_signature: signature,
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
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.external_change_events += 1;
        state.externally_changed_artifacts.insert(key);
    }

    pub(crate) fn external_change_notice(
        &self,
        workspace_identity: &WorkspaceIdentity,
        path: &Path,
    ) -> Option<ArtifactExternalChangeNotice> {
        self.external_change_notices(workspace_identity, vec![path.to_path_buf()])
            .into_iter()
            .next()
    }

    pub(crate) fn external_change_notices(
        &self,
        workspace_identity: &WorkspaceIdentity,
        paths: impl IntoIterator<Item = PathBuf>,
    ) -> Vec<ArtifactExternalChangeNotice> {
        self.scan_tracked_artifacts_once();
        let paths = paths.into_iter().collect::<Vec<_>>();
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        paths
            .into_iter()
            .filter_map(|path| {
                let key = artifact_key(workspace_identity, &path);
                state
                    .externally_changed_artifacts
                    .contains(&key)
                    .then(|| external_change_notice_for_path(path))
            })
            .collect()
    }

    fn ensure_live_watcher_started(&self) {
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.live_watcher_started {
                return;
            }
            state.live_watcher_started = true;
        }
        let monitor = self.clone();
        if tokio::runtime::Handle::try_current().is_ok() {
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_millis(750));
                loop {
                    interval.tick().await;
                    monitor.scan_tracked_artifacts_once();
                }
            });
        }
    }

    pub(crate) fn scan_tracked_artifacts_once(&self) {
        let tracked = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state
                .tracked_artifacts
                .iter()
                .map(|(key, artifact)| (key.clone(), artifact.clone()))
                .collect::<Vec<_>>()
        };

        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.live_watcher_scans += 1;
        for (key, artifact) in tracked {
            let full_path = artifact.workspace_root.join(&artifact.path);
            let signature = file_signature(&full_path);
            let changed = {
                let Some(current) = state.tracked_artifacts.get_mut(&key) else {
                    continue;
                };
                if current.last_observed_signature == signature {
                    false
                } else {
                    current.last_observed_signature = signature;
                    true
                }
            };
            if changed {
                state.external_change_events += 1;
                state.externally_changed_artifacts.insert(key);
            }
        }
    }

    pub(crate) fn health_snapshot(&self) -> ArtifactExternalChangeHealthSnapshot {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let issues = state
            .externally_changed_artifacts
            .iter()
            .map(|key| external_change_issue_for_key(key, state.tracked_artifacts.get(key)))
            .collect();
        ArtifactExternalChangeHealthSnapshot {
            tracked_artifacts: state.tracked_artifacts.len(),
            externally_changed_artifacts: state.externally_changed_artifacts.len(),
            external_change_events: state.external_change_events,
            live_watcher_started: state.live_watcher_started,
            live_watcher_scans: state.live_watcher_scans,
            live_watcher_scan_errors: state.live_watcher_scan_errors,
            issues,
        }
    }
}

fn external_change_notice_for_path(path: PathBuf) -> ArtifactExternalChangeNotice {
    ArtifactExternalChangeNotice {
        path,
        message:
            "artifact changed outside Chariox workspace live sync after the last managed observation"
                .to_string(),
    }
}

fn artifact_key(workspace_identity: &WorkspaceIdentity, path: &Path) -> String {
    format!(
        "{}:{}",
        workspace_identity.worktree_root_fingerprint,
        path.to_string_lossy()
    )
}

fn external_change_issue_for_key(
    key: &str,
    tracked: Option<&TrackedExternalArtifact>,
) -> ArtifactExternalChangeIssue {
    if let Some(tracked) = tracked {
        return ArtifactExternalChangeIssue {
            artifact_key: key.to_string(),
            provider_run_id: Some(tracked.provider_run_id.clone()),
            workspace_fingerprint: tracked.workspace_fingerprint.clone(),
            workspace_root: Some(tracked.workspace_root.to_string_lossy().into_owned()),
            path: tracked.path.to_string_lossy().into_owned(),
        };
    }

    let (workspace_fingerprint, path) = key
        .split_once(':')
        .map(|(fingerprint, path)| (fingerprint.to_string(), path.to_string()))
        .unwrap_or_else(|| (String::new(), key.to_string()));
    ArtifactExternalChangeIssue {
        artifact_key: key.to_string(),
        provider_run_id: None,
        workspace_fingerprint,
        workspace_root: None,
        path,
    }
}

fn file_signature(path: &Path) -> Option<FileSignature> {
    match fs::metadata(path) {
        Ok(metadata) => Some(FileSignature {
            len: metadata.len(),
            modified_ms: metadata
                .modified()
                .ok()
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_millis()),
        }),
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::ArtifactExternalChangeMonitor;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_root(name: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("chariox-external-monitor-{name}-{nanos}"));
        fs::create_dir_all(&root).expect("create test root");
        root
    }

    #[test]
    fn health_snapshot_counts_tracked_and_changed_artifacts() {
        let monitor = ArtifactExternalChangeMonitor::default();
        let workspace = crate::io::WorkspaceIdentity::local("repo-a");
        let root = test_root("health");

        monitor.observe_managed_read("run-1", &workspace, &root, "src/lib.rs".as_ref());
        monitor.observe_managed_read("run-1", &workspace, &root, "src/main.rs".as_ref());
        monitor.record_external_change(&workspace, "src/lib.rs".as_ref());
        monitor.record_external_change(&workspace, "src/lib.rs".as_ref());

        let health = monitor.health_snapshot();

        assert_eq!(health.tracked_artifacts, 2);
        assert_eq!(health.externally_changed_artifacts, 1);
        assert_eq!(health.external_change_events, 2);
        assert!(health.live_watcher_started);
        assert_eq!(health.issues.len(), 1);
        assert_eq!(health.issues[0].provider_run_id.as_deref(), Some("run-1"));
        assert_eq!(health.issues[0].path, "src/lib.rs");
    }

    #[test]
    fn live_scan_records_tracked_artifact_changes() {
        let monitor = ArtifactExternalChangeMonitor::default();
        let workspace = crate::io::WorkspaceIdentity::local("repo-a");
        let root = test_root("scan");
        let file = root.join("src/lib.rs");
        fs::create_dir_all(file.parent().unwrap()).expect("create parent");
        fs::write(&file, "alpha\n").expect("write fixture");

        monitor.observe_managed_read("run-1", &workspace, &root, "src/lib.rs".as_ref());
        fs::write(&file, "alpha\nbeta\n").expect("external write");
        monitor.scan_tracked_artifacts_once();

        let health = monitor.health_snapshot();
        assert_eq!(health.externally_changed_artifacts, 1);
        assert_eq!(health.external_change_events, 1);
        assert_eq!(health.live_watcher_scans, 1);
        assert_eq!(health.issues.len(), 1);
        assert_eq!(health.issues[0].provider_run_id.as_deref(), Some("run-1"));
        assert_eq!(
            health.issues[0].workspace_root.as_deref(),
            Some(root.to_str().unwrap())
        );
        assert_eq!(health.issues[0].path, "src/lib.rs");
    }

    #[test]
    fn managed_write_refreshes_signature_without_external_event() {
        let monitor = ArtifactExternalChangeMonitor::default();
        let workspace = crate::io::WorkspaceIdentity::local("repo-a");
        let root = test_root("managed-write");
        let file = root.join("src/lib.rs");
        fs::create_dir_all(file.parent().unwrap()).expect("create parent");
        fs::write(&file, "alpha\n").expect("write fixture");

        monitor.observe_managed_read("run-1", &workspace, &root, "src/lib.rs".as_ref());
        fs::write(&file, "managed\n").expect("workspace live sync write fixture");
        monitor.observe_managed_write("run-1", &workspace, &root, "src/lib.rs".as_ref());
        monitor.scan_tracked_artifacts_once();

        let health = monitor.health_snapshot();
        assert_eq!(health.externally_changed_artifacts, 0);
        assert_eq!(health.external_change_events, 0);
        assert!(health.issues.is_empty());
    }

    #[test]
    fn external_change_notice_scans_and_reports_tracked_path() {
        let monitor = ArtifactExternalChangeMonitor::default();
        let workspace = crate::io::WorkspaceIdentity::local("repo-a");
        let root = test_root("notice");
        let file = root.join("src/lib.rs");
        fs::create_dir_all(file.parent().unwrap()).expect("create parent");
        fs::write(&file, "alpha\n").expect("write fixture");

        monitor.observe_managed_read("run-1", &workspace, &root, "src/lib.rs".as_ref());
        fs::write(&file, "external\n").expect("external write");

        let notice = monitor
            .external_change_notice(&workspace, "src/lib.rs".as_ref())
            .expect("external change should be noticed");

        assert_eq!(notice.path, std::path::PathBuf::from("src/lib.rs"));
        assert!(notice
            .message
            .contains("outside Chariox workspace live sync"));
    }

    #[test]
    fn external_change_notices_scan_once_for_multiple_paths() {
        let monitor = ArtifactExternalChangeMonitor::default();
        let workspace = crate::io::WorkspaceIdentity::local("repo-a");
        let root = test_root("multi-notice");
        let first = root.join("src/lib.rs");
        let second = root.join("src/main.rs");
        fs::create_dir_all(first.parent().unwrap()).expect("create parent");
        fs::write(&first, "alpha\n").expect("write first");
        fs::write(&second, "one\n").expect("write second");

        monitor.observe_managed_read("run-1", &workspace, &root, "src/lib.rs".as_ref());
        monitor.observe_managed_read("run-1", &workspace, &root, "src/main.rs".as_ref());
        fs::write(&first, "external\n").expect("external write");

        let notices = monitor.external_change_notices(
            &workspace,
            vec![
                std::path::PathBuf::from("src/lib.rs"),
                std::path::PathBuf::from("src/main.rs"),
            ],
        );

        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].path, std::path::PathBuf::from("src/lib.rs"));
    }
}
