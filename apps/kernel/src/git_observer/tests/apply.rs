use base64::Engine as _;

use super::super::{
    GitTurnContext, WorkspaceLiveSyncApplyStatus, WorkspaceLiveSyncChange,
    WorkspaceLiveSyncFileChange, WorkspaceLiveSyncFileChangeKind,
    apply_workspace_live_sync_change_to_target, capture_turn_snapshot, git_output,
    tracked_workspace_live_sync_change_after_turn, workspace_live_sync_git_branch,
    workspace_live_sync_identity_conflict, workspace_live_sync_repo_fingerprint,
};
use super::support::{run_git, test_context};

#[test]
fn workspace_live_sync_apply_target_applies_exact_base_changes() {
    let source = std::env::temp_dir().join(format!(
        "arroba-tracked-sync-source-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let target = std::env::temp_dir().join(format!(
        "arroba-tracked-sync-target-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(source.join("src")).expect("source should be created");
    std::fs::create_dir_all(target.join("src")).expect("target should be created");
    run_git(&source, &["init"]);
    run_git(&source, &["config", "user.email", "agent@example.com"]);
    run_git(&source, &["config", "user.name", "Agent"]);
    std::fs::write(source.join("src/lib.rs"), "old\n").expect("source should write");
    std::fs::write(source.join("remove.txt"), "remove\n").expect("source should write");
    run_git(&source, &["add", "."]);
    run_git(&source, &["commit", "-m", "seed commit"]);
    std::fs::write(target.join("src/lib.rs"), "old\n").expect("target should write");
    std::fs::write(target.join("remove.txt"), "remove\n").expect("target should write");
    run_git(&target, &["init"]);
    run_git(&target, &["config", "user.email", "agent@example.com"]);
    run_git(&target, &["config", "user.name", "Agent"]);
    run_git(&target, &["add", "."]);
    run_git(&target, &["commit", "-m", "target seed"]);
    let target_head_before =
        git_output(&target, &["rev-parse", "HEAD"]).expect("target head should be readable");

    let before = capture_turn_snapshot(GitTurnContext {
        workspace_live_sync_tracked: true,
        ..test_context(&source, "prompt-1")
    })
    .expect("pre-turn snapshot should capture");
    std::fs::write(source.join("src/lib.rs"), "new\n").expect("source should update");
    std::fs::write(source.join("src/new.rs"), "added\n").expect("source should add");
    std::fs::remove_file(source.join("remove.txt")).expect("source should delete");
    let after = capture_turn_snapshot(GitTurnContext {
        workspace_live_sync_tracked: true,
        ..test_context(&source, "prompt-1")
    })
    .expect("post-turn snapshot should capture");
    let change = tracked_workspace_live_sync_change_after_turn(&before, &after)
        .expect("tracked turn should produce a change");

    let results = apply_workspace_live_sync_change_to_target(&change, &target);

    assert!(
        results
            .iter()
            .all(|result| result.status == WorkspaceLiveSyncApplyStatus::Applied)
    );
    assert_eq!(
        std::fs::read_to_string(target.join("src/lib.rs")).expect("target should read"),
        "new\n"
    );
    assert_eq!(
        std::fs::read_to_string(target.join("src/new.rs")).expect("target should read"),
        "added\n"
    );
    assert!(!target.join("remove.txt").exists());
    assert_eq!(
        git_output(&target, &["rev-parse", "HEAD"]).expect("target head should be readable"),
        target_head_before
    );
    assert!(
        git_output(&target, &["status", "--porcelain"])
            .expect("target status should be readable")
            .contains("src/lib.rs")
    );

    let _ = std::fs::remove_dir_all(&source);
    let _ = std::fs::remove_dir_all(&target);
}

#[test]
fn workspace_live_sync_apply_target_skips_conflicting_target() {
    let source = std::env::temp_dir().join(format!(
        "arroba-tracked-sync-conflict-source-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let target = std::env::temp_dir().join(format!(
        "arroba-tracked-sync-conflict-target-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(source.join("src")).expect("source should be created");
    std::fs::create_dir_all(target.join("src")).expect("target should be created");
    run_git(&source, &["init"]);
    run_git(&source, &["config", "user.email", "agent@example.com"]);
    run_git(&source, &["config", "user.name", "Agent"]);
    std::fs::write(source.join("src/lib.rs"), "old\n").expect("source should write");
    run_git(&source, &["add", "."]);
    run_git(&source, &["commit", "-m", "seed commit"]);
    std::fs::write(target.join("src/lib.rs"), "target local edit\n").expect("target should write");

    let before = capture_turn_snapshot(GitTurnContext {
        workspace_live_sync_tracked: true,
        ..test_context(&source, "prompt-1")
    })
    .expect("pre-turn snapshot should capture");
    std::fs::write(source.join("src/lib.rs"), "new\n").expect("source should update");
    let after = capture_turn_snapshot(GitTurnContext {
        workspace_live_sync_tracked: true,
        ..test_context(&source, "prompt-1")
    })
    .expect("post-turn snapshot should capture");
    let change = tracked_workspace_live_sync_change_after_turn(&before, &after)
        .expect("tracked turn should produce a change");

    let results = apply_workspace_live_sync_change_to_target(&change, &target);

    assert_eq!(
        results[0].status,
        WorkspaceLiveSyncApplyStatus::SkippedConflict
    );
    assert_eq!(
        std::fs::read_to_string(target.join("src/lib.rs")).expect("target should read"),
        "target local edit\n"
    );

    let _ = std::fs::remove_dir_all(&source);
    let _ = std::fs::remove_dir_all(&target);
}

#[test]
fn workspace_live_sync_apply_target_treats_already_applied_paths_as_applied() {
    let target = std::env::temp_dir().join(format!(
        "arroba-tracked-sync-idempotent-target-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(target.join("src")).expect("target should be created");
    std::fs::write(target.join("src/added.rs"), "added\n").expect("target should write");
    std::fs::write(target.join("src/lib.rs"), "new\n").expect("target should write");
    std::fs::write(target.join("src/new_name.rs"), "moved\n").expect("target should write");
    let encode = |value: &str| base64::engine::general_purpose::STANDARD.encode(value.as_bytes());
    let change = WorkspaceLiveSyncChange {
        session_id: "session-1".to_string(),
        agent_id: "agent-1".to_string(),
        provider_run_id: "provider-run-1".to_string(),
        prompt_id: "prompt-1".to_string(),
        repo_root: "/source".to_string(),
        worktree_path: "/source".to_string(),
        branch: Some("main".to_string()),
        changed_paths: vec![
            "src/added.rs".to_string(),
            "src/lib.rs".to_string(),
            "src/remove.rs".to_string(),
            "src/new_name.rs".to_string(),
        ],
        file_changes: vec![
            WorkspaceLiveSyncFileChange {
                path: "src/added.rs".to_string(),
                previous_path: None,
                kind: WorkspaceLiveSyncFileChangeKind::Added,
                before_content_base64: None,
                after_content_base64: Some(encode("added\n")),
                binary: false,
            },
            WorkspaceLiveSyncFileChange {
                path: "src/lib.rs".to_string(),
                previous_path: None,
                kind: WorkspaceLiveSyncFileChangeKind::Modified,
                before_content_base64: Some(encode("old\n")),
                after_content_base64: Some(encode("new\n")),
                binary: false,
            },
            WorkspaceLiveSyncFileChange {
                path: "src/remove.rs".to_string(),
                previous_path: None,
                kind: WorkspaceLiveSyncFileChangeKind::Deleted,
                before_content_base64: Some(encode("remove\n")),
                after_content_base64: None,
                binary: false,
            },
            WorkspaceLiveSyncFileChange {
                path: "src/new_name.rs".to_string(),
                previous_path: Some("src/old_name.rs".to_string()),
                kind: WorkspaceLiveSyncFileChangeKind::Renamed,
                before_content_base64: Some(encode("old\n")),
                after_content_base64: Some(encode("moved\n")),
                binary: false,
            },
        ],
        status_fingerprint: "fingerprint".to_string(),
    };

    let results = apply_workspace_live_sync_change_to_target(&change, &target);

    assert!(
        results
            .iter()
            .all(|result| result.status == WorkspaceLiveSyncApplyStatus::Applied)
    );
    assert_eq!(
        std::fs::read_to_string(target.join("src/added.rs")).expect("target should read"),
        "added\n"
    );
    assert_eq!(
        std::fs::read_to_string(target.join("src/lib.rs")).expect("target should read"),
        "new\n"
    );
    assert!(!target.join("src/remove.rs").exists());
    assert_eq!(
        std::fs::read_to_string(target.join("src/new_name.rs")).expect("target should read"),
        "moved\n"
    );

    let _ = std::fs::remove_dir_all(&target);
}

#[test]
fn workspace_live_sync_apply_target_continues_after_path_failure() {
    let target = std::env::temp_dir().join(format!(
        "arroba-tracked-sync-partial-failure-target-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&target).expect("target should be created");
    let encode = |value: &str| base64::engine::general_purpose::STANDARD.encode(value.as_bytes());
    let change = WorkspaceLiveSyncChange {
        session_id: "session-1".to_string(),
        agent_id: "agent-1".to_string(),
        provider_run_id: "provider-run-1".to_string(),
        prompt_id: "prompt-1".to_string(),
        repo_root: "/source".to_string(),
        worktree_path: "/source".to_string(),
        branch: Some("main".to_string()),
        changed_paths: vec!["../escape.txt".to_string(), "src/applied.rs".to_string()],
        file_changes: vec![
            WorkspaceLiveSyncFileChange {
                path: "../escape.txt".to_string(),
                previous_path: None,
                kind: WorkspaceLiveSyncFileChangeKind::Added,
                before_content_base64: None,
                after_content_base64: Some(encode("blocked\n")),
                binary: false,
            },
            WorkspaceLiveSyncFileChange {
                path: "src/applied.rs".to_string(),
                previous_path: None,
                kind: WorkspaceLiveSyncFileChangeKind::Added,
                before_content_base64: None,
                after_content_base64: Some(encode("applied\n")),
                binary: false,
            },
        ],
        status_fingerprint: "fingerprint".to_string(),
    };

    let results = apply_workspace_live_sync_change_to_target(&change, &target);

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].status, WorkspaceLiveSyncApplyStatus::FailedIo);
    assert!(results[0].message.contains("must be relative"));
    assert_eq!(results[1].status, WorkspaceLiveSyncApplyStatus::Applied);
    assert_eq!(
        std::fs::read_to_string(target.join("src/applied.rs")).expect("target should read"),
        "applied\n"
    );

    let _ = std::fs::remove_dir_all(&target);
}

#[test]
fn workspace_live_sync_apply_target_skips_ignored_target_path() {
    let target = std::env::temp_dir().join(format!(
        "arroba-tracked-sync-ignore-target-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(target.join("ignored")).expect("target should be created");
    std::fs::write(target.join(".arrobaignore"), "ignored/\n").expect("ignore should write");
    let encode = |value: &str| base64::engine::general_purpose::STANDARD.encode(value);
    let change = WorkspaceLiveSyncChange {
        session_id: "session-1".to_string(),
        agent_id: "agent-1".to_string(),
        provider_run_id: "provider-run-1".to_string(),
        prompt_id: "prompt-1".to_string(),
        repo_root: "/source".to_string(),
        worktree_path: "/source".to_string(),
        branch: Some("main".to_string()),
        changed_paths: vec!["ignored/file.txt".to_string()],
        file_changes: vec![WorkspaceLiveSyncFileChange {
            path: "ignored/file.txt".to_string(),
            previous_path: None,
            kind: WorkspaceLiveSyncFileChangeKind::Added,
            before_content_base64: None,
            after_content_base64: Some(encode("secret\n")),
            binary: false,
        }],
        status_fingerprint: "fingerprint".to_string(),
    };

    let results = apply_workspace_live_sync_change_to_target(&change, &target);

    assert_eq!(
        results[0].status,
        WorkspaceLiveSyncApplyStatus::SkippedConflict
    );
    assert!(results[0].message.contains("ignored"));
    assert!(!target.join("ignored/file.txt").exists());

    let _ = std::fs::remove_dir_all(&target);
}

#[test]
fn workspace_live_sync_apply_target_skips_forced_excluded_path() {
    let target = std::env::temp_dir().join(format!(
        "arroba-tracked-sync-force-exclude-target-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&target).expect("target should be created");
    let encode = |value: &str| base64::engine::general_purpose::STANDARD.encode(value);
    let change = WorkspaceLiveSyncChange {
        session_id: "session-1".to_string(),
        agent_id: "agent-1".to_string(),
        provider_run_id: "provider-run-1".to_string(),
        prompt_id: "prompt-1".to_string(),
        repo_root: "/source".to_string(),
        worktree_path: "/source".to_string(),
        branch: Some("main".to_string()),
        changed_paths: vec![".env.local".to_string()],
        file_changes: vec![WorkspaceLiveSyncFileChange {
            path: ".env.local".to_string(),
            previous_path: None,
            kind: WorkspaceLiveSyncFileChangeKind::Added,
            before_content_base64: None,
            after_content_base64: Some(encode("TOKEN=secret\n")),
            binary: false,
        }],
        status_fingerprint: "fingerprint".to_string(),
    };

    let results = apply_workspace_live_sync_change_to_target(&change, &target);

    assert_eq!(
        results[0].status,
        WorkspaceLiveSyncApplyStatus::SkippedConflict
    );
    assert!(!target.join(".env.local").exists());

    let _ = std::fs::remove_dir_all(&target);
}

#[test]
fn workspace_live_sync_apply_target_conflicts_on_binary_mismatch() {
    let target = std::env::temp_dir().join(format!(
        "arroba-tracked-sync-binary-conflict-target-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&target).expect("target should be created");
    std::fs::write(target.join("image.bin"), [0xff, 1, 9, 3]).expect("target should write");
    let encode = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
    let change = WorkspaceLiveSyncChange {
        session_id: "session-1".to_string(),
        agent_id: "agent-1".to_string(),
        provider_run_id: "provider-run-1".to_string(),
        prompt_id: "prompt-1".to_string(),
        repo_root: "/source".to_string(),
        worktree_path: "/source".to_string(),
        branch: Some("main".to_string()),
        changed_paths: vec!["image.bin".to_string()],
        file_changes: vec![WorkspaceLiveSyncFileChange {
            path: "image.bin".to_string(),
            previous_path: None,
            kind: WorkspaceLiveSyncFileChangeKind::Modified,
            before_content_base64: Some(encode(&[0xff, 1, 2, 3])),
            after_content_base64: Some(encode(&[0xff, 1, 2, 4])),
            binary: true,
        }],
        status_fingerprint: "fingerprint".to_string(),
    };

    let results = apply_workspace_live_sync_change_to_target(&change, &target);

    assert_eq!(
        results[0].status,
        WorkspaceLiveSyncApplyStatus::SkippedConflict
    );
    assert!(results[0].message.contains("binary"));
    assert_eq!(
        std::fs::read(target.join("image.bin")).expect("target should read"),
        vec![0xff, 1, 9, 3]
    );

    let _ = std::fs::remove_dir_all(&target);
}

#[test]
fn workspace_live_sync_apply_target_conflicts_on_incompatible_rename() {
    let target = std::env::temp_dir().join(format!(
        "arroba-tracked-sync-rename-conflict-target-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&target).expect("target should be created");
    std::fs::write(target.join("old.txt"), "old\n").expect("old target should write");
    std::fs::write(target.join("new.txt"), "already here\n").expect("new target should write");
    let encode = |value: &str| base64::engine::general_purpose::STANDARD.encode(value.as_bytes());
    let change = WorkspaceLiveSyncChange {
        session_id: "session-1".to_string(),
        agent_id: "agent-1".to_string(),
        provider_run_id: "provider-run-1".to_string(),
        prompt_id: "prompt-1".to_string(),
        repo_root: "/source".to_string(),
        worktree_path: "/source".to_string(),
        branch: Some("main".to_string()),
        changed_paths: vec!["new.txt".to_string()],
        file_changes: vec![WorkspaceLiveSyncFileChange {
            path: "new.txt".to_string(),
            previous_path: Some("old.txt".to_string()),
            kind: WorkspaceLiveSyncFileChangeKind::Renamed,
            before_content_base64: Some(encode("old\n")),
            after_content_base64: Some(encode("moved\n")),
            binary: false,
        }],
        status_fingerprint: "fingerprint".to_string(),
    };

    let results = apply_workspace_live_sync_change_to_target(&change, &target);

    assert_eq!(
        results[0].status,
        WorkspaceLiveSyncApplyStatus::SkippedConflict
    );
    assert!(results[0].message.contains("already exists"));
    assert_eq!(
        std::fs::read_to_string(target.join("old.txt")).expect("old target should read"),
        "old\n"
    );
    assert_eq!(
        std::fs::read_to_string(target.join("new.txt")).expect("new target should read"),
        "already here\n"
    );

    let _ = std::fs::remove_dir_all(&target);
}

#[test]
fn workspace_live_sync_identity_conflict_detects_branch_drift() {
    let target = std::env::temp_dir().join(format!(
        "arroba-tracked-sync-identity-target-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&target).expect("target should be created");
    run_git(&target, &["init"]);
    run_git(&target, &["config", "user.email", "agent@example.com"]);
    run_git(&target, &["config", "user.name", "Agent"]);
    std::fs::write(target.join("README.md"), "seed\n").expect("target should write");
    run_git(&target, &["add", "."]);
    run_git(&target, &["commit", "-m", "seed"]);
    run_git(&target, &["checkout", "-b", "sync-main"]);
    let fingerprint =
        workspace_live_sync_repo_fingerprint(&target).expect("fingerprint should resolve");

    assert_eq!(
        workspace_live_sync_git_branch(&target).as_deref(),
        Some("sync-main")
    );
    assert!(
        workspace_live_sync_identity_conflict(
            &target,
            Some("sync-main"),
            Some(fingerprint.as_str()),
        )
        .is_none()
    );

    run_git(&target, &["checkout", "-b", "other"]);

    let conflict = workspace_live_sync_identity_conflict(
        &target,
        Some("sync-main"),
        Some(fingerprint.as_str()),
    )
    .expect("branch drift should conflict");
    assert!(conflict.contains("target branch changed"));

    let _ = std::fs::remove_dir_all(&target);
}

#[test]
fn workspace_live_sync_apply_target_rebases_non_overlapping_text_changes() {
    let target = std::env::temp_dir().join(format!(
        "arroba-tracked-sync-rebase-target-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(target.join("src")).expect("target should be created");
    std::fs::write(target.join("src/lib.rs"), "a\nlocal\nb\nc\n").expect("target should write");
    let encode = |value: &str| base64::engine::general_purpose::STANDARD.encode(value.as_bytes());
    let change = WorkspaceLiveSyncChange {
        session_id: "session-1".to_string(),
        agent_id: "agent-1".to_string(),
        provider_run_id: "provider-run-1".to_string(),
        prompt_id: "prompt-1".to_string(),
        repo_root: "/source".to_string(),
        worktree_path: "/source".to_string(),
        branch: Some("main".to_string()),
        changed_paths: vec!["src/lib.rs".to_string()],
        file_changes: vec![WorkspaceLiveSyncFileChange {
            path: "src/lib.rs".to_string(),
            previous_path: None,
            kind: WorkspaceLiveSyncFileChangeKind::Modified,
            before_content_base64: Some(encode("a\nb\nc\n")),
            after_content_base64: Some(encode("a\nb\nsource\nc\n")),
            binary: false,
        }],
        status_fingerprint: "fingerprint".to_string(),
    };

    let results = apply_workspace_live_sync_change_to_target(&change, &target);

    assert_eq!(results[0].status, WorkspaceLiveSyncApplyStatus::Rebased);
    assert_eq!(
        std::fs::read_to_string(target.join("src/lib.rs")).expect("target should read"),
        "a\nlocal\nb\nsource\nc\n"
    );

    let _ = std::fs::remove_dir_all(&target);
}
