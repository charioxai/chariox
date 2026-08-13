use super::*;

#[test]
fn shell_command_capability_runs_through_capability_boundary() {
    let worktree_root = std::env::temp_dir().join("chariox-shell-app-test");
    std::fs::create_dir_all(&worktree_root).expect("worktree dir should exist");
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            "workspace-1",
            worktree_root.display().to_string(),
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-shell",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let result = crate::capability::ShellCommandService::new()
        .run(crate::capability::RunShellCommandRequest::new(
            session.id(),
            attachment.id(),
            "/bin/sh",
            vec!["-lc".to_string(), "printf shell-app".to_string()],
            std::path::PathBuf::from(session.worktree_id()),
            None,
        ))
        .expect("shell capability should succeed");

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout, "shell-app");
}

#[test]
fn directory_tree_file_and_git_capabilities_run_through_capability_boundary() {
    let worktree_root = std::env::temp_dir().join("chariox-kernel-app-capability-test");
    let _ = std::fs::remove_dir_all(&worktree_root);
    std::fs::create_dir_all(worktree_root.join("src")).expect("worktree dir should exist");
    std::fs::write(worktree_root.join("README.md"), "hello").expect("file should exist");
    std::fs::write(worktree_root.join("src/lib.rs"), "before").expect("file should exist");
    std::process::Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(&worktree_root)
        .output()
        .expect("git init should work");

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            "workspace-1",
            worktree_root.display().to_string(),
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-capability",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let tree = crate::capability::DirectoryTreeService::new()
        .read_tree(crate::capability::ReadDirectoryTreeRequest::new(
            session.id(),
            attachment.id(),
            std::path::PathBuf::from(session.worktree_id()),
            None,
            2,
        ))
        .expect("tree read should succeed");
    let file = crate::capability::FileCapabilityService::new()
        .read_file(crate::capability::ReadFileRequest::new(
            session.id(),
            attachment.id(),
            std::path::PathBuf::from(session.worktree_id()),
            worktree_root.join("src/lib.rs"),
        ))
        .expect("file read should succeed");
    let _claim = app
        .workspace_coordinator()
        .acquire_worktree_write_claim(
            session.workspace_id().to_string(),
            session.worktree_id().to_string(),
            session.id().to_string(),
            Some(attachment.id().to_string()),
            "file_edit",
        )
        .expect("edit claim should be acquired");
    let edit = crate::capability::FileCapabilityService::new()
        .edit_file(crate::capability::EditFileRequest::new(
            session.id(),
            attachment.id(),
            std::path::PathBuf::from(session.worktree_id()),
            worktree_root.join("src/lib.rs"),
            "after".to_string(),
        ))
        .expect("file edit should succeed");
    let git = crate::capability::GitCapabilityService::new()
        .inspect(crate::capability::InspectGitRequest::new(
            session.id(),
            attachment.id(),
            std::path::PathBuf::from(session.worktree_id()),
            None,
        ))
        .expect("git inspect should succeed");

    assert!(tree
        .entries
        .iter()
        .any(|entry| entry.relative_path == "README.md"));
    assert_eq!(file.contents, "before");
    assert_eq!(edit.bytes_written, 5);
    assert_eq!(edit.old_size, 6);
    assert_eq!(edit.new_size, 5);
    assert!(edit.changed);
    assert_eq!(
        std::fs::read_to_string(worktree_root.join("src/lib.rs")).expect("file readable"),
        "after"
    );
    assert!(git.status.contains("main"));
}

#[test]
fn screenshot_capability_returns_structured_unavailable_result() {
    let _guard = crate::env_lock::lock();
    std::env::set_var("CHARIOX_SCREENSHOT_DISABLE", "1");
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            "workspace-1",
            std::env::temp_dir().display().to_string(),
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-screenshot",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let result = crate::capability::ScreenshotCapabilityService::new()
        .capture(crate::capability::CaptureScreenshotRequest::new(
            session.id(),
            attachment.id(),
            crate::app::attachment_artifact_root(session.id(), attachment.id(), "screenshots"),
        ))
        .expect("screenshot request should return structured result");
    std::env::remove_var("CHARIOX_SCREENSHOT_DISABLE");

    assert_eq!(
        result.status,
        crate::capability::ScreenshotStatus::Unavailable
    );
    assert!(result.artifact_path.is_none());
}

#[test]
fn transfer_capability_stores_artifact_under_session_root() {
    let worktree_root = std::env::temp_dir().join("chariox-transfer-app-test");
    let _ = std::fs::remove_dir_all(&worktree_root);
    std::fs::create_dir_all(&worktree_root).expect("worktree should exist");
    let source = worktree_root.join("artifact.txt");
    std::fs::write(&source, "artifact").expect("source should exist");
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            "workspace-1",
            worktree_root.display().to_string(),
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-transfer",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let _claim = app
        .workspace_coordinator()
        .acquire_worktree_write_claim(
            session.workspace_id().to_string(),
            session.worktree_id().to_string(),
            session.id().to_string(),
            Some(attachment.id().to_string()),
            "transfer_store",
        )
        .expect("transfer claim should be acquired");
    let result = crate::capability::FileTransferService::new()
        .store_file(crate::capability::StoreTransferredFileRequest::new(
            session.id(),
            attachment.id(),
            std::path::PathBuf::from(session.worktree_id()),
            crate::app::attachment_artifact_root(session.id(), attachment.id(), "transfers"),
            source,
            None,
        ))
        .expect("transfer should succeed");

    assert!(result
        .stored_path
        .to_string_lossy()
        .contains("chariox-session-artifacts"));
    assert_eq!(result.bytes, 8);
}
