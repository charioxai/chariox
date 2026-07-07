use super::*;
use crate::io::types::{AgentEditOperation, ArtifactEditWarning, TextRange};

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

fn read_opaque(
    coordinator: &mut ArtifactEditCoordinator,
    path: &str,
    content: &[u8],
) -> ArtifactReadResult {
    coordinator.read_artifact(ArtifactReadRequest {
        workspace_identity: workspace(),
        path: PathBuf::from(path),
        domain: ArtifactDomainKind::OpaqueBlob,
        content: ArtifactContent::Bytes(content.to_vec()),
    })
}

fn text_range_of(haystack: &str, needle: &str) -> TextRange {
    let start = haystack.find(needle).expect("needle should exist");
    TextRange::new(start, start + needle.len())
}

fn git_workspace(root: &str) -> WorkspaceIdentity {
    WorkspaceIdentity {
        vcs_provider: Some("git".to_string()),
        repo_id: None,
        repo_url: Some("https://github.com/example/repo.git".to_string()),
        branch: Some("main".to_string()),
        head_commit: Some("commit-a".to_string()),
        worktree_root_fingerprint: root.to_string(),
    }
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
fn opaque_write_replaces_whole_artifact() {
    let mut coordinator = ArtifactEditCoordinator::new();
    let read = read_opaque(&mut coordinator, "assets/logo.bin", &[0, 1, 2]);
    let result = coordinator.apply_edit(ArtifactWriteRequest {
        workspace_identity: workspace(),
        intent: AgentEditIntent {
            path: PathBuf::from("assets/logo.bin"),
            snapshot_id: Some(read.snapshot_id),
            operation: AgentEditOperation::WriteArtifact {
                content: ArtifactContent::Bytes(vec![3, 4, 5, 6]),
            },
        },
    });

    assert!(matches!(result, EditResult::Applied { .. }));
    assert_eq!(
        coordinator.current_content(&read.artifact_id),
        Some(&ArtifactContent::Bytes(vec![3, 4, 5, 6]))
    );
}

#[test]
fn stale_opaque_write_rejects_as_whole_file_conflict() {
    let mut coordinator = ArtifactEditCoordinator::new();
    let first_read = read_opaque(&mut coordinator, "assets/logo.bin", &[0, 1, 2]);
    let _second_read = read_opaque(&mut coordinator, "assets/logo.bin", &[0, 1, 9]);

    let result = coordinator.apply_edit(ArtifactWriteRequest {
        workspace_identity: workspace(),
        intent: AgentEditIntent {
            path: PathBuf::from("assets/logo.bin"),
            snapshot_id: Some(first_read.snapshot_id),
            operation: AgentEditOperation::WriteArtifact {
                content: ArtifactContent::Bytes(vec![3, 4, 5]),
            },
        },
    });

    assert!(matches!(
        result,
        EditResult::Rejected {
            reason: ArtifactEditError::Conflict { .. }
        }
    ));
    assert_eq!(
        coordinator.current_content(&first_read.artifact_id),
        Some(&ArtifactContent::Bytes(vec![0, 1, 9]))
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
fn concurrent_non_overlapping_snapshot_edits_land_on_intended_targets() {
    let mut coordinator = ArtifactEditCoordinator::new();
    let base = "left\nmiddle\nright\n";
    let first_read = read_text(&mut coordinator, "src/lib.rs", base);
    let second_read = read_text(&mut coordinator, "src/lib.rs", base);

    let first_result = coordinator.apply_edit(ArtifactWriteRequest {
        workspace_identity: workspace(),
        intent: AgentEditIntent {
            path: PathBuf::from("src/lib.rs"),
            snapshot_id: Some(first_read.snapshot_id),
            operation: AgentEditOperation::ReplaceRange {
                range: text_range_of(base, "left"),
                old_text: "left".to_string(),
                new_text: "LEFT!".to_string(),
            },
        },
    });
    assert!(matches!(first_result, EditResult::Applied { .. }));

    let second_result = coordinator.apply_edit(ArtifactWriteRequest {
        workspace_identity: workspace(),
        intent: AgentEditIntent {
            path: PathBuf::from("src/lib.rs"),
            snapshot_id: Some(second_read.snapshot_id),
            operation: AgentEditOperation::ReplaceRange {
                range: text_range_of(base, "right"),
                old_text: "right".to_string(),
                new_text: "RIGHT!".to_string(),
            },
        },
    });

    assert!(matches!(
        second_result,
        EditResult::AppliedWithWarning {
            warning: ArtifactEditWarning::RebasedOverNonOverlappingChange { .. },
            ..
        }
    ));
    assert_eq!(
        coordinator.current_content(&second_read.artifact_id),
        Some(&ArtifactContent::Text(
            "LEFT!\nmiddle\nRIGHT!\n".to_string()
        ))
    );
}

#[test]
fn git_workspace_artifacts_coordinate_across_different_roots() {
    let mut coordinator = ArtifactEditCoordinator::new();
    let local = git_workspace("/local/repo");
    let remote = git_workspace("/remote/repo");
    let local_read = coordinator.read_artifact(ArtifactReadRequest {
        workspace_identity: local,
        path: PathBuf::from("src/lib.rs"),
        domain: ArtifactDomainKind::TextDocument,
        content: ArtifactContent::Text("left\nright\n".to_string()),
    });
    let remote_read = coordinator.read_artifact(ArtifactReadRequest {
        workspace_identity: remote,
        path: PathBuf::from("src/lib.rs"),
        domain: ArtifactDomainKind::TextDocument,
        content: ArtifactContent::Text("left\nright\n".to_string()),
    });

    assert_eq!(local_read.artifact_id, remote_read.artifact_id);
    assert_eq!(remote_read.version.value(), 1);
}

#[test]
fn stale_range_edit_rebases_across_before_and_after_insertions_exactly() {
    let mut coordinator = ArtifactEditCoordinator::new();
    let base = "header\nalpha\nTARGET\nomega\nfooter\n";
    let first_read = read_text(&mut coordinator, "src/lib.rs", base);
    let _ = coordinator.read_artifact(ArtifactReadRequest {
        workspace_identity: workspace(),
        path: PathBuf::from("src/lib.rs"),
        domain: ArtifactDomainKind::TextDocument,
        content: ArtifactContent::Text(
            "intro\nheader\nalpha\nTARGET\nomega\nfooter\noutro\n".to_string(),
        ),
    });

    let result = coordinator.apply_edit(ArtifactWriteRequest {
        workspace_identity: workspace(),
        intent: AgentEditIntent {
            path: PathBuf::from("src/lib.rs"),
            snapshot_id: Some(first_read.snapshot_id),
            operation: AgentEditOperation::ReplaceRange {
                range: text_range_of(base, "TARGET"),
                old_text: "TARGET".to_string(),
                new_text: "REPLACED".to_string(),
            },
        },
    });

    assert!(matches!(
        result,
        EditResult::AppliedWithWarning {
            warning: ArtifactEditWarning::RebasedOverNonOverlappingChange { .. },
            ..
        }
    ));
    assert_eq!(
        coordinator.current_content(&first_read.artifact_id),
        Some(&ArtifactContent::Text(
            "intro\nheader\nalpha\nREPLACED\nomega\nfooter\noutro\n".to_string()
        ))
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
    let owner_a = ArtifactReservationOwner::new("run-a", Some("agent-a".to_string()), "edit");
    let owner_b = ArtifactReservationOwner::new("run-b", Some("agent-b".to_string()), "edit");
    let first = coordinator
        .try_reserve_ranges(
            &workspace,
            Path::new("src/lib.rs"),
            vec![TextRange::new(4, 8)],
            owner_a,
        )
        .expect("first reservation should be accepted");

    let rejected = coordinator.try_reserve_ranges(
        &workspace,
        Path::new("src/lib.rs"),
        vec![TextRange::new(7, 10)],
        owner_b.clone(),
    );
    assert!(matches!(
        rejected,
        Err(ArtifactEditError::ActiveReservationConflict { .. })
    ));

    coordinator.release_reservation(first);
    assert!(coordinator
        .try_reserve_ranges(
            &workspace,
            Path::new("src/lib.rs"),
            vec![TextRange::new(7, 10)],
            owner_b,
        )
        .is_ok());
}

#[test]
fn edit_reservations_allow_non_overlapping_writers() {
    let mut coordinator = ArtifactEditCoordinator::new();
    let workspace = workspace();
    let owner_a = ArtifactReservationOwner::new("run-a", Some("agent-a".to_string()), "edit");
    let owner_b = ArtifactReservationOwner::new("run-b", Some("agent-b".to_string()), "edit");
    let _first = coordinator
        .try_reserve_ranges(
            &workspace,
            Path::new("src/lib.rs"),
            vec![TextRange::new(0, 3)],
            owner_a,
        )
        .expect("first reservation should be accepted");

    assert!(coordinator
        .try_reserve_ranges(
            &workspace,
            Path::new("src/lib.rs"),
            vec![TextRange::new(4, 8)],
            owner_b,
        )
        .is_ok());
}
#[test]
fn reservation_lifecycle_allows_retry_after_release() {
    let mut coordinator = ArtifactEditCoordinator::new();
    let workspace = workspace();
    let owner_a = ArtifactReservationOwner::new("run-a", Some("agent-a".to_string()), "edit");
    let owner_b = ArtifactReservationOwner::new("run-b", Some("agent-b".to_string()), "edit");

    let first = coordinator
        .try_reserve_ranges(
            &workspace,
            Path::new("src/lib.rs"),
            vec![TextRange::new(0, 10)],
            owner_a,
        )
        .expect("first reservation should be accepted");
    assert_eq!(coordinator.active_reservation_snapshots().len(), 1);

    let rejected = coordinator.try_reserve_ranges(
        &workspace,
        Path::new("src/lib.rs"),
        vec![TextRange::new(5, 12)],
        owner_b.clone(),
    );
    assert!(matches!(
        rejected,
        Err(ArtifactEditError::ActiveReservationConflict { .. })
    ));
    assert_eq!(coordinator.active_reservation_snapshots().len(), 1);

    coordinator.release_reservation(first);
    assert!(coordinator.active_reservation_snapshots().is_empty());
    assert!(coordinator
        .try_reserve_ranges(
            &workspace,
            Path::new("src/lib.rs"),
            vec![TextRange::new(5, 12)],
            owner_b,
        )
        .is_ok());
}

#[test]
fn reservations_are_scoped_per_artifact() {
    let mut coordinator = ArtifactEditCoordinator::new();
    let workspace = workspace();
    let owner_a = ArtifactReservationOwner::new("run-a", Some("agent-a".to_string()), "edit");
    let owner_b = ArtifactReservationOwner::new("run-b", Some("agent-b".to_string()), "edit");

    let _first = coordinator
        .try_reserve_ranges(
            &workspace,
            Path::new("src/lib.rs"),
            vec![TextRange::new(0, 10)],
            owner_a,
        )
        .expect("first reservation should be accepted");

    assert!(coordinator
        .try_reserve_ranges(
            &workspace,
            Path::new("src/main.rs"),
            vec![TextRange::new(0, 10)],
            owner_b,
        )
        .is_ok());
    assert_eq!(coordinator.active_reservation_snapshots().len(), 2);
}
