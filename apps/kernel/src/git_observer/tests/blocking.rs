use std::os::unix::fs::PermissionsExt;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::super::capture_turn_snapshot;
use super::support::{run_git, test_context};

#[test]
fn slow_git_snapshots_leave_async_workers_available() {
    let root = std::env::temp_dir().join(format!(
        "chariox-git-observer-blocking-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&root).expect("temp repo should be created");
    run_git(&root, &["init"]);
    run_git(&root, &["config", "user.email", "test@example.com"]);
    run_git(&root, &["config", "user.name", "Test"]);
    std::fs::write(root.join("README.md"), "seed\n").unwrap();
    run_git(&root, &["add", "README.md"]);
    run_git(&root, &["commit", "-m", "seed"]);

    let hook = root.join(".git/held-fsmonitor");
    std::fs::write(
        &hook,
        "#!/bin/sh\n: > .git/entered-$$\ni=0\nwhile [ ! -f .git/release-monitor ] && [ \"$i\" -lt 1000 ]; do sleep 0.01; i=$((i + 1)); done\nprintf 'next-token\\000'\n",
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    run_git(&root, &["config", "core.fsmonitor", hook.to_str().unwrap()]);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let snapshots: Vec<_> = (0..2)
        .map(|i| {
            let context = test_context(&root, &format!("prompt-{i}"));
            runtime.spawn(async move { capture_turn_snapshot(context) })
        })
        .collect();
    let deadline = Instant::now() + Duration::from_secs(10);
    let entered = loop {
        let entered = std::fs::read_dir(root.join(".git"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("entered-"))
            .count();
        if entered == 2 || Instant::now() >= deadline {
            break entered == 2;
        }
        std::thread::sleep(Duration::from_millis(10));
    };

    let (send, receive) = mpsc::channel();
    let probe = runtime.spawn(async move { send.send(()).unwrap() });
    let responsive = entered && receive.recv_timeout(Duration::from_secs(2)).is_ok();

    // Release both real Git processes before asserting, including on the red path.
    std::fs::write(root.join(".git/release-monitor"), "").unwrap();
    let captured = runtime.block_on(async {
        let mut captured = true;
        for snapshot in snapshots {
            captured &= snapshot.await.unwrap().is_some();
        }
        probe.await.unwrap();
        captured
    });
    drop(runtime);
    std::fs::remove_dir_all(root).unwrap();

    assert!(
        entered,
        "both Git status commands should reach the held monitor"
    );
    assert!(captured, "both snapshots should preserve their Git results");
    assert!(responsive, "slow Git snapshots occupied every async worker");
}
