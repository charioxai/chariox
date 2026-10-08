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
        #[cfg(unix)]
        {
            let pid = i32::try_from(child.id()).expect("child PID fits the signal interface");
            assert!(pid > 1, "refusing reserved child PID");
            if let Some(group) = verified_isolated_group(pid) {
                // The child created this group; every current member was
                // verified as the child or one of its descendants.
                unsafe { libc::kill(-group, libc::SIGKILL) };
            }
        }
        assert!(child.id() > 1, "refusing reserved child PID");
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
fn verified_isolated_group(pid: i32) -> Option<i32> {
    if pid <= 1 || unsafe { libc::getpgid(pid) } != pid {
        return None;
    }
    let processes = sysinfo::System::new_all();
    let root = sysinfo::Pid::from_u32(pid as u32);
    if processes.process(root)?.parent() != Some(sysinfo::Pid::from_u32(std::process::id())) {
        return None;
    }
    let parents = processes
        .processes()
        .iter()
        .map(|(id, process)| (id.as_u32(), process.parent().map(|parent| parent.as_u32())))
        .collect::<std::collections::HashMap<_, _>>();
    let members = processes
        .processes()
        .keys()
        .filter_map(|id| {
            let member = i32::try_from(id.as_u32()).ok()?;
            (member > 1 && unsafe { libc::getpgid(member) } == pid).then_some(member as u32)
        })
        .collect::<Vec<_>>();
    if !members.contains(&(pid as u32))
        || members
            .iter()
            .any(|member| !owned_descendant(*member, pid as u32, &parents))
    {
        return None;
    }
    (unsafe { libc::getpgid(pid) } == pid).then_some(pid)
}

#[cfg(unix)]
fn owned_descendant(
    mut member: u32,
    root: u32,
    parents: &std::collections::HashMap<u32, Option<u32>>,
) -> bool {
    if root <= 1 || member <= 1 {
        return false;
    }
    for _ in 0..=parents.len() {
        if member == root {
            return true;
        }
        match parents.get(&member).copied().flatten() {
            Some(parent) if parent > 1 && parent != member => member = parent,
            _ => return false,
        }
    }
    false
}

#[cfg(all(test, unix))]
mod signal_guard_tests {
    use super::*;
    #[test]
    fn reserved_groups_are_refused_before_process_inspection() {
        for pid in [i32::MIN, -1, 0, 1] {
            assert_eq!(verified_isolated_group(pid), None);
        }
    }
    #[test]
    fn group_members_must_descend_from_the_owned_child() {
        let parents = [
            (20, Some(10)),
            (21, Some(20)),
            (22, Some(21)),
            (30, Some(1)),
            (40, Some(41)),
            (41, Some(40)),
        ]
        .into_iter()
        .collect();
        assert!(owned_descendant(20, 20, &parents));
        assert!(owned_descendant(22, 20, &parents));
        for member in [0, 1, 30, 40, 99] {
            assert!(!owned_descendant(member, 20, &parents));
        }
    }
}
