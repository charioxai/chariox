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
