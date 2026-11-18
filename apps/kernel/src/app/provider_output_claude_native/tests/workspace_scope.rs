//! A numbered trust selection the kernel did not start settles with a reason.
use super::*;

#[test]
fn invalid_startup_trust_selection_has_an_actionable_failure() {
    let root = crate::test_support::TestWorktree::new("invalid-trust-selection");
    let context = root.path().join("context.txt");
    fs::write(&context, "").unwrap();
    write_claude_native_marker(
        context.to_str().unwrap(),
        "startup-workspace-trust:select-yes:123",
    );
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let error = ProviderOutputClaudeNativeBridge::new(&mut app)
        .process_pending_claude_workspace_trust("run", context.to_str().unwrap())
        .expect_err("invalid selection state must settle with a reason");
    assert!(error.to_string().contains("selection state is invalid"));
    app.shutdown_cleanup().unwrap();
}
