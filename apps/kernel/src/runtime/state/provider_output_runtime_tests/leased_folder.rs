// MP-08 / MP-11: a leased agent spawned with no folder never runs in the worker's service
// directory (`/` under a service manager); it gets a fresh folder of its own.
use super::*;

#[tokio::test]
async fn a_leased_agent_without_a_folder_runs_in_a_fresh_folder_of_its_own() {
    crate::test_support::isolated_env_test!();
    let root = crate::test_support::TestWorktree::new("leased-folder");
    let user_home = root.path().join("user");
    std::fs::create_dir_all(&user_home).unwrap();
    std::env::set_var("HOME", &user_home);
    std::env::set_current_dir("/").unwrap();
    let mut config = crate::DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let app = Arc::new(Mutex::new(DaemonApp::bootstrap(config).unwrap()));
    let runtime = owned_runtime_state(&app).await;
    let lease = runtime
        .create_relay_execution_lease("home", "room", "agent-home-1", false, "owner")
        .await
        .unwrap();
    let spawn = |worktree: Option<String>| {
        runtime.create_relay_leased_agent(
            &lease.id, "dev-stub", "default", None, None, None, None, None, worktree, None,
        )
    };
    let leased = spawn(None).await.unwrap();
    let session = app
        .lock()
        .await
        .sessions()
        .get_session(&leased.backing_session_id)
        .unwrap();
    let folder = std::path::Path::new(session.worktree_id());
    assert_ne!(folder, std::path::Path::new("/"), "the agent runs in /");
    assert!(folder.is_dir(), "{folder:?}");
    assert!(
        folder.starts_with(&user_home) && folder.ends_with("agent-home-1"),
        "a folder of its own under the worker's home: {folder:?}"
    );
    // An explicit folder is still used as given.
    let explicit = user_home.join("explicit");
    std::fs::create_dir_all(&explicit).unwrap();
    let leased = spawn(Some(explicit.display().to_string())).await.unwrap();
    let session = app
        .lock()
        .await
        .sessions()
        .get_session(&leased.backing_session_id)
        .unwrap();
    assert_eq!(session.worktree_id(), explicit.display().to_string());
}
