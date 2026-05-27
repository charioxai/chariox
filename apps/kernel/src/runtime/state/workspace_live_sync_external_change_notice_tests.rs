use super::*;

#[test]
fn edit_result_includes_external_change_notice_on_rejection() {
    let output = workspace_live_sync_edit_result(
        crate::io::EditResult::Rejected {
            reason: crate::io::ArtifactEditError::Conflict {
                path: PathBuf::from("src/lib.rs"),
                base_version: crate::io::ArtifactVersion::initial(),
                current_version: crate::io::ArtifactVersion::initial().next(),
                requested_ranges: vec![crate::io::TextRange::new(0, 5)],
                changed_ranges: vec![crate::io::TextRange::new(0, 5)],
                message: "overlap".to_string(),
            },
        },
        WorkspaceLiveSyncChangeContext {
            path: PathBuf::from("src/lib.rs"),
            before: None,
            after: None,
        },
        Some(crate::io::ArtifactExternalChangeNotice {
            path: PathBuf::from("src/lib.rs"),
            message: "changed outside workspace live sync".to_string(),
        }),
    );

    assert!(!output.ok);
    assert_eq!(output.payload["external_change"]["detected"], true);
    assert_eq!(output.payload["external_change"]["path"], "src/lib.rs");
    assert_eq!(output.payload["reason"]["kind"], "conflict");
}

#[test]
fn edit_result_includes_external_change_notice_on_success() {
    let output = workspace_live_sync_edit_result(
        crate::io::EditResult::AppliedWithWarning {
            new_version: crate::io::ArtifactVersion::initial().next(),
            warning: crate::io::ArtifactEditWarning::RebasedOverNonOverlappingChange {
                base_version: crate::io::ArtifactVersion::initial(),
                applied_version: crate::io::ArtifactVersion::initial().next(),
            },
        },
        WorkspaceLiveSyncChangeContext {
            path: PathBuf::from("src/lib.rs"),
            before: Some(WorkspaceLiveSyncTextSnapshot {
                existed: true,
                text: "alpha\n".to_string(),
            }),
            after: Some(WorkspaceLiveSyncTextSnapshot {
                existed: true,
                text: "beta\n".to_string(),
            }),
        },
        Some(crate::io::ArtifactExternalChangeNotice {
            path: PathBuf::from("src/lib.rs"),
            message: "changed outside workspace live sync".to_string(),
        }),
    );

    assert!(output.ok);
    assert_eq!(output.payload["external_change"]["detected"], true);
    assert_eq!(
        output.payload["external_changes"].as_array().unwrap().len(),
        1
    );
    assert_eq!(
        output.payload["warning"]["kind"],
        "rebased_over_non_overlapping_change"
    );
    assert!(output.payload["change"]["diff"]
        .as_str()
        .unwrap()
        .contains("-alpha"));
}
#[test]
fn patch_result_includes_external_change_notices() {
    let mut payload = serde_json::json!({
        "applied": true,
        "atomic": true,
        "changes": [],
    });

    add_workspace_live_sync_external_change_notices_payload(
        &mut payload,
        vec![
            crate::io::ArtifactExternalChangeNotice {
                path: PathBuf::from("src/lib.rs"),
                message: "changed".to_string(),
            },
            crate::io::ArtifactExternalChangeNotice {
                path: PathBuf::from("src/main.rs"),
                message: "changed too".to_string(),
            },
        ],
    );

    assert_eq!(payload["external_change"]["path"], "src/lib.rs");
    assert_eq!(payload["external_changes"].as_array().unwrap().len(), 2);
}

#[test]
fn remote_workspace_live_sync_identity_match_uses_repo_url_and_branch_not_root() {
    let home = crate::io::WorkspaceIdentity {
        vcs_provider: Some("git".to_string()),
        repo_id: None,
        repo_url: Some("https://github.com/example/repo.git".to_string()),
        branch: Some("main".to_string()),
        head_commit: Some("home-head".to_string()),
        worktree_root_fingerprint: "/home/repo".to_string(),
    };
    let worker = crate::io::WorkspaceIdentity {
        worktree_root_fingerprint: "/worker/repo".to_string(),
        ..home.clone()
    };

    assert!(workspace_live_sync_workspace_identities_match(
        &home, &worker
    ));
}

