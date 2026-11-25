//! Row 550: the shell of the owner's own terminal is the owner: what runs in it needs no access request.
//! A process the kernel launched for anything else still needs its own grant.
use super::*;

fn read_pid(path: &std::path::Path) -> Option<u32> {
    for _ in 0..100 {
        if let Some(pid) = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| text.trim().parse().ok())
        {
            return Some(pid);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    None
}

fn stop(pid: u32) {
    assert!(pid > 1, "refusing to signal pid {pid}");
    let _ = std::process::Command::new("kill")
        .arg(pid.to_string())
        .status();
}

#[tokio::test]
async fn a_program_in_the_owners_terminal_shell_is_covered_and_another_kernel_child_is_not() {
    let app =
        crate::test_support::bootstrap_authenticated_app(crate::config::DaemonConfig::for_tests())
            .unwrap();
    let state = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
        std::sync::Arc::new(tokio::sync::Mutex::new(app)),
        32,
    )
    .runtime_state();
    let dir = std::env::temp_dir().join(format!(
        "chariox-owner-shell-{:016x}",
        rand::random::<u64>()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    // The terminal's shell is a child of the kernel (here: of the test), and runs a program.
    let mut shell = std::process::Command::new("sh")
        .args(["-c", "sleep 60 & echo $! > \"$D/inside\"; wait"])
        .env("D", &dir)
        .spawn()
        .unwrap();
    // A program the kernel launched for something else: an agent's tool, say.
    let mut elsewhere = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .unwrap();
    let inside = read_pid(&dir.join("inside")).expect("the shell started its program");
    let identity = |pid| process::inspect(pid).unwrap().0;

    let covered = |pid| state.access_grants_for(&identity(pid)).unwrap();
    assert!(
        covered(inside).is_empty(),
        "before the owner's shell is covered it has no access"
    );

    state.admit_owner_shell_access(shell.id()).unwrap();
    let grants = covered(inside);
    assert_eq!(
        grants.len(),
        1,
        "the program in the shell has the shell's access"
    );
    assert!(grants[0]
        .summary
        .grant_id
        .starts_with(crate::runtime::kernel_access::OWNER_SHELL_GRANT_PREFIX));
    assert!(
        covered(elsewhere.id()).is_empty(),
        "a kernel child outside that shell needs its own grant"
    );
    assert!(
        covered(shell.id()).len() == 1,
        "the shell itself is covered"
    );

    stop(inside);
    shell.kill().unwrap();
    shell.wait().unwrap();
    elsewhere.kill().unwrap();
    elsewhere.wait().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}
