//! MP-08 / MP-10 / MP-11: fail-first review regression.
use super::*;
#[cfg(target_os = "linux")]
#[test]
fn review0821_http_dns_is_owned_and_bounded() {
    use std::{
        os::unix::fs::PermissionsExt,
        time::{Duration, Instant},
    };
    if std::env::var_os("CHARIOX_REVIEW0821_DNS_CHILD").is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &format!(
                    "{}::review0821_http_dns_is_owned_and_bounded",
                    module_path!().split_once("::").unwrap().1
                ),
                "--test-threads=1",
            ])
            .env("CHARIOX_REVIEW0821_DNS_CHILD", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        return;
    }
    let root = crate::test_support::TestWorktree::new("review0821-dns");
    let utility = root.path().join("getent");
    std::fs::write(
        &utility,
        r#"#!/bin/sh
echo owned > "$0.started"
sleep .4
echo survived > "$0.survived"
"#,
    )
    .unwrap();
    std::fs::set_permissions(&utility, std::fs::Permissions::from_mode(0o700)).unwrap();
    let original = std::env::var_os("PATH").unwrap();
    let paths = std::env::join_paths(
        std::iter::once(root.path().to_path_buf()).chain(std::env::split_paths(&original)),
    )
    .unwrap();
    std::env::set_var("PATH", paths);
    let began = Instant::now();
    let result = inventory::health(
        &EnvironmentProbe::Http {
            url: "http://bounded.example.invalid/".into(),
            expected_status: 200,
            timeout_ms: 50,
        },
        &None,
        &[],
    );
    let elapsed = began.elapsed();
    std::env::set_var("PATH", original);
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        !root.path().join("getent.started").exists(),
        "HTTP must not execute an ambient PATH resolver"
    );
    assert!(
        !root.path().join("getent.survived").exists(),
        "resolver survived the probe deadline"
    );
    assert!(elapsed < Duration::from_millis(300));
    assert!(!matches!(result, Ok(true)));
}
