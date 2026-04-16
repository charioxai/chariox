use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::io::text::{TextDocumentDomain, TextEditPlan};
use crate::io::types::{
    AgentEditIntent, ArtifactContent, ArtifactDomainKind, ArtifactEditError, ArtifactEditWarning,
    ArtifactId, ArtifactReadResult, ArtifactSnapshotId, ArtifactVersion, EditResult, TextRange,
    WorkspaceIdentity,
};

#[derive(Debug, Clone)]
pub struct ArtifactReadRequest {
    pub workspace_identity: WorkspaceIdentity,
    pub path: PathBuf,
    pub domain: ArtifactDomainKind,
    pub content: ArtifactContent,
}

#[derive(Debug, Clone)]
pub struct ArtifactWriteRequest {
    pub workspace_identity: WorkspaceIdentity,
    pub intent: AgentEditIntent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedArtifactEdit {
    pub artifact_id: ArtifactId,
    pub path: PathBuf,
    pub domain: ArtifactDomainKind,
    pub previous_version: ArtifactVersion,
    pub new_version: ArtifactVersion,
    pub content: ArtifactContent,
    pub warning: Option<ArtifactEditWarning>,
}

#[derive(Debug, Clone)]
struct TrackedArtifact {
    path: PathBuf,
    domain: ArtifactDomainKind,
    version: ArtifactVersion,
    content: ArtifactContent,
    content_hash: String,
}

#[derive(Debug, Clone)]
struct SnapshotRecord {
    artifact_id: ArtifactId,
    version: ArtifactVersion,
    content: ArtifactContent,
}

#[derive(Debug, Clone)]
struct ActiveEditReservation {
    owner: String,
    ranges: Vec<TextRange>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactReservationToken {
    artifact_id: ArtifactId,
    owner: String,
}

#[derive(Debug, Default)]
pub struct ArtifactEditCoordinator {
    artifacts: BTreeMap<ArtifactId, TrackedArtifact>,
    snapshots: BTreeMap<ArtifactSnapshotId, SnapshotRecord>,
    active_reservations: BTreeMap<ArtifactId, Vec<ActiveEditReservation>>,
    next_snapshot: u64,
}

impl ArtifactEditCoordinator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn read_artifact(&mut self, request: ArtifactReadRequest) -> ArtifactReadResult {
        let artifact_id = artifact_id_for(&request.workspace_identity, &request.path);
        let content_hash = hash_content(&request.content);
        let version = match self.artifacts.get(&artifact_id) {
            Some(tracked) if tracked.content_hash == content_hash => tracked.version,
            Some(tracked) => tracked.version.next(),
            None => ArtifactVersion::initial(),
        };
        let snapshot_id = self.allocate_snapshot_id(&artifact_id, version);
        let tracked = TrackedArtifact {
            path: request.path.clone(),
            domain: request.domain,
            version,
            content: request.content.clone(),
            content_hash,
        };
        self.snapshots.insert(
            snapshot_id.clone(),
            SnapshotRecord {
                artifact_id: artifact_id.clone(),
                version,
                content: request.content.clone(),
            },
        );
        self.artifacts.insert(artifact_id.clone(), tracked);
        ArtifactReadResult {
            artifact_id,
            path: request.path,
            domain: request.domain,
            version,
            snapshot_id,
            content: request.content,
        }
    }

    pub fn apply_edit(&mut self, request: ArtifactWriteRequest) -> EditResult {
        match self.prepare_edit(request) {
            Ok(prepared) => self.commit_prepared_edit(prepared),
            Err(reason) => EditResult::Rejected { reason },
        }
    }

    pub fn prepare_edit(
        &self,
        request: ArtifactWriteRequest,
    ) -> Result<PreparedArtifactEdit, ArtifactEditError> {
        self.prepare_edit_inner(request)
    }

    pub fn commit_prepared_edit(&mut self, prepared: PreparedArtifactEdit) -> EditResult {
        match self.commit_prepared_edit_inner(prepared) {
            Ok(result) => result,
            Err(reason) => EditResult::Rejected { reason },
        }
    }

