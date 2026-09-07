use super::*;

struct EmptyWorkspace(std::path::PathBuf);

impl EmptyWorkspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "chariox-git-free-files-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir(&path).expect("new disposable workspace");
        Self(path)
    }
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
