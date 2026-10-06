#[path = "support/cli_startup.rs"]
mod support;
use support::assert_no_startup;

#[test]
fn kernel_help_has_no_runtime_side_effects() {
    for flag in ["--help", "-h"] {
        assert_no_startup(
            env!("CARGO_BIN_EXE_chariox-kernel"),
            &[flag],
            true,
            "usage: chariox-kernel",
        );
    }
}
#[test]
fn kernel_rejects_unknown_and_malformed_arguments_before_startup() {
    for args in [
        vec!["--unknown"],
        vec!["unexpected"],
        vec!["--help", "--unknown"],
        vec!["--version", "extra"],
        vec!["--print-local-daemon-protocol-version", "extra"],
        vec!["--owner-managed-enroll-stdin", "unexpected"],
        vec!["--owner-managed-ready", "unexpected"],
        vec!["--prepare-protected-slice-identity"],
        vec!["--prepare-protected-slice-identity", "0"],
        vec!["--prepare-protected-slice-identity", "12345", "extra"],
    ] {
        assert_no_startup(env!("CARGO_BIN_EXE_chariox-kernel"), &args, false, "error:");
    }
}
#[test]
fn bootstrap_help_has_no_runtime_side_effects() {
    for flag in ["--help", "-h"] {
        assert_no_startup(
            env!("CARGO_BIN_EXE_chariox-managed-bootstrap"),
            &[flag],
            true,
            "usage: chariox-managed-bootstrap",
        );
    }
}
#[test]
fn bootstrap_rejects_unknown_arguments_before_startup() {
    for args in [
        vec!["--unknown"],
        vec!["unexpected"],
        vec!["--disposable-worker", "--unknown"],
        vec!["--version", "extra"],
        vec!["--help", "extra"],
    ] {
        assert_no_startup(
            env!("CARGO_BIN_EXE_chariox-managed-bootstrap"),
            &args,
            false,
            "error:",
        );
    }
}
#[test]
fn cli_launcher_help_needs_no_bun_or_runtime_state() {
    for args in [
        vec!["--help"],
        vec!["-h"],
        vec!["serve", "--help"],
        vec!["serve", "-h"],
    ] {
        assert_no_startup(
            env!("CARGO_BIN_EXE_chariox-cli"),
            &args,
            true,
            "usage: chariox",
        );
    }
}
#[test]
fn cli_launcher_rejects_unknown_arguments_before_building_or_logging() {
    for args in [
        vec!["--unknown"],
        vec!["unexpected"],
        vec!["--workspace", "/tmp", "--unknown"],
        vec!["--help", "--unknown"],
        vec!["--workspace"],
        vec!["serve", "--unknown"],
        vec!["serve", "--help", "--unknown"],
        vec!["serve", "source", "session-1", "publication-1", "--unknown"],
        vec![
            "serve",
            "source",
            "session-1",
            "publication-1",
            "12345",
            "--help",
            "--unknown",
        ],
        vec![
            "serve",
            "source",
            "session-1",
            "publication-1",
            "12345",
            "--host",
            "--unknown",
        ],
    ] {
        assert_no_startup(env!("CARGO_BIN_EXE_chariox-cli"), &args, false, "error:");
    }
}
