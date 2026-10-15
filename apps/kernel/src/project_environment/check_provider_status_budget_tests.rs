//! MP-08 / MP-10 / MP-11: remaining account budget.
use super::*;
#[cfg(unix)]
#[test]
fn check_provider_status_budget_check_settles_with_remaining_budget() {
    use std::{os::unix::fs::PermissionsExt, time::Duration};
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    let root = crate::test_support::TestWorktree::new("review1118-account");
    std::env::set_var("CHARIOX_HOME", root.path());
    let bin = root.path().join("claude");
    std::fs::write(
        &bin,
        r#"#!/bin/sh
if [ "$1" = '--version' ]; then : > "$0.version-probed"; sleep 1.2; echo 2.1.0; else : > "$0.status-probed"; sleep .08; echo '{"loggedIn":true}'; fi
"#,
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::env::set_var("CHARIOX_CLAUDE_BIN", &bin);
    let ctx = context(root.path());
    let profile = ctx
        .profiles
        .create_managed("owner", "claude", "budget")
        .unwrap();
    let mut r = requirement("account");
    r.spec=serde_json::from_value(serde_json::json!({"kind":"accounts","provider":"claude","linked_profile_ref":profile.profile_id,"required_capabilities":[]})).unwrap();
    let (observation, elapsed, setup_elapsed) =
        check_requirement_timed(root.path(), ctx, r, Duration::from_millis(600));
    assert!(
        elapsed < Duration::from_millis(900),
        "Check runner/provider consumed more than remaining budget: {elapsed:?}; fixture setup {setup_elapsed:?} excluded"
    );
    assert!(
        bin.with_extension("status-probed").exists(),
        "official status command was reached"
    );
    assert!(
        bin.with_extension("version-probed").exists(),
        "the deliberately stalled version command was reached"
    );
    assert_eq!(observation.reason_code, "check_budget_exceeded");
}

#[cfg(unix)]
#[test]
fn review1118_codex_check_uses_bounded_official_status() {
    use std::{os::unix::fs::PermissionsExt, time::Duration};
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    let root = crate::test_support::TestWorktree::new("review1118-codex-budget");
    std::env::set_var("CHARIOX_HOME", root.path());
    let bin = root.path().join("codex");
    std::fs::write(&bin, r#"#!/bin/sh
if [ "$1" = '--version' ]; then : > "$0.version-probed"; sleep 1.2; echo 0.120.0; elif [ "$1" = 'login' ] && [ "$2" = 'status' ]; then : > "$0.status-probed"; sleep .08; echo 'Logged in using ChatGPT'; else exit 2; fi
"#).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::env::set_var("CHARIOX_CODEX_BIN", &bin);
    let ctx = context(root.path());
    let profile = ctx
        .profiles
        .create_managed("owner", "codex", "budget")
        .unwrap();
    let mut r = requirement("account");
    r.spec = serde_json::from_value(serde_json::json!({"kind":"accounts","provider":"codex","linked_profile_ref":profile.profile_id,"required_capabilities":[]})).unwrap();
    let (observation, elapsed, setup_elapsed) =
        check_requirement_timed(root.path(), ctx, r, Duration::from_millis(600));
    assert!(elapsed < Duration::from_millis(900), "Check runner/provider consumed more than remaining budget: {elapsed:?}; fixture setup {setup_elapsed:?} excluded");
    assert!(
        bin.with_extension("status-probed").exists(),
        "official status command was reached"
    );
    assert!(
        bin.with_extension("version-probed").exists(),
        "the deliberately stalled version command was reached"
    );
    assert_eq!(observation.reason_code, "check_budget_exceeded");
}
