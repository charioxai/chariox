use base64::Engine as _;

use super::super::{
    capture_turn_snapshot, tracked_workspace_live_sync_change_after_turn, GitTurnContext,
    WorkspaceLiveSyncFileChangeKind,
};
use super::support::{run_git, test_context, tracked_snapshot};

#[test]
fn tracked_workspace_live_sync_change_records_clean_turn_paths() {
    let before = tracked_snapshot(false, "");
    let after = tracked_snapshot(true, " M src/lib.rs\n?? new.txt");

    let change = tracked_workspace_live_sync_change_after_turn(&before, &after)
        .expect("clean tracked turn should journal changed paths");

    assert_eq!(change.changed_paths, vec!["new.txt", "src/lib.rs"]);
    assert_eq!(change.status_fingerprint, " M src/lib.rs\n?? new.txt");
}

#[test]
fn tracked_workspace_live_sync_change_skips_dirty_start() {
    let before = tracked_snapshot(true, " M src/lib.rs");
    let after = tracked_snapshot(true, " M src/lib.rs\n M src/other.rs");

    assert!(tracked_workspace_live_sync_change_after_turn(&before, &after).is_none());
}

#[test]
fn tracked_workspace_live_sync_change_records_dirty_to_dirty_content_delta() {
    let root = std::env::temp_dir().join(format!(
        "chariox-tracked-sync-dirty-delta-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(root.join("outputs")).expect("temp repo should be created");
    run_git(&root, &["init"]);
    run_git(&root, &["config", "user.email", "agent@example.com"]);
    run_git(&root, &["config", "user.name", "Agent"]);
    std::fs::write(root.join("outputs/conflict.txt"), "one\ntarget\nthree\n")
        .expect("tracked file should write");
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "seed commit"]);
    std::fs::write(root.join("outputs/conflict.txt"), "one\nsource\nthree\n")
        .expect("dirty file should write");

    let before = capture_turn_snapshot(GitTurnContext {
        workspace_live_sync_tracked: true,
        ..test_context(&root, "prompt-1")
    })
    .expect("dirty pre-turn snapshot should capture");
    std::fs::write(root.join("outputs/conflict.txt"), "one\nresolved\nthree\n")
        .expect("dirty file should update again");
    let after = capture_turn_snapshot(GitTurnContext {
        workspace_live_sync_tracked: true,
        ..test_context(&root, "prompt-1")
    })
    .expect("dirty post-turn snapshot should capture");

    let change = tracked_workspace_live_sync_change_after_turn(&before, &after)
        .expect("dirty tracked turn should produce a content delta");

    assert_eq!(change.changed_paths, vec!["outputs/conflict.txt"]);
    assert_eq!(change.file_changes.len(), 1);
    assert_eq!(
        change.file_changes[0].kind,
        WorkspaceLiveSyncFileChangeKind::Modified
    );
    let before_content = base64::engine::general_purpose::STANDARD
        .decode(
            change.file_changes[0]
                .before_content_base64
                .as_deref()
                .expect("before content should be present"),
        )
        .expect("before content should decode");
    let after_content = base64::engine::general_purpose::STANDARD
        .decode(
            change.file_changes[0]
                .after_content_base64
                .as_deref()
                .expect("after content should be present"),
        )
        .expect("after content should decode");
    assert_eq!(before_content, b"one\nsource\nthree\n");
    assert_eq!(after_content, b"one\nresolved\nthree\n");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn tracked_workspace_live_sync_change_filters_forced_exclusions() {
    let before = tracked_snapshot(false, "");
    let after = tracked_snapshot(
        true,
        " M .env\n M .envrc\n M config/.env.local\n M .chariox/state.json\n M .codex/session.json\n M .opencode/state.json\n M .claude/settings.json\n M .cursor/index.json\n M daemon.sock\n M daemon.socket\n M .tmp-chariox/socket\n M .tmp-live-workspace-live-sync-drill/state.json\n M .tmp-live-remote-workspace-live-sync-drill/state.json\n M history/session.jsonl\n M session-history/session.jsonl\n M operational-history/events.db\n M operational-history-1.db\n M node_modules/pkg/index.js\n M target/debug/app\n M .cache/tool/output.json\n M .turbo/cache.json\n M .next/cache/app\n M dist/app.js\n M build/app.js\n M .venv/pyvenv.cfg\n M venv/pyvenv.cfg\n M __pycache__/mod.pyc\n M .pytest_cache/v/cache/nodeids\n M .mypy_cache/module.json\n M .ruff_cache/module.json\n M .gradle/caches/module.bin\n M .m2/repository/artifact.jar\n M .pnpm-store/v3/files/index\n M src/lib.rs",
    );

    let change = tracked_workspace_live_sync_change_after_turn(&before, &after)
        .expect("allowed tracked path should remain");

    assert_eq!(change.changed_paths, vec!["src/lib.rs"]);
}

#[test]
fn tracked_workspace_live_sync_change_filters_charioxignore_patterns() {
    let root = std::env::temp_dir().join(format!(
        "chariox-tracked-sync-ignore-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(root.join("ignored")).expect("temp worktree should be created");
    std::fs::write(root.join(".gitignore"), "ignored/\n*.secret\n")
        .expect("gitignore should write");
    let mut before = tracked_snapshot(false, "");
    before.repo_root = root.display().to_string();
    before.worktree_path = root.display().to_string();
    let mut after = tracked_snapshot(true, " M ignored/file.txt\n M src/lib.rs\n?? token.secret");
    after.repo_root = root.display().to_string();
    after.worktree_path = root.display().to_string();

    let change = tracked_workspace_live_sync_change_after_turn(&before, &after)
        .expect("allowed tracked path should remain");

    assert_eq!(change.changed_paths, vec!["src/lib.rs"]);
    assert_eq!(
        std::fs::read_to_string(root.join(".charioxignore"))
            .expect(".charioxignore should initialize"),
        "ignored/\n*.secret\n"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn tracked_workspace_live_sync_change_initializes_empty_charioxignore_without_gitignore() {
    let root = std::env::temp_dir().join(format!(
        "chariox-tracked-sync-empty-ignore-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&root).expect("temp worktree should be created");
    let mut before = tracked_snapshot(false, "");
    before.repo_root = root.display().to_string();
    before.worktree_path = root.display().to_string();
    let mut after = tracked_snapshot(true, " M src/lib.rs\n?? token.secret");
    after.repo_root = root.display().to_string();
    after.worktree_path = root.display().to_string();

    let change = tracked_workspace_live_sync_change_after_turn(&before, &after)
        .expect("tracked paths should remain when no ignore file exists");

    assert_eq!(change.changed_paths, vec!["src/lib.rs", "token.secret"]);
    assert_eq!(
        std::fs::read_to_string(root.join(".charioxignore"))
            .expect(".charioxignore should initialize"),
        ""
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn tracked_workspace_live_sync_change_filters_renames_from_ignored_paths() {
    let root = std::env::temp_dir().join(format!(
        "chariox-tracked-sync-ignore-rename-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&root).expect("temp worktree should be created");
    std::fs::write(root.join(".charioxignore"), "ignored/\n").expect("ignore should write");
    let mut before = tracked_snapshot(false, "");
    before.repo_root = root.display().to_string();
    before.worktree_path = root.display().to_string();
    let mut after = tracked_snapshot(true, "R  ignored/old.txt -> src/new.txt\n M src/lib.rs");
    after.repo_root = root.display().to_string();
    after.worktree_path = root.display().to_string();

    let change = tracked_workspace_live_sync_change_after_turn(&before, &after)
        .expect("allowed tracked path should remain");

    assert_eq!(change.changed_paths, vec!["src/lib.rs"]);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn tracked_workspace_live_sync_change_captures_file_snapshots() {
    let root = std::env::temp_dir().join(format!(
        "chariox-tracked-sync-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(root.join("src")).expect("temp repo should be created");
    run_git(&root, &["init"]);
    run_git(&root, &["config", "user.email", "agent@example.com"]);
    run_git(&root, &["config", "user.name", "Agent"]);
    std::fs::write(root.join("src/lib.rs"), "pub fn old() {}\n")
        .expect("tracked file should write");
    std::fs::write(root.join("README.md"), "delete me\n").expect("delete file should write");
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "seed commit"]);

    let before = capture_turn_snapshot(GitTurnContext {
        workspace_live_sync_tracked: true,
        ..test_context(&root, "prompt-1")
    })
    .expect("pre-turn snapshot should capture");
    std::fs::write(root.join("src/lib.rs"), "pub fn new() {}\n")
        .expect("tracked file should update");
    std::fs::write(root.join("src/new.rs"), "pub fn added() {}\n").expect("new file should write");
    std::fs::remove_file(root.join("README.md")).expect("tracked file should delete");
    let after = capture_turn_snapshot(GitTurnContext {
        workspace_live_sync_tracked: true,
        ..test_context(&root, "prompt-1")
    })
    .expect("post-turn snapshot should capture");

    let change = tracked_workspace_live_sync_change_after_turn(&before, &after)
        .expect("tracked turn should produce file changes");

    assert_eq!(
        change.changed_paths,
        vec!["README.md", "src/lib.rs", "src/new.rs"]
    );
    assert_eq!(change.file_changes.len(), 3);
    let modified = change
        .file_changes
        .iter()
        .find(|change| change.path == "src/lib.rs")
        .expect("modified path should be present");
    let old_base64 = base64::engine::general_purpose::STANDARD.encode("pub fn old() {}\n");
    let new_base64 = base64::engine::general_purpose::STANDARD.encode("pub fn new() {}\n");
    assert_eq!(
        modified.before_content_base64.as_deref(),
        Some(old_base64.as_str())
    );
    assert_eq!(
        modified.after_content_base64.as_deref(),
        Some(new_base64.as_str())
    );
    let added = change
        .file_changes
        .iter()
        .find(|change| change.path == "src/new.rs")
        .expect("added path should be present");
    assert_eq!(added.before_content_base64, None);
    assert!(added.after_content_base64.is_some());
    let deleted = change
        .file_changes
        .iter()
        .find(|change| change.path == "README.md")
        .expect("deleted path should be present");
    assert!(deleted.before_content_base64.is_some());
    assert_eq!(deleted.after_content_base64, None);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn tracked_workspace_live_sync_change_skips_already_synced_status_lines() {
    let root = std::env::temp_dir().join(format!(
        "chariox-tracked-sync-delta-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(root.join("outputs")).expect("temp worktree should be created");
    let mut before = tracked_snapshot(false, " M tracked.txt\n?? outputs/existing.txt");
    before.repo_root = root.display().to_string();
    before.worktree_path = root.display().to_string();
    let mut after = tracked_snapshot(
        true,
        " M tracked.txt\n?? .charioxignore\n?? outputs/existing.txt",
    );
    after.repo_root = root.display().to_string();
    after.worktree_path = root.display().to_string();

    assert!(tracked_workspace_live_sync_change_after_turn(&before, &after).is_none());

    let mut after_new = tracked_snapshot(
        true,
        " M tracked.txt\n?? outputs/existing.txt\n?? outputs/new.txt",
    );
    after_new.repo_root = root.display().to_string();
    after_new.worktree_path = root.display().to_string();

    let change = tracked_workspace_live_sync_change_after_turn(&before, &after_new)
        .expect("new status lines should still fan out");
    assert_eq!(change.changed_paths, vec!["outputs/new.txt"]);

    let _ = std::fs::remove_dir_all(&root);
}
