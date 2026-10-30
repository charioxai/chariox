//! MP-08 / MP-10 / MP-11: real Git metadata and a supplementary owned I/O stall.
use super::*;
fn repository() -> crate::test_support::TestWorktree {
    let root = crate::test_support::TestWorktree::new("waiting-room-git-metadata");
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
    git(&[
        "remote",
        "add",
        "origin",
        "https://github.com/charioxai/chariox.git",
    ]);
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
    root
}
#[test]
fn waiting_room_metadata_preserves_real_launch_and_label_values() {
    let root = repository();
    let path = root.path().to_string_lossy().into_owned();
    let cache = Cache::default();
    let launch = cache.prime(Query::Launch(path.clone()));
    let Some(Value::Launch(Some(launch))) = launch else {
        panic!("real launch target unavailable")
    };
    assert_eq!(launch.workspace_id, path);
    assert_eq!(launch.worktree_id, path);
    assert_eq!(launch.workspace_label.as_deref(), Some("charioxai/chariox"));
    assert_eq!(launch.worktree_label.as_deref(), Some("main"));
    assert!(
        cache.prime(Query::Workspace(path.clone()))
            == Some(Value::Label(Some("charioxai/chariox".into())))
    );
    assert!(
        cache.prime(Query::Worktree {
            worktree: path.clone(),
            workspace: path
        }) == Some(Value::Label(Some("main".into())))
    );
}
#[cfg(unix)]
#[test]
fn waiting_room_metadata_retains_labels_and_coalesces_a_stalled_refresh() {
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    let root = repository();
    let state = crate::test_support::TestWorktree::new("waiting-room-label-state");
    let cache = Arc::new(Cache::default());
    let query = Query::Workspace(root.path().to_string_lossy().into_owned());
    let expected = Some(Value::Label(Some("charioxai/chariox".into())));
    assert!(cache.prime(query.clone()) == expected);
    let revision = cache.revision.load(Ordering::Acquire);
    cache
        .entries
        .lock()
        .unwrap()
        .get_mut(&query)
        .unwrap()
        .expires = Instant::now();
    let marker = state.path().join("worker.pid");
    std::env::set_var("CHARIOX_WORKSPACE_METADATA_STALL_MARKER", &marker);
    let began = Instant::now();
    assert!(cache.get(query.clone()) == expected);
    assert!(
        began.elapsed() < Duration::from_millis(500),
        "Git refresh blocked the projection caller"
    );
    let until = Instant::now() + Duration::from_secs(5);
    while !marker.exists() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
    let pid = std::fs::read_to_string(&marker).expect("owned label worker reached the stall");
    for _ in 0..5 {
        assert!(cache.get(query.clone()) == expected);
    }
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        std::fs::read_to_string(&marker).unwrap(),
        pid,
        "polls launched duplicate refreshes"
    );
    while cache
        .entries
        .lock()
        .unwrap()
        .get(&query)
        .unwrap()
        .refreshing
        && Instant::now() < until
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !cache
            .entries
            .lock()
            .unwrap()
            .get(&query)
            .unwrap()
            .refreshing
    );
    assert!(
        cache.get(query) == expected,
        "a failed probe discarded last-known labels"
    );
    assert_eq!(
        cache.revision.load(Ordering::Acquire),
        revision,
        "failed refresh changed the public projection generation"
    );
    let pid = pid.parse::<i32>().unwrap();
    assert!(pid > 1 && pid != std::process::id() as i32);
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
    std::env::remove_var("CHARIOX_WORKSPACE_METADATA_STALL_MARKER");
}