    pub fn resolve_artifact_id(
        &self,
        workspace_identity: &WorkspaceIdentity,
        path: &Path,
    ) -> ArtifactId {
        artifact_id_for(workspace_identity, path)
    }

    pub fn current_content(&self, artifact_id: &ArtifactId) -> Option<&ArtifactContent> {
        self.artifacts
            .get(artifact_id)
            .map(|tracked| &tracked.content)
    }

    pub fn forget_artifact(&mut self, workspace_identity: &WorkspaceIdentity, path: &Path) {
        let artifact_id = artifact_id_for(workspace_identity, path);
        self.artifacts.remove(&artifact_id);
        self.snapshots
            .retain(|_, snapshot| snapshot.artifact_id != artifact_id);
        self.active_reservations.remove(&artifact_id);
    }

    pub fn try_reserve_ranges(
        &mut self,
        workspace_identity: &WorkspaceIdentity,
        path: &Path,
        ranges: Vec<TextRange>,
        owner: impl Into<String>,
    ) -> Result<ArtifactReservationToken, ArtifactEditError> {
        let owner = owner.into();
        let artifact_id = artifact_id_for(workspace_identity, path);
        let ranges = if ranges.is_empty() {
            vec![whole_artifact_range()]
        } else {
            ranges
        };
        if let Some(active) = self.active_reservations.get(&artifact_id) {
            for reservation in active {
                if reservation.owner == owner {
                    continue;
                }
                if ranges_overlap(&ranges, &reservation.ranges) {
                    let tracked_version = self
                        .artifacts
                        .get(&artifact_id)
                        .map(|tracked| tracked.version)
                        .unwrap_or_else(ArtifactVersion::initial);
                    return Err(ArtifactEditError::Conflict {
                        path: path.to_path_buf(),
                        base_version: tracked_version,
                        current_version: tracked_version,
                        requested_ranges: ranges,
                        changed_ranges: reservation.ranges.clone(),
                        message: "managed I/O edit range is currently reserved by another writer"
                            .to_string(),
                    });
                }
            }
        }
        self.active_reservations
            .entry(artifact_id.clone())
            .or_default()
            .push(ActiveEditReservation {
                owner: owner.clone(),
                ranges,
            });
        Ok(ArtifactReservationToken { artifact_id, owner })
    }

    pub fn release_reservation(&mut self, token: ArtifactReservationToken) {
        let Some(reservations) = self.active_reservations.get_mut(&token.artifact_id) else {
            return;
        };
        if let Some(index) = reservations
            .iter()
            .position(|reservation| reservation.owner == token.owner)
        {
            reservations.remove(index);
        }
        if reservations.is_empty() {
            self.active_reservations.remove(&token.artifact_id);
        }
    }

    fn prepare_edit_inner(
        &self,
        request: ArtifactWriteRequest,
    ) -> Result<PreparedArtifactEdit, ArtifactEditError> {
        let artifact_id = artifact_id_for(&request.workspace_identity, &request.intent.path);
        let tracked = self.artifacts.get(&artifact_id).cloned().ok_or_else(|| {
            ArtifactEditError::ArtifactNotTracked {
                path: request.intent.path.clone(),
            }
        })?;
        match tracked.domain {
            ArtifactDomainKind::TextDocument => self.apply_text_edit(artifact_id, tracked, request),
            domain => Err(ArtifactEditError::UnsupportedDomain { domain }),
        }
    }