#[test]
fn remote_workspace_live_sync_identity_mismatch_rejects_other_branch() {
    let home = crate::io::WorkspaceIdentity {
        vcs_provider: Some("git".to_string()),
        repo_id: None,
        repo_url: Some("https://github.com/example/repo.git".to_string()),
        branch: Some("main".to_string()),
        head_commit: None,
        worktree_root_fingerprint: "/home/repo".to_string(),
    };
    let worker = crate::io::WorkspaceIdentity {
        branch: Some("feature".to_string()),
        worktree_root_fingerprint: "/worker/repo".to_string(),
        ..home.clone()
    };

    assert!(!workspace_live_sync_workspace_identities_match(
        &home, &worker
    ));
}

#[test]
fn remote_workspace_live_sync_final_apply_rejects_worker_external_change() {
    let root = std::env::temp_dir().join(format!(
        "arroba-remote-workspace-live-sync-{}",
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&root).expect("create root");
    let path = PathBuf::from("src.txt");
    std::fs::write(root.join(&path), "external\n").expect("write fixture");
    let initial = vec![remote_workspace_live_sync_state(
        &path,
        Some("base\n".to_string()),
    )];
    let final_states = vec![remote_workspace_live_sync_state(
        &path,
        Some("agent\n".to_string()),
    )];

    let result = apply_remote_workspace_live_sync_final_states(&root, &initial, &final_states)
        .expect("apply should return structured rejection");

    assert!(result.is_some());
    let result = result.unwrap();
    assert!(!result.ok);
    assert_eq!(
        result.payload["reason"]["kind"],
        "external_change_during_remote_apply"
    );
    assert_eq!(
        std::fs::read_to_string(root.join(&path)).expect("read result"),
        "external\n"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn workspace_live_sync_opaque_read_payload_includes_base64() {
    let payload = workspace_live_sync_read_payload(crate::io::ArtifactReadResult {
        artifact_id: crate::io::ArtifactId::new("artifact:opaque"),
        path: PathBuf::from("asset.bin"),
        domain: crate::io::ArtifactDomainKind::OpaqueBlob,
        version: crate::io::ArtifactVersion::initial(),
        snapshot_id: crate::io::ArtifactSnapshotId::new("snap:opaque:1"),
        content: crate::io::ArtifactContent::Bytes(vec![0, 1, 255]),
    });

    assert_eq!(payload["domain"], "opaque");
    assert_eq!(payload["byte_count"], 3);
    assert_eq!(payload["content_base64"], "AAH/");
    assert!(payload.get("content_text").is_none());
}

#[test]
fn workspace_live_sync_write_content_decodes_opaque_base64() {
    let args = crate::transport::runtime_tools::WorkspaceLiveSyncWriteArtifactArgs {
        path: "asset.bin".to_string(),
        content_text: None,
        content_base64: Some("AAH/".to_string()),
        snapshot_id: None,
        domain: Some("opaque".to_string()),
    };

    let content = workspace_live_sync_write_content_from_args(
        "test_workspace_live_sync_write_content",
        crate::io::ArtifactDomainKind::OpaqueBlob,
        &args,
    )
    .expect("opaque base64 should decode");

    assert_eq!(content, crate::io::ArtifactContent::Bytes(vec![0, 1, 255]));
}

#[test]
fn workspace_live_sync_snapshot_id_treats_create_sentinel_as_absent() {
    assert_eq!(workspace_live_sync_snapshot_id_from_arg(None), None);
    assert_eq!(
        workspace_live_sync_snapshot_id_from_arg(Some(String::new())),
        None
    );
    assert_eq!(
        workspace_live_sync_snapshot_id_from_arg(Some("__arroba_create__".to_string())),
        None
    );
    assert_eq!(
        workspace_live_sync_snapshot_id_from_arg(Some("create".to_string())),
        None
    );
    assert_eq!(
        workspace_live_sync_snapshot_id_from_arg(Some("new".to_string())),
        None
    );
    assert_eq!(
        workspace_live_sync_snapshot_id_from_arg(Some("absent".to_string())),
        None
    );
    assert_eq!(
        workspace_live_sync_snapshot_id_from_arg(Some("*".to_string())),
        None
    );
    assert_eq!(
        workspace_live_sync_snapshot_id_from_arg(Some("snap:test".to_string())),
        Some(crate::io::ArtifactSnapshotId::new("snap:test"))
    );
}

#[test]
fn workspace_live_sync_write_snapshot_id_ignores_different_artifact_snapshot() {
    assert_eq!(
        workspace_live_sync_write_snapshot_id_from_arg(
            Some("snap:repo:main:seed.txt:1:1".to_string()),
            Path::new("outputs/opencode.txt"),
        ),
        None
    );
    assert_eq!(
        workspace_live_sync_write_snapshot_id_from_arg(
            Some("snap:repo:main:outputs/opencode.txt:2:4".to_string()),
            Path::new("outputs/opencode.txt"),
        ),
        Some(crate::io::ArtifactSnapshotId::new(
            "snap:repo:main:outputs/opencode.txt:2:4"
        ))
    );
}

#[test]
fn remote_workspace_live_sync_final_apply_writes_opaque_bytes() {
    let root = std::env::temp_dir().join(format!(
        "arroba-remote-managed-opaque-{}",
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&root).expect("create root");
    let path = PathBuf::from("asset.bin");
    std::fs::write(root.join(&path), [1, 2, 3]).expect("write fixture");
    let initial = vec![remote_workspace_live_sync_state_from_content(
        &path,
        Some(crate::io::ArtifactContent::Bytes(vec![1, 2, 3])),
    )];
    let final_states = vec![remote_workspace_live_sync_state_from_content(
        &path,
        Some(crate::io::ArtifactContent::Bytes(vec![4, 5, 6])),
    )];

    let result = apply_remote_workspace_live_sync_final_states(&root, &initial, &final_states)
        .expect("opaque final apply should run");

    assert!(result.is_none());
    assert_eq!(
        std::fs::read(root.join(&path)).expect("read result"),
        vec![4, 5, 6]
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn managed_whole_file_operations_move_and_delete_opaque_bytes() {
    let root = std::env::temp_dir().join(format!(
        "arroba-managed-whole-file-{}",
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::write(root.join("from.bin"), [1, 2, 3]).expect("write source");
    std::fs::write(root.join("delete.bin"), [9, 8]).expect("write delete target");
    let workspace = crate::io::WorkspaceIdentity::local("whole-file-repo");
    let mut coordinator = crate::io::ArtifactEditCoordinator::new();
    let monitor = crate::io::ArtifactExternalChangeMonitor::default();

    let result = apply_managed_whole_file_operations(
        &mut coordinator,
        workspace.clone(),
        root.clone(),
        crate::io::ArtifactDomainKind::OpaqueBlob,
        vec![
            ManagedWholeFileOperation::Move {
                from_path: PathBuf::from("from.bin"),
                to_path: PathBuf::from("to.bin"),
            },
            ManagedWholeFileOperation::Delete {
                path: PathBuf::from("delete.bin"),
            },
        ],
        crate::io::ArtifactReservationOwner::new("run-1", Some("agent-1".to_string()), "test"),
        &monitor,
    )
    .expect("whole-file operations should run");

    assert!(result.ok);
    assert!(!root.join("from.bin").exists());
    assert!(!root.join("delete.bin").exists());
    assert_eq!(std::fs::read(root.join("to.bin")).unwrap(), vec![1, 2, 3]);
    let to_id = coordinator.resolve_artifact_id(&workspace, &PathBuf::from("to.bin"));
    assert_eq!(
        coordinator.current_content(&to_id),
        Some(&crate::io::ArtifactContent::Bytes(vec![1, 2, 3]))
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn remote_managed_whole_file_operations_return_opaque_move_and_delete_states() {
    let workspace = crate::io::WorkspaceIdentity::local("remote-whole-file-repo");
    let workspace_context = WorkspaceLiveSyncWorkspaceContext {
        root: PathBuf::from("/tmp/remote-whole-file-repo"),
        identity: workspace.clone(),
        generation: 0,
        identity_changed: false,
        valid: true,
    };
    let mut coordinator = crate::io::ArtifactEditCoordinator::new();
    let states = vec![
        remote_workspace_live_sync_state_from_content(
            &PathBuf::from("from.bin"),
            Some(crate::io::ArtifactContent::Bytes(vec![1, 2, 3])),
        ),
        remote_workspace_live_sync_state_from_content(&PathBuf::from("to.bin"), None),
        remote_workspace_live_sync_state_from_content(
            &PathBuf::from("delete.bin"),
            Some(crate::io::ArtifactContent::Bytes(vec![9, 8])),
        ),
    ];

    let (result, final_states) = apply_remote_managed_whole_file_operations(
        &mut coordinator,
        workspace.clone(),
        crate::io::ArtifactDomainKind::OpaqueBlob,
        vec![
            ManagedWholeFileOperation::Move {
                from_path: PathBuf::from("from.bin"),
                to_path: PathBuf::from("to.bin"),
            },
            ManagedWholeFileOperation::Delete {
                path: PathBuf::from("delete.bin"),
            },
        ],
        states,
        crate::io::ArtifactReservationOwner::new(
            "remote:run-1",
            Some("agent-1".to_string()),
            "arroba.move_artifact",
        ),
        &workspace_context,
    )
    .expect("remote whole-file operations should apply");

    assert!(result.ok);
    assert_eq!(final_states.len(), 3);
    assert!(
        !remote_workspace_live_sync_state_for_path(&final_states, &PathBuf::from("from.bin"))
            .unwrap()
            .exists
    );
    assert_eq!(
        remote_workspace_live_sync_state_for_path(&final_states, &PathBuf::from("to.bin"))
            .unwrap()
            .content_base64
            .as_deref(),
        Some("AQID")
    );
    assert!(
        !remote_workspace_live_sync_state_for_path(&final_states, &PathBuf::from("delete.bin"))
            .unwrap()
            .exists
    );
    let to_id = coordinator.resolve_artifact_id(&workspace, &PathBuf::from("to.bin"));
    assert_eq!(
        coordinator.current_content(&to_id),
        Some(&crate::io::ArtifactContent::Bytes(vec![1, 2, 3]))
    );
}

#[test]
fn remote_managed_opaque_move_final_apply_preserves_deleted_source_domain() {
    let root = std::env::temp_dir().join(format!(
        "arroba-remote-managed-opaque-move-{}",
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::write(root.join("from.bin"), [0, 8, 255, 10]).expect("write source");
    let workspace = crate::io::WorkspaceIdentity::local("remote-opaque-move-repo");
    let workspace_context = WorkspaceLiveSyncWorkspaceContext {
        root: root.clone(),
        identity: workspace.clone(),
        generation: 0,
        identity_changed: false,
        valid: true,
    };
    let mut coordinator = crate::io::ArtifactEditCoordinator::new();
    let initial_states = vec![
        remote_workspace_live_sync_state_from_content_with_domain(
            &PathBuf::from("from.bin"),
            Some(crate::io::ArtifactContent::Bytes(vec![0, 8, 255, 10])),
            crate::io::ArtifactDomainKind::OpaqueBlob,
        ),
        remote_workspace_live_sync_state_from_content_with_domain(
            &PathBuf::from("to.bin"),
            None,
            crate::io::ArtifactDomainKind::OpaqueBlob,
        ),
    ];

    let (result, final_states) = apply_remote_managed_whole_file_operations(
        &mut coordinator,
        workspace.clone(),
        crate::io::ArtifactDomainKind::OpaqueBlob,
        vec![ManagedWholeFileOperation::Move {
            from_path: PathBuf::from("from.bin"),
            to_path: PathBuf::from("to.bin"),
        }],
        initial_states.clone(),
        crate::io::ArtifactReservationOwner::new(
            "remote:run-1",
            Some("agent-1".to_string()),
            "arroba.move_artifact",
        ),
        &workspace_context,
    )
    .expect("remote whole-file operation should apply");

    assert!(result.ok);
    assert_eq!(
        remote_workspace_live_sync_state_for_path(&final_states, &PathBuf::from("from.bin"))
            .unwrap()
            .domain
            .as_deref(),
        Some("opaque")
    );

    let final_apply =
        apply_remote_workspace_live_sync_final_states(&root, &initial_states, &final_states)
            .expect("opaque final apply should not decode deleted source as text");

    assert!(final_apply.is_none());
    assert!(!root.join("from.bin").exists());
    assert_eq!(
        std::fs::read(root.join("to.bin")).expect("read moved opaque bytes"),
        vec![0, 8, 255, 10]
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn remote_managed_patch_operations_return_move_and_delete_final_states() {
    let root = std::env::temp_dir().join(format!(
        "arroba-remote-managed-patch-{}",
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&root).expect("create root");
    let workspace = crate::io::WorkspaceIdentity::local("remote-patch-repo");
    let workspace_context = WorkspaceLiveSyncWorkspaceContext {
        root: root.clone(),
        identity: workspace.clone(),
        generation: 0,
        identity_changed: false,
        valid: true,
    };
    let mut coordinator = crate::io::ArtifactEditCoordinator::new();
    let states = vec![
        remote_workspace_live_sync_state(&PathBuf::from("a.txt"), Some("hello\n".to_string())),
        remote_workspace_live_sync_state(&PathBuf::from("b.txt"), None),
        remote_workspace_live_sync_state(&PathBuf::from("c.txt"), Some("delete me\n".to_string())),
    ];

    let (result, final_states) = apply_remote_managed_patch_operations(
        &mut coordinator,
        workspace.clone(),
        crate::io::ArtifactDomainKind::TextDocument,
        vec![
            ManagedPatchOperation::Move {
                from_path: PathBuf::from("a.txt"),
                to_path: PathBuf::from("b.txt"),
                old_text: Some("hello\n".to_string()),
                new_text: Some("goodbye\n".to_string()),
            },
            ManagedPatchOperation::Delete {
                path: PathBuf::from("c.txt"),
            },
        ],
        states,
        crate::io::ArtifactReservationOwner::new(
            "remote:run-1",
            Some("agent-1".to_string()),
            "arroba.apply_patch",
        ),
        &workspace_context,
    )
    .expect("remote patch should apply");

    assert!(result.ok);
    assert_eq!(final_states.len(), 3);
    assert_eq!(
        remote_workspace_live_sync_state_for_path(&final_states, &PathBuf::from("a.txt"))
            .unwrap()
            .content_text,
        None
    );
    assert_eq!(
        remote_workspace_live_sync_state_for_path(&final_states, &PathBuf::from("b.txt"))
            .unwrap()
            .content_text
            .as_deref(),
        Some("goodbye\n")
    );
    assert_eq!(
        remote_workspace_live_sync_state_for_path(&final_states, &PathBuf::from("c.txt"))
            .unwrap()
            .content_text,
        None
    );
    let b_id = coordinator.resolve_artifact_id(&workspace, &PathBuf::from("b.txt"));
    assert_eq!(
        coordinator.current_content(&b_id),
        Some(&crate::io::ArtifactContent::Text("goodbye\n".to_string()))
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn workspace_live_sync_ignore_initializes_from_gitignore() {
    let root = std::env::temp_dir().join(format!(
        "arroba-workspace-live-sync-ignore-{}",
        crate::session::unix_epoch_ms()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::write(root.join(".gitignore"), "ignored/\n*.secret\n").expect("write gitignore");

    workspace_live_sync_reject_ignored_path(
        &root,
        &PathBuf::from("src/lib.rs"),
        "test_workspace_live_sync_ignore",
    )
    .expect("ordinary file should not be ignored");
    let initialized = std::fs::read_to_string(root.join(".arrobaignore"))
        .expect(".arrobaignore should initialize from .gitignore");
    assert_eq!(initialized, "ignored/\n*.secret\n");
    assert!(workspace_live_sync_reject_ignored_path(
        &root,
        &PathBuf::from("ignored/file.txt"),
        "test_workspace_live_sync_ignore",
    )
    .is_err());
    assert!(workspace_live_sync_reject_ignored_path(
        &root,
        &PathBuf::from("nested/token.secret"),
        "test_workspace_live_sync_ignore",
    )
    .is_err());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn workspace_live_sync_force_excludes_runtime_and_private_paths() {
    let root = std::env::temp_dir().join(format!(
        "arroba-workspace-live-sync-force-ignore-{}",
        crate::session::unix_epoch_ms()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create root");

    for path in [
        ".git/config",
        ".arroba/logs/kernel.ndjson",
        ".arrobaignore",
        ".env",
        ".env.local",
        "node_modules/pkg/index.js",
        "target/debug/app",
    ] {
        assert!(
            workspace_live_sync_reject_ignored_path(
                &root,
                &PathBuf::from(path),
                "test_workspace_live_sync_force_ignore",
            )
            .is_err(),
            "{path} should be force excluded"
        );
    }

    let _ = std::fs::remove_dir_all(root);
}
