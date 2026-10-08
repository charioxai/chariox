use std::fs::File;
use std::process::{Command, Stdio};
use std::time::Duration;
use wait_timeout::ChildExt;

const CHILD_TEST: &str = "CHARIOX_ISOLATED_ENV_TEST";

pub(crate) fn environment_test_isolated() -> bool {
    std::env::var_os(CHILD_TEST).is_some()
}

pub(crate) fn isolate_environment_test() -> bool {
    let thread = std::thread::current();
    let name = thread
        .name()
        .expect("environment fixture needs a test name");
    if std::env::var(CHILD_TEST).as_deref() == Ok(name) {
        return false;
    }
    let root = super::TestWorktree::new("environment");
    let mut command = Command::new(std::env::current_exe().expect("test binary should resolve"));
    // The parent already selected this test, including an explicitly requested ignored proof.
    command.args([
        name,
        "--exact",
        "--include-ignored",
        "--nocapture",
        "--test-threads=1",
    ]);
    command.env_clear();
    // Preserve tool locations, never the kernel's bootstrap/provider state.
    for (key, value) in std::env::vars_os() {
        let key_name = key.to_string_lossy();
        if key_name.starts_with("CARGO_")
            || key_name.starts_with("RUSTUP_")
            || matches!(
                key_name.as_ref(),
                "PATH"
                    | "RUST_MIN_STACK"
                    | "LD_LIBRARY_PATH"
                    | "DYLD_LIBRARY_PATH"
                    | "SYSTEMROOT"
                    | "COMSPEC"
                    | "PATHEXT"
                    | "PYTHON"
                    | "NODE"
            )
        {
            command.env(key, value);
        }
    }
    for (key, relative) in [
        ("HOME", "home"),
        ("CHARIOX_HOME", "home/.chariox"),
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_STATE_HOME", "state"),
        ("XDG_CACHE_HOME", "cache"),
        ("XDG_DATA_HOME", "data"),
        ("XDG_RUNTIME_DIR", "runtime"),
        ("TMPDIR", "tmp"),
    ] {
        let path = root.path().join(relative);
        std::fs::create_dir_all(&path).expect("isolated environment directory should exist");
        command.env(key, path);
    }
    let stdout = root.path().join("stdout");
    let stderr = root.path().join("stderr");
    command
        .env(CHILD_TEST, name)
        .stdout(Stdio::from(File::create(&stdout).unwrap()))
        .stderr(Stdio::from(File::create(&stderr).unwrap()));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .expect("isolated environment test should start");
    let status = child.wait_timeout(Duration::from_secs(180)).unwrap();
    if status.is_none() {
        let pid = i32::try_from(child.id()).expect("isolated child PID fits the signal API");
        assert!(pid > 1, "refusing to signal a reserved isolated child PID");
        #[cfg(unix)]
        if owned_test_process_group(pid) && child.try_wait().is_ok_and(|status| status.is_none()) {
            // The child created this group; every current member is its descendant.
            unsafe { libc::kill(-pid, libc::SIGKILL); }
        }
        let _ = child.kill();
        let _ = child.wait();
    }
    let stdout = std::fs::read_to_string(stdout).unwrap();
    let stderr = std::fs::read_to_string(stderr).unwrap();
    assert!(
        status.is_some_and(|status| status.success()) && stdout.contains("running 1 test"),
        "isolated environment test {name} failed ({status:?}):\n{stdout}\n{stderr}"
    );
    for line in stdout.lines().chain(stderr.lines()) {
        if line.starts_with("SKIP ") || line.starts_with("skipping ") {
            eprintln!("{name}: {line}");
        }
    }
    true
}

#[cfg(unix)]
fn owned_test_process_group(pid: i32) -> bool {
    if pid <= 1 || unsafe { libc::getpgid(pid) } != pid {
        return false;
    }
    let Ok(output) = Command::new("ps").args(["-eo", "pid=,ppid=,pgid="])
        .env("LC_ALL", "C").output() else { return false; };
    if !output.status.success() { return false; }
    let rows: Option<Vec<(i32, i32, i32)>> = String::from_utf8_lossy(&output.stdout).lines()
        .map(|line| {
            let mut fields = line.split_whitespace();
            Some((fields.next()?.parse().ok()?, fields.next()?.parse().ok()?, fields.next()?.parse().ok()?))
        }).collect();
    let Some(rows) = rows else { return false; };
    test_group_members_owned(pid, std::process::id(), &rows)
}

#[cfg(unix)]
fn test_group_members_owned(root: i32, parent: u32, rows: &[(i32, i32, i32)]) -> bool {
    if root <= 1 || !rows.iter().any(|&(pid, ppid, group)|
        pid == root && u32::try_from(ppid) == Ok(parent) && group == root) {
        return false;
    }
    rows.iter().filter(|&&(_, _, group)| group == root).all(|&(member, _, _)| {
        if member <= 1 { return false; }
        let mut ancestor = member;
        for _ in 0..rows.len() {
            if ancestor == root { return true; }
            let Some(&(_, ppid, _)) = rows.iter().find(|&&(pid, _, _)| pid == ancestor) else { return false; };
            if ppid <= 1 { return false; }
            ancestor = ppid;
        }
        false
    })
}

#[cfg(unix)]
#[test]
fn mp11_isolated_test_signal_rejects_reserved_and_foreign_groups() {
    let owned = [(10, 9, 10), (11, 10, 10), (12, 11, 10)];
    assert!(test_group_members_owned(10, 9, &owned));
    for reserved in [-1, 0, 1] {
        assert!(!test_group_members_owned(reserved, 9, &owned));
    }
    assert!(!test_group_members_owned(10, 8, &owned));
    assert!(!test_group_members_owned(10, 9, &[(10, 9, 10), (1, 10, 10)]));
    assert!(!test_group_members_owned(10, 9, &[(10, 9, 10), (11, 1, 10)]));
    assert!(!test_group_members_owned(10, 9, &[(10, 9, 10), (11, 12, 10), (12, 11, 10)]));
    assert!(!test_group_members_owned(10, 9, &[(11, 10, 10)]));
}
