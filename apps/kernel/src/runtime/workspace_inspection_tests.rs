//! MP-08 / MP-10 / MP-11: supplementary supervision and real Git regression checks.
use super::*;

fn queries(root: &std::path::Path) -> Vec<Inspection> {
    let workspace = root.to_string_lossy().into_owned();
    vec![
        Inspection::Worktrees {
            workspace: workspace.clone(),
            current: None,
        },
        Inspection::GitOverview(GetWorkspaceGitOverviewRequest {
            workspace_id: workspace.clone(),
            worktree_id: workspace.clone(),
            compare_ref: None,
        }),
        Inspection::Files(ListWorkspaceFilesRequest {
            workspace_id: workspace.clone(),
            worktree_id: workspace,
            path_prefix: None,
            compare_ref: None,
            limit: None,
        }),
    ]
}

#[cfg(unix)]
#[test]
fn workspace_metadata_stalls_keep_the_runtime_live_and_reap_the_owned_child() {
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    let root = crate::test_support::TestWorktree::new("workspace-metadata-budget");
    let state = crate::test_support::TestWorktree::new("workspace-private-state");
    let marker = state.path().join("worker.pid");
    std::env::set_var("CHARIOX_WORKSPACE_METADATA_STALL_MARKER", &marker);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    for request in queries(root.path()) {
        let began = Instant::now();
        runtime.block_on(async {
            let ticker = tokio::spawn(async {
                tokio::time::sleep(Duration::from_millis(50)).await;
                Instant::now()
            });
            let result =
                inspect_until(state.path().into(), request, began + Duration::from_secs(5)).await;
            assert!(
                result.is_err(),
                "an unreachable metadata query must settle its deadline"
            );
            assert!(
                ticker.await.unwrap().duration_since(began) < Duration::from_secs(1),
                "filesystem/Git I/O blocked the async runtime"
            );
        });
        assert!(began.elapsed() < Duration::from_secs(7));
        let pid = std::fs::read_to_string(&marker)
            .expect("owned worker reached the stalled metadata call")
            .parse::<i32>()
            .unwrap();
        assert!(pid > 1 && pid != std::process::id() as i32);
        assert_eq!(
            unsafe { libc::kill(pid, 0) },
            -1,
            "owned interruptible child was not reaped"
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
        assert!(!std::fs::read_dir(state.path()).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("files-check-")));
    }
    std::env::remove_var("CHARIOX_WORKSPACE_METADATA_STALL_MARKER");
}

#[test]
fn workspace_metadata_workers_preserve_real_git_views() {
    let root = crate::test_support::TestWorktree::new("workspace-metadata-git");
    let state = crate::test_support::TestWorktree::new("workspace-metadata-private-state");
    let git = |args: &[&str]| {
        assert!(std::process::Command::new("git")
            .args(args)
            .current_dir(root.path())
            .output()
            .unwrap()
            .status
            .success())
    };
    git(&["init", "-b", "main"]);
    std::fs::write(root.path().join("README.md"), "public documentation\n").unwrap();
    git(&["add", "README.md"]);
    git(&[
        "-c",
        "user.name=Regression",
        "-c",
        "user.email=regression@example.invalid",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-m",
        "Public documentation",
    ]);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    for request in queries(root.path()) {
        match runtime
            .block_on(inspect(state.path().into(), request))
            .unwrap()
        {
            LocalDaemonResponse::WorkspaceWorktreesListed { worktrees, .. } => {
                assert_eq!(worktrees.len(), 1);
                assert!(worktrees[0].current);
                assert_eq!(worktrees[0].branch.as_deref(), Some("main"));
            }
            LocalDaemonResponse::WorkspaceGitOverview { overview } => {
                assert!(overview.repo_root.is_some())
            }
            LocalDaemonResponse::WorkspaceFilesListed { listing } => {
                assert!(listing.entries.iter().any(|e| e.name == "README.md"))
            }
            _ => panic!("unexpected workspace metadata response"),
        }
    }
}
