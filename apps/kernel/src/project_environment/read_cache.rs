//! MP-08 / MP-10 / MP-11: derived specification reads; fresh Check state stays outside this cache.
use super::*;
use crate::{error::DaemonError, session::RuntimeProject};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
};

const ENTRIES: usize = 8;
const BYTES: usize = 32 * 1024 * 1024;
#[derive(Clone, PartialEq, Eq)]
struct Version(Vec<Option<Vec<i128>>>);
struct Entry {
    root: PathBuf,
    project: RuntimeProject,
    version: Version,
    snapshot: Arc<ProjectEnvironment>,
    bytes: usize,
}
static CACHE: OnceLock<Mutex<VecDeque<Entry>>> = OnceLock::new();

impl ProjectEnvironmentStore {
    pub(crate) fn cached_read_snapshot(
        &self,
        project: &RuntimeProject,
    ) -> Result<ProjectEnvironment, DaemonError> {
        let Some(before) = self.read_version(project.id()) else {
            return self.snapshot(project);
        };
        let cache = CACHE.get_or_init(|| Mutex::new(VecDeque::new()));
        let cached = {
            let mut entries = cache
                .lock()
                .map_err(|_| environment_error("Environment read cache unavailable"))?;
            entries
                .iter()
                .position(|e| e.root == self.root && e.project == *project && e.version == before)
                .map(|index| {
                    let entry = entries.remove(index).unwrap();
                    let snapshot = entry.snapshot.clone();
                    entries.push_back(entry);
                    snapshot
                })
        };
        if let Some(snapshot) = cached {
            return Ok((*snapshot).clone());
        }
        let snapshot = self.snapshot(project)?;
        // An atomic publication during the read must never bless a mixed snapshot.
        if self.read_version(project.id()).as_ref() != Some(&before) {
            return Ok(snapshot);
        }
        let bytes = serde_json::to_vec(&snapshot)
            .map_err(|_| environment_error("Environment cache encoding failed"))?
            .len();
        if bytes > BYTES / 2 {
            return Ok(snapshot);
        }
        let mut entries = cache
            .lock()
            .map_err(|_| environment_error("Environment read cache unavailable"))?;
        entries.retain(|e| e.root != self.root || e.project.id() != project.id());
        entries.push_back(Entry {
            root: self.root.clone(),
            project: project.clone(),
            version: before,
            snapshot: Arc::new(snapshot.clone()),
            bytes,
        });
        while entries.len() > ENTRIES || entries.iter().map(|e| e.bytes).sum::<usize>() > BYTES {
            entries.pop_front();
        }
        Ok(snapshot)
    }
    #[cfg(unix)]
    fn read_version(&self, project: &str) -> Option<Version> {
        use std::os::unix::fs::MetadataExt;
        let base = self.path(project);
        let mut stamps = vec![];
        for (path, directory) in std::iter::once((self.root.clone(), true)).chain(
            [
                base.clone(),
                base.with_extension("identity.json"),
                base.with_extension("detect.json"),
                base.with_extension("revisions.json"),
            ]
            .into_iter()
            .map(|p| (p, false)),
        ) {
            let metadata = match std::fs::symlink_metadata(path) {
                Ok(m) => m,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    stamps.push(None);
                    continue;
                }
                Err(_) => return None,
            };
            if metadata.file_type().is_symlink()
                || (directory && !metadata.is_dir())
                || (!directory && !metadata.is_file())
            {
                return None;
            }
            let mut stamp = vec![
                metadata.dev() as i128,
                metadata.ino() as i128,
                metadata.uid() as i128,
                metadata.mode() as i128,
            ];
            // Lock creation/chmod can touch the directory's times without changing an input.
            if !directory {
                stamp.extend([
                    metadata.len() as i128,
                    metadata.mtime() as i128,
                    metadata.mtime_nsec() as i128,
                    metadata.ctime() as i128,
                    metadata.ctime_nsec() as i128,
                ]);
            }
            stamps.push(Some(stamp));
        }
        Some(Version(stamps))
    }
    #[cfg(not(unix))]
    fn read_version(&self, _project: &str) -> Option<Version> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn read_cache_tracks_real_publication_project_changes_and_invalid_files() {
        let root = crate::test_support::TestWorktree::new("environment-read-cache");
        let store = ProjectEnvironmentStore::new(root.path());
        let project = RuntimeProject::new(
            "cache-project",
            "owner",
            "/public",
            "Public",
            crate::session::RuntimeProjectKind::Named,
        );
        let initial = store.cached_read_snapshot(&project).unwrap();
        assert_eq!(store.cached_read_snapshot(&project).unwrap(), initial);
        assert_eq!(store.cached_read_snapshot(&project).unwrap(), initial);
        let target = EnvironmentTargetBinding {
            machine_id: "machine".into(),
            target_instance_generation: "generation".into(),
            slice_ref: None,
        };
        let mut detection = EnvironmentDetectionCache {
            project_id: project.id().into(),
            evidence_digest: "evidence-one".into(),
            modeled_folders: Default::default(),
            proposals: vec![EnvironmentProposal {
                proposal_id: "unbound-input-token".into(),
                requirement: Requirement {
                    requirement_id: "package".into(),
                    title: "Node".into(),
                    scope: RequirementScope::Folder {
                        folder_id: initial.folders[0].folder_id.clone(),
                    },
                    spec: RequirementSpec::Software {
                        identity: "node".into(),
                        version_constraint: Some("1".into()),
                        platform: None,
                        install_scope: EnvironmentInstallScope::Project,
                        install_source: None,
                        detect_only: true,
                    },
                    depends_on: vec![],
                    platform_variants: vec![],
                    origins: vec![],
                    required: true,
                    legacy_entry: None,
                },
            }],
            operation: EnvironmentOperation {
                operation_id: "detect".into(),
                attempt: 1,
                local_project_id: project.id().into(),
                revision_digest: initial.content_digest.clone(),
                target: target.clone(),
                kind: EnvironmentOperationKind::Detect,
                phase: EnvironmentOperationPhase::Ready,
                selected_items: vec![],
                per_item_opt_ins: vec![],
                per_item_results: vec![],
                created_at_ms: 1,
                updated_at_ms: 1,
                cancellation: None,
                receipts: vec![],
                recovery_state: EnvironmentRecoveryState::Settled,
            },
        };
        store.save_detection(&detection).unwrap();
        let first = store.cached_read_snapshot(&project).unwrap();
        assert_eq!(first.proposals.len(), 1);
        assert_eq!(store.cached_read_snapshot(&project).unwrap(), first);
        detection.proposals[0].requirement.title = "Updated Node".into();
        store.save_detection(&detection).unwrap();
        let second = store.cached_read_snapshot(&project).unwrap();
        assert_ne!(
            second.proposals[0].proposal_id, first.proposals[0].proposal_id,
            "published specification changes must rotate review tokens"
        );
        let mut checks = check_store::EnvironmentChecks::default();
        let mut operation = detection.operation.clone();
        operation.kind = EnvironmentOperationKind::Check;
        operation.operation_id = "fresh-check".into();
        operation.phase = EnvironmentOperationPhase::Requested;
        checks.operations.push(operation);
        store.save_checks(project.id(), &checks).unwrap();
        let mut current = store.cached_read_snapshot(&project).unwrap();
        assert_eq!(
            current, second,
            "the specification cache contains no Check journal state"
        );
        store.attach_checks(&mut current, &target, 2).unwrap();
        assert_eq!(
            current.operations.last().unwrap().operation_id,
            "fresh-check"
        );
        let mut restarted = store.cached_read_snapshot(&project).unwrap();
        let mut generation = target.clone();
        generation.target_instance_generation = "restarted".into();
        store.attach_checks(&mut restarted, &generation, 3).unwrap();
        assert_eq!(
            restarted.operations.last().unwrap().phase,
            EnvironmentOperationPhase::Failed,
            "a cached specification must never retain an old process's Running operation"
        );
        std::fs::remove_file(store.path(project.id()).with_extension("detect.json")).unwrap();
        assert_eq!(
            store.cached_read_snapshot(&project).unwrap(),
            initial,
            "deleted discovery cannot remain in a cached view"
        );
        let other = RuntimeProject::new(
            "cache-project",
            "other-owner",
            "/other",
            "Other",
            crate::session::RuntimeProjectKind::Named,
        );
        let changed = store.cached_read_snapshot(&other).unwrap();
        assert_eq!(changed.folders.len(), 1);
        assert_eq!(changed.folders[0].local_workspace_binding, "/other");
        assert_ne!(changed, initial);
        let corrupt = store.path(project.id()).with_extension("revisions.json");
        std::fs::write(&corrupt, b"{}").unwrap();
        assert!(
            store.cached_read_snapshot(&project).is_err(),
            "invalid durable publication cannot return a cached view"
        );
        std::fs::remove_file(&corrupt).unwrap();
        assert_eq!(store.cached_read_snapshot(&project).unwrap(), initial);
        #[cfg(unix)]
        {
            let outside = root.path().join("outside-revision");
            std::fs::write(&outside, b"{}").unwrap();
            std::os::unix::fs::symlink(outside, &corrupt).unwrap();
            assert!(
                store.cached_read_snapshot(&project).is_err(),
                "symlink replacement cannot reuse an old cache entry"
            );
        }
    }
}
