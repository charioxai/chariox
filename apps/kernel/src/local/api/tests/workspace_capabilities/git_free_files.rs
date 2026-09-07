use super::*;

struct EmptyWorkspace(std::path::PathBuf);

impl EmptyWorkspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "chariox-git-free-files-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&path).expect("new disposable workspace");
        Self(path)
    }
}

#[test]
#[cfg(unix)]
fn local_request_api_git_free_files_preserve_bounds_and_fingerprints() {
    run_workspace_capability_test("git_free_workspace_bounds", || {
        let root = EmptyWorkspace::new();
        let outside = EmptyWorkspace::new();
        std::fs::create_dir(root.0.join("folder")).unwrap();
        std::fs::write(root.0.join("z.txt"), "abcdef").unwrap();
        std::fs::write(outside.0.join("private.txt"), "outside").unwrap();
        std::os::unix::fs::symlink(&outside.0, root.0.join("escape")).unwrap();
        let path = root.0.display().to_string();
        let harness = LocalRouterTestHarness::new();
        let list = |prefix: &str| {
            LocalDaemonRequest::ListWorkspaceFiles(ListWorkspaceFilesRequest {
                workspace_id: path.clone(),
                worktree_id: path.clone(),
                path_prefix: Some(prefix.to_string()),
                compare_ref: None,
                limit: Some(1),
            })
        };
        let response = harness.dispatch(list("")).unwrap();
        let LocalDaemonResponse::WorkspaceFilesListed { listing } = response else {
            panic!("listing")
        };
        assert_eq!(
            listing.total_entries, 2,
            "escaping symlink must not be listed"
        );
        assert!(listing.truncated);
        assert_eq!(listing.entries.len(), 1);
        assert_eq!(
            listing.entries[0].name, "folder",
            "directories sort before files"
        );
        for prefix in ["..", "folder/../..", "/", "escape"] {
            assert!(harness.dispatch(list(prefix)).is_err(), "accepted {prefix}");
        }
        let read = |file: &str, known: Option<String>| {
            LocalDaemonRequest::GetWorkspaceFileContent(GetWorkspaceFileContentRequest {
                workspace_id: path.clone(),
                worktree_id: path.clone(),
                path: file.to_string(),
                compare_ref: None,
                known_fingerprint: known,
                max_bytes: Some(3),
            })
        };
        for file in [
            "../private.txt",
            "escape/private.txt",
            "folder",
            "/etc/passwd",
        ] {
            assert!(
                harness.dispatch(read(file, None)).is_err(),
                "accepted {file}"
            );
        }
        let LocalDaemonResponse::WorkspaceFileContent { content } =
            harness.dispatch(read("z.txt", None)).unwrap()
        else {
            panic!("content")
        };
        assert_eq!(content.content_text.as_deref(), Some("abc"));
        assert!(content.truncated);
        assert!(matches!(
            harness
                .dispatch(read("z.txt", Some(content.fingerprint)))
                .unwrap(),
            LocalDaemonResponse::WorkspaceFileContentNotModified { .. }
        ));
        std::fs::write(root.0.join(".git"), "gitdir: /missing-chariox-git-dir\n").unwrap();
        assert!(
            harness.dispatch(list("")).is_err(),
            "broken Git metadata must not trigger fallback"
        );
        assert!(harness.dispatch(read("z.txt", None)).is_err());
    });
}

impl Drop for EmptyWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn local_request_api_browses_git_free_workspace_artifacts() {
    run_workspace_capability_test("git_free_workspace_artifacts", || {
        let root = EmptyWorkspace::new();
        std::fs::create_dir(root.0.join("artifacts")).unwrap();
        std::fs::write(root.0.join("artifacts/fix.patch"), "patch contents\n").unwrap();
        let path = root.0.display().to_string();
        let harness = LocalRouterTestHarness::new();
        let response = harness
            .dispatch(LocalDaemonRequest::ListWorkspaceFiles(
                ListWorkspaceFilesRequest {
                    workspace_id: path.clone(),
                    worktree_id: path.clone(),
                    path_prefix: Some("artifacts".to_string()),
                    compare_ref: None,
                    limit: None,
                },
            ))
            .expect("ordinary files must not require Git");
        let LocalDaemonResponse::WorkspaceFilesListed { listing } = response else {
            panic!("expected workspace files");
        };
        assert_eq!(listing.total_entries, 1);
        assert_eq!(listing.entries[0].path, "artifacts/fix.patch");
        assert!(!listing.entries[0].changed);
        assert_eq!(listing.compare_ref, "");
        let response = harness
            .dispatch(LocalDaemonRequest::GetWorkspaceFileContent(
                GetWorkspaceFileContentRequest {
                    workspace_id: path.clone(),
                    worktree_id: path,
                    path: "artifacts/fix.patch".to_string(),
                    compare_ref: None,
                    known_fingerprint: None,
                    max_bytes: None,
                },
            ))
            .expect("artifact content must not require Git");
        let LocalDaemonResponse::WorkspaceFileContent { content } = response else {
            panic!("expected workspace content");
        };
        assert_eq!(content.content_text.as_deref(), Some("patch contents\n"));
        assert_eq!(content.status, None);
        assert_eq!(content.compare_ref, "");
        assert!(
            !root.0.join(".git").exists(),
            "browsing must not initialize Git"
        );
    });
}