    fn apply_text_edit(
        &self,
        artifact_id: ArtifactId,
        tracked: TrackedArtifact,
        request: ArtifactWriteRequest,
    ) -> Result<PreparedArtifactEdit, ArtifactEditError> {
        let current =
            tracked
                .content
                .as_text()
                .ok_or_else(|| ArtifactEditError::UnsupportedDomain {
                    domain: tracked.domain,
                })?;
        let (base_version, base_content) = match request.intent.snapshot_id.as_ref() {
            Some(snapshot_id) => {
                let snapshot = self.snapshots.get(snapshot_id).ok_or_else(|| {
                    ArtifactEditError::SnapshotNotFound {
                        snapshot_id: snapshot_id.clone(),
                    }
                })?;
                if snapshot.artifact_id != artifact_id {
                    return Err(ArtifactEditError::InvalidOperation {
                        message: "snapshot belongs to a different artifact".to_string(),
                    });
                }
                let base = snapshot.content.as_text().ok_or_else(|| {
                    ArtifactEditError::UnsupportedDomain {
                        domain: tracked.domain,
                    }
                })?;
                (snapshot.version, base.to_string())
            }
            None => (tracked.version, current.to_string()),
        };
        let plan = TextDocumentDomain::plan_operation(&base_content, &request.intent.operation)?;
        let rebased = match TextDocumentDomain::rebase_plan(&base_content, current, &plan) {
            Ok(rebased) => rebased,
            Err(ArtifactEditError::Conflict { .. }) => {
                return Err(self.text_conflict_error(
                    &tracked,
                    base_version,
                    current,
                    &base_content,
                    &plan,
                ));
            }
            Err(error) => return Err(error),
        };
        let new_content = TextDocumentDomain::apply_plan(current, &rebased)?;
        let new_version = tracked.version.next();
        let content = ArtifactContent::Text(new_content);
        let warning = if base_version != tracked.version {
            Some(ArtifactEditWarning::RebasedOverNonOverlappingChange {
                base_version,
                applied_version: tracked.version,
            })
        } else {
            None
        };
        Ok(PreparedArtifactEdit {
            artifact_id,
            path: tracked.path,
            domain: tracked.domain,
            previous_version: tracked.version,
            new_version,
            content,
            warning,
        })
    }

    fn text_conflict_error(
        &self,
        tracked: &TrackedArtifact,
        base_version: ArtifactVersion,
        current: &str,
        base: &str,
        plan: &TextEditPlan,
    ) -> ArtifactEditError {
        ArtifactEditError::Conflict {
            path: tracked.path.clone(),
            base_version,
            current_version: tracked.version,
            requested_ranges: vec![plan.range],
            changed_ranges: TextDocumentDomain::changed_ranges(base, current),
            message: "edit overlaps changes made since the base snapshot; reread and retry"
                .to_string(),
        }
    }

    fn commit_prepared_edit_inner(
        &mut self,
        prepared: PreparedArtifactEdit,
    ) -> Result<EditResult, ArtifactEditError> {
        let tracked = self.artifacts.get(&prepared.artifact_id).ok_or_else(|| {
            ArtifactEditError::ArtifactNotTracked {
                path: prepared.path.clone(),
            }
        })?;
        if tracked.version != prepared.previous_version {
            return Err(ArtifactEditError::Conflict {
                path: prepared.path,
                base_version: prepared.previous_version,
                current_version: tracked.version,
                requested_ranges: Vec::new(),
                changed_ranges: Vec::new(),
                message: "artifact changed after edit preparation; reread and retry".to_string(),
            });
        }

        let snapshot_id = self.allocate_snapshot_id(&prepared.artifact_id, prepared.new_version);
        self.snapshots.insert(
            snapshot_id,
            SnapshotRecord {
                artifact_id: prepared.artifact_id.clone(),
                version: prepared.new_version,
                content: prepared.content.clone(),
            },
        );
        self.artifacts.insert(
            prepared.artifact_id,
            TrackedArtifact {
                path: prepared.path,
                domain: prepared.domain,
                version: prepared.new_version,
                content: prepared.content.clone(),
                content_hash: hash_content(&prepared.content),
            },
        );
        if let Some(warning) = prepared.warning {
            Ok(EditResult::AppliedWithWarning {
                new_version: prepared.new_version,
                warning,
            })
        } else {
            Ok(EditResult::Applied {
                new_version: prepared.new_version,
            })
        }
    }

