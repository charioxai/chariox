//! MP-08 / MP-10 / MP-11: owned MCP resolver.
use super::*;
#[cfg(target_os = "linux")]
#[test]
fn review1118_mcp_check_owns_stalled_resolver() {
    use std::{
        os::unix::fs::PermissionsExt,
        time::{Duration, Instant},
    };
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    let root = crate::test_support::TestWorktree::new("review1118-dns");
    std::env::set_var("CHARIOX_HOME", root.path());
    std::env::set_var("NO_PROXY", "*");
    let bin = root.path().join("getent");
    std::fs::write(
        &bin,
        r#"#!/bin/sh
echo started > "$0.started"
sleep 4
echo survived > "$0.survived"
echo '127.0.0.1 STREAM'
"#,
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
    let old = std::env::var_os("PATH").unwrap();
    std::env::set_var(
        "PATH",
        std::env::join_paths(
            std::iter::once(root.path().to_path_buf()).chain(std::env::split_paths(&old)),
        )
        .unwrap(),
    );
    let ctx = context(root.path());
    let r = mcp_requirement("http://stalled.example.invalid/mcp");
    let began = Instant::now();
    // MP-08 / MP-10 / MP-11: include the new one-second lease-release
    // reserve; the stalled DNS dispatch retains its one-second timeout.
    let observation = check_requirement(root.path(), ctx, r, Duration::from_secs(5));
    let elapsed = began.elapsed();
    std::env::set_var("PATH", old);
    std::thread::sleep(Duration::from_millis(700));
    assert!(
        root.path().join("getent.started").exists(),
        "MCP must use owned bounded OS resolution"
    );
    assert!(!root.path().join("getent.survived").exists());
    assert!(elapsed < Duration::from_secs(2), "{elapsed:?}");
    assert_ne!(observation.status, EnvironmentObservationStatus::Ready);
}