// MP-08 / MP-10: exercise both real public projection cache fast paths.
#[cfg(unix)]
#[test]
fn review2018_cached_snapshots_retry_initial_timeouts_and_capacity_skips() {
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    use crate::runtime::waiting_room_public_projection::{
        build_waiting_room_public_snapshot_from_cached_shared,
        WaitingRoomSessionSummaryProjectionStore,
    };
    let root = repository();
    let state = crate::test_support::TestWorktree::new("review2018-label-state");
    let path = root.path().to_str().unwrap();
    let session = Arc::new(crate::session::RuntimeSession::new(
        "session", None, path, path, "machine", "daemon",
    ));
    let store = WaitingRoomSessionSummaryProjectionStore::default();
    let events = crate::runtime::metaagent_event::MetaagentEventStore::default();
    let snapshot = || {
        build_waiting_room_public_snapshot_from_cached_shared(
            std::slice::from_ref(&session),
            1,
            &store,
            &events,
            0,
            &Default::default(),
            &[],
            &[],
            vec![],
            false,
            None,
            crate::local::RelayStatus {
                runtime_process_identity: None,
                capabilities: vec![],
                configured: false,
                connected: false,
                relay_url: None,
                relay_token_configured: false,
                daemon_id: "daemon".into(),
                daemon_alias: None,
                machine_id: "machine".into(),
                machine_alias: None,
            },
            vec![],
            vec![],
            vec![],
            1,
            crate::session::DEFAULT_LOCAL_USER_ID,
        )
        .unwrap()
    };
    let marker = state.path().join("worker.pid");
    std::env::set_var("CHARIOX_WORKSPACE_METADATA_STALL_MARKER", &marker);
    let first = snapshot();
    assert_eq!(first.sessions[0].workspace_label, None);
    let until = Instant::now() + Duration::from_secs(8);
    loop {
        if marker.exists()
            && cache()
                .entries
                .lock()
                .unwrap()
                .values()
                .all(|e| !e.refreshing)
        {
            break;
        }
        assert!(
            Instant::now() < until,
            "initial owned workers did not settle"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    // The real worker has timed out. Advance only the cache TTL, then poll
    // exactly the same sessions/revisions/auxiliary data as an idle client.
    std::env::remove_var("CHARIOX_WORKSPACE_METADATA_STALL_MARKER");
    for e in cache().entries.lock().unwrap().values_mut() {
        e.expires = Instant::now();
    }
    let until = Instant::now() + Duration::from_secs(8);
    loop {
        let current = snapshot();
        if current.sessions[0].workspace_label.as_deref() == Some("charioxai/chariox")
            && current.sessions[0].worktree_label.as_deref() == Some("main")
            && current.launch_target.is_some()
        {
            break;
        }
        assert!(
            Instant::now() < until,
            "cached project_snapshot stranded expired or capacity-skipped labels"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

// MP-08 / MP-10: a metadata-only completion must wake idle/resumed streams,
// without manufacturing a session mutation, and cannot lose an early wakeup.
#[tokio::test(flavor = "current_thread")]
async fn review2018_metadata_completion_wakes_inventory_without_session_mutation() {
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    let root = crate::test_support::TestWorktree::new("review2018-inventory-wake");
    std::env::set_var("CHARIOX_HOME", root.path());
    let app = crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
        Arc::new(tokio::sync::Mutex::new(app)),
        1,
    );
    let runtime = router.runtime_state();
    let sessions = runtime.session_projection_change_sequence();
    let query = Query::Workspace(root.path().to_string_lossy().into_owned());
    assert!(cache().begin(&query).1);
    let before = runtime.waiting_room_change_sequence();
    let wait = runtime.wait_for_waiting_room_change_after(before);
    tokio::pin!(wait);
    tokio::select! {
        _ = &mut wait => panic!("inventory woke before any projection change"),
        _ = tokio::time::sleep(Duration::from_millis(20)) => {},
    }
    cache().finish(
        query.clone(),
        Some(Value::Label(Some("charioxai/chariox".into()))),
    );
    assert_ne!(
        runtime.waiting_room_change_sequence(),
        before,
        "metadata completion must change the shared inventory clock"
    );
    tokio::time::timeout(Duration::from_secs(1), &mut wait)
        .await
        .expect("metadata completion wakes an already waiting stream");
    assert_eq!(runtime.session_projection_change_sequence(), sessions);
    let before = runtime.waiting_room_change_sequence();
    cache()
        .entries
        .lock()
        .unwrap()
        .get_mut(&query)
        .unwrap()
        .expires = Instant::now();
    assert!(cache().begin(&query).1);
    cache().finish(query, Some(Value::Label(Some("charioxai/renamed".into()))));
    tokio::time::timeout(
        Duration::from_secs(1),
        runtime.wait_for_waiting_room_change_after(before),
    )
    .await
    .expect("a completion before registration cannot be lost");
    assert_eq!(runtime.session_projection_change_sequence(), sessions);
}
