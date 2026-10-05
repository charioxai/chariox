#[path = "../../kernel/tests/support/cli_startup.rs"]
mod support;
use support::assert_no_startup;
#[test]
fn relay_help_has_no_runtime_side_effects() {
    for flag in ["--help", "-h"] {
        assert_no_startup(
            env!("CARGO_BIN_EXE_chariox-relay"),
            &[flag],
            true,
            "usage: chariox-relay",
        );
    }
}
#[test]
fn relay_rejects_unknown_arguments_before_startup() {
    for args in [
        vec!["--unknown"],
        vec!["unexpected"],
        vec!["--help", "--unknown"],
        vec!["--version", "extra"],
    ] {
        assert_no_startup(env!("CARGO_BIN_EXE_chariox-relay"), &args, false, "error:");
    }
}