    fn allocate_snapshot_id(
        &mut self,
        artifact_id: &ArtifactId,
        version: ArtifactVersion,
    ) -> ArtifactSnapshotId {
        self.next_snapshot += 1;
        ArtifactSnapshotId::new(format!(
            "snap:{}:{}:{}",
            artifact_id.as_str(),
            version.value(),
            self.next_snapshot
        ))
    }
}

fn artifact_id_for(workspace_identity: &WorkspaceIdentity, path: &Path) -> ArtifactId {
    ArtifactId::new(format!(
        "{}:{}",
        workspace_identity.worktree_root_fingerprint,
        normalize_path(path)
    ))
}

fn normalize_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn hash_content(content: &ArtifactContent) -> String {
    let mut hasher = Sha256::new();
    match content {
        ArtifactContent::Text(text) => {
            hasher.update(b"text\0");
            hasher.update(text.as_bytes());
        }
        ArtifactContent::Bytes(bytes) => {
            hasher.update(b"bytes\0");
            hasher.update(bytes);
        }
    }
    format!("{:x}", hasher.finalize())
}

fn whole_artifact_range() -> TextRange {
    TextRange::new(0, usize::MAX)
}

fn ranges_overlap(left: &[TextRange], right: &[TextRange]) -> bool {
    left.iter()
        .any(|left| right.iter().any(|right| left.overlaps(*right)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::types::{AgentEditOperation, TextRange};

    fn workspace() -> WorkspaceIdentity {
        WorkspaceIdentity::local("repo-a")
    }

    fn read_text(
        coordinator: &mut ArtifactEditCoordinator,
        path: &str,
        content: &str,
    ) -> ArtifactReadResult {
        coordinator.read_artifact(ArtifactReadRequest {
            workspace_identity: workspace(),
            path: PathBuf::from(path),
            domain: ArtifactDomainKind::TextDocument,
            content: ArtifactContent::Text(content.to_string()),
        })
    }

    #[test]
    fn managed_read_tracks_snapshot_and_version() {
        let mut coordinator = ArtifactEditCoordinator::new();
        let read = read_text(&mut coordinator, "src/lib.rs", "fn main() {}\n");

        assert_eq!(read.version, ArtifactVersion::initial());
        assert_eq!(read.domain, ArtifactDomainKind::TextDocument);
        assert!(coordinator.current_content(&read.artifact_id).is_some());
    }

    #[test]
    fn text_edit_applies_against_current_snapshot() {
        let mut coordinator = ArtifactEditCoordinator::new();
        let read = read_text(&mut coordinator, "src/lib.rs", "alpha\nbeta\n");
        let result = coordinator.apply_edit(ArtifactWriteRequest {
            workspace_identity: workspace(),
            intent: AgentEditIntent {
                path: PathBuf::from("src/lib.rs"),
                snapshot_id: Some(read.snapshot_id),
                operation: AgentEditOperation::ReplaceText {
                    old_text: "beta".to_string(),
                    new_text: "gamma".to_string(),
                },
            },
        });

        assert!(matches!(result, EditResult::Applied { .. }));
        assert_eq!(
            coordinator.current_content(&read.artifact_id),
            Some(&ArtifactContent::Text("alpha\ngamma\n".to_string()))
        );
    }

    #[test]
    fn stale_non_overlapping_text_edit_rebases_and_applies() {
        let mut coordinator = ArtifactEditCoordinator::new();
        let first_read = read_text(&mut coordinator, "src/lib.rs", "one\ntwo\nthree\n");
        let second_read = coordinator.read_artifact(ArtifactReadRequest {
            workspace_identity: workspace(),
            path: PathBuf::from("src/lib.rs"),
            domain: ArtifactDomainKind::TextDocument,
            content: ArtifactContent::Text("zero\none\ntwo\nthree\n".to_string()),
        });
        assert_eq!(second_read.version, ArtifactVersion::initial().next());

        let result = coordinator.apply_edit(ArtifactWriteRequest {
            workspace_identity: workspace(),
            intent: AgentEditIntent {
                path: PathBuf::from("src/lib.rs"),
                snapshot_id: Some(first_read.snapshot_id),
                operation: AgentEditOperation::ReplaceText {
                    old_text: "three".to_string(),
                    new_text: "four".to_string(),
                },
            },
        });

        assert!(matches!(result, EditResult::AppliedWithWarning { .. }));
        assert_eq!(
            coordinator.current_content(&first_read.artifact_id),
            Some(&ArtifactContent::Text("zero\none\ntwo\nfour\n".to_string()))
        );
    }

    #[test]
    fn stale_edit_between_multiple_external_changes_rebases_and_applies() {
        let mut coordinator = ArtifactEditCoordinator::new();
        let first_read = read_text(&mut coordinator, "src/lib.rs", "a\nb\nc\nd\ne\n");
        let _ = coordinator.read_artifact(ArtifactReadRequest {
            workspace_identity: workspace(),
            path: PathBuf::from("src/lib.rs"),
            domain: ArtifactDomainKind::TextDocument,
            content: ArtifactContent::Text("A\nb\nc\nd\nE\n".to_string()),
        });

        let result = coordinator.apply_edit(ArtifactWriteRequest {
            workspace_identity: workspace(),
            intent: AgentEditIntent {
                path: PathBuf::from("src/lib.rs"),
                snapshot_id: Some(first_read.snapshot_id),
                operation: AgentEditOperation::ReplaceText {
                    old_text: "c".to_string(),
                    new_text: "C".to_string(),
                },
            },
        });

        assert!(matches!(result, EditResult::AppliedWithWarning { .. }));
        assert_eq!(
            coordinator.current_content(&first_read.artifact_id),
            Some(&ArtifactContent::Text("A\nb\nC\nd\nE\n".to_string()))
        );
    }

    #[test]
    fn stale_overlapping_text_edit_is_rejected() {
        let mut coordinator = ArtifactEditCoordinator::new();
        let first_read = read_text(&mut coordinator, "src/lib.rs", "one\ntwo\nthree\n");
        let _ = coordinator.apply_edit(ArtifactWriteRequest {
            workspace_identity: workspace(),
            intent: AgentEditIntent {
                path: PathBuf::from("src/lib.rs"),
                snapshot_id: Some(first_read.snapshot_id.clone()),
                operation: AgentEditOperation::ReplaceText {
                    old_text: "two".to_string(),
                    new_text: "TWO".to_string(),
                },
            },
        });

        let result = coordinator.apply_edit(ArtifactWriteRequest {
            workspace_identity: workspace(),
            intent: AgentEditIntent {
                path: PathBuf::from("src/lib.rs"),
                snapshot_id: Some(first_read.snapshot_id),
                operation: AgentEditOperation::ReplaceRange {
                    range: TextRange::new(4, 7),
                    old_text: "two".to_string(),
                    new_text: "deux".to_string(),
                },
            },
        });

        assert!(matches!(
            result,
            EditResult::Rejected {
                reason: ArtifactEditError::Conflict { .. }
            }
        ));
    }

    #[test]
    fn edit_reservations_reject_overlapping_writers() {
        let mut coordinator = ArtifactEditCoordinator::new();
        let workspace = workspace();
        let first = coordinator
            .try_reserve_ranges(
                &workspace,
                Path::new("src/lib.rs"),
                vec![TextRange::new(4, 8)],
                "agent-a",
            )
            .expect("first reservation should be accepted");

        let rejected = coordinator.try_reserve_ranges(
            &workspace,
            Path::new("src/lib.rs"),
            vec![TextRange::new(7, 10)],
            "agent-b",
        );
        assert!(matches!(rejected, Err(ArtifactEditError::Conflict { .. })));

        coordinator.release_reservation(first);
        assert!(coordinator
            .try_reserve_ranges(
                &workspace,
                Path::new("src/lib.rs"),
                vec![TextRange::new(7, 10)],
                "agent-b",
            )
            .is_ok());
    }

    #[test]
    fn edit_reservations_allow_non_overlapping_writers() {
        let mut coordinator = ArtifactEditCoordinator::new();
        let workspace = workspace();
        let _first = coordinator
            .try_reserve_ranges(
                &workspace,
                Path::new("src/lib.rs"),
                vec![TextRange::new(0, 3)],
                "agent-a",
            )
            .expect("first reservation should be accepted");

        assert!(coordinator
            .try_reserve_ranges(
                &workspace,
                Path::new("src/lib.rs"),
                vec![TextRange::new(4, 8)],
                "agent-b",
            )
            .is_ok());
    }
}
