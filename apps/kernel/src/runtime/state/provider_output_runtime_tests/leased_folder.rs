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

async fn worker_runtime(
    root: &crate::test_support::TestWorktree,
    home: &str,
) -> (Arc<Mutex<DaemonApp>>, KernelRuntimeState) {
    let user_home = root.path().join("user");
    std::fs::create_dir_all(&user_home).unwrap();
    std::env::set_var(
        "HOME",
        home.replace("{root}", &user_home.display().to_string()),
    );
    std::env::set_current_dir("/").unwrap();
    let mut config = crate::DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    config.kernel_runtime_role = crate::config::KernelRuntimeRole::RemoteLeaseWorker;
    config.remote_lease_capacity = Some(4);
    let app = Arc::new(Mutex::new(DaemonApp::bootstrap(config).unwrap()));
    let runtime = owned_runtime_state(&app).await;
    (app, runtime)
}

fn caller() -> crate::app::LeaseCallerBinding {
    crate::app::LeaseCallerBinding {
        home_kernel_id: "home".to_string(),
        authenticated_machine_id: "home-machine".to_string(),
        owner_user_id: "owner".to_string(),
        realm_id: "realm".to_string(),
        public_key_thumbprint: "key".to_string(),
    }
}

#[tokio::test]
async fn a_spawn_that_finds_no_folder_does_not_use_up_the_lease_authorization() {
    crate::test_support::isolated_env_test!();
    let root = crate::test_support::TestWorktree::new("leased-folder-unresolved");
    // No usable home folder: the worker cannot make an agent folder.
    let (_app, runtime) = worker_runtime(&root, "{root}").await;
    // The kernel keeps its state under the real home; only the spawn sees the unusable one, so the
    // test needs no write access to `/`.
    std::env::set_var("HOME", "/");
    let lease = runtime
        .create_bound_relay_execution_lease(
            "home",
            "room",
            "agent-home-1",
            false,
            "owner",
            caller(),
        )
        .await
        .unwrap();
    runtime
        .authorize_relay_execution_lease_caller(&lease.id, caller())
        .await
        .unwrap();
    let spawn = |worktree: Option<String>| {
        runtime.create_relay_leased_agent(
            &lease.id, "dev-stub", "default", None, None, None, None, None, worktree, None,
        )
    };
    let refused = spawn(None).await.unwrap_err();
    assert!(refused.to_string().contains("--dir"), "{refused}");
    let explicit = root.path().join("explicit");
    std::fs::create_dir_all(&explicit).unwrap();
    spawn(Some(explicit.display().to_string()))
        .await
        .expect("the refused spawn left the authorization for the retry");
}

#[tokio::test]
async fn a_second_lease_of_one_home_agent_does_not_share_the_first_ones_folder() {
    crate::test_support::isolated_env_test!();
    let root = crate::test_support::TestWorktree::new("leased-folder-reuse");
    let (app, runtime) = worker_runtime(&root, "{root}").await;
    let mut folders = Vec::new();
    for attempt in 0..2 {
        let lease = runtime
            .create_bound_relay_execution_lease(
                "home",
                "room",
                "agent-home-1",
                false,
                "owner",
                caller(),
            )
            .await
            .unwrap();
        runtime
            .authorize_relay_execution_lease_caller(&lease.id, caller())
            .await
            .unwrap();
        let spawned = runtime
            .create_relay_leased_agent(
                &lease.id, "dev-stub", "default", None, None, None, None, None, None, None,
            )
            .await;
        if attempt == 0 {
            let leased = spawned.expect("the first lease gets a folder");
            let session = app
                .lock()
                .await
                .sessions()
                .get_session(&leased.backing_session_id)
                .unwrap();
            folders.push(session.worktree_id().to_string());
        } else {
            let refused = spawned.expect_err("the folder of the first agent is taken");
            assert!(
                refused.to_string().contains("already exists")
                    && refused.to_string().contains("--dir"),
                "{refused}"
            );
        }
    }
    assert!(std::path::Path::new(&folders[0]).is_dir());
}

#[tokio::test]
async fn a_refused_leased_spawn_removes_the_folder_it_made() {
    crate::test_support::isolated_env_test!();
    let root = crate::test_support::TestWorktree::new("leased-folder-refused");
    let (_app, runtime) = worker_runtime(&root, "{root}").await;
    let lease = runtime
        .create_bound_relay_execution_lease(
            "home",
            "room",
            "agent-home-1",
            false,
            "owner",
            caller(),
        )
        .await
        .unwrap();
    runtime
        .authorize_relay_execution_lease_caller(&lease.id, caller())
        .await
        .unwrap();
    let refused = runtime
        .create_relay_leased_agent(
            &lease.id,
            "no-such-provider",
            "default",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
    assert!(refused.is_err(), "{refused:?}");
    let folder = root
        .path()
        .join("user")
        .join("chariox-workspaces")
        .join("agent-home-1");
    assert!(!folder.exists(), "the refused spawn left {folder:?}");
}
