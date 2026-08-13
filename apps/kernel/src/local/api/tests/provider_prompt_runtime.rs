use super::*;

fn assert_native_tui_config_error(error: DaemonError, operation: &'static str) {
    match error {
        DaemonError::LocalTransport {
            operation: actual,
            message,
        } => {
            assert_eq!(actual, operation);
            assert!(message.contains("provider-native TUI"));
        }
        other => panic!("unexpected error: {other}"),
    }
}

fn temp_git_repo(label: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "chariox-{label}-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    fs::create_dir_all(&root).expect("temp repo should be created");
    run_git(&root, &["init"]);
    run_git(&root, &["config", "user.email", "tests@example.invalid"]);
    run_git(&root, &["config", "user.name", "Chariox Tests"]);
    fs::write(root.join("README.md"), "turn actions seed\n").expect("seed file should be written");
    run_git(&root, &["add", "README.md"]);
    run_git(&root, &["commit", "-m", "initial"]);
    root
}

fn run_with_large_test_stack(name: &'static str, test: fn()) {
    std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(test)
        .unwrap_or_else(|error| panic!("{name} test thread should spawn: {error}"))
        .join()
        .unwrap_or_else(|error| std::panic::resume_unwind(error));
}

fn run_git(cwd: &std::path::Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git command should run");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_output(cwd: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git command should run");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

mod prompt_routing;
mod queue_and_notices;
mod turn_actions;
