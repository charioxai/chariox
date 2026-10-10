use std::fs::File;
use std::process::{Child, Command, Stdio};
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
        terminate_isolated_child(&mut child);
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

fn terminate_isolated_child(child: &mut Child) {
    let root_pid = child.id();
    if root_pid <= 1 || root_pid > i32::MAX as u32 || !matches!(child.try_wait(), Ok(None)) {
        return;
    }
    #[cfg(target_os = "linux")]
    {
        use std::collections::{BTreeMap, BTreeSet};
        let mut processes = BTreeMap::new();
        for entry in std::fs::read_dir("/proc").unwrap().flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<u32>().ok())
            else {
                continue;
            };
            if pid <= 1 || pid > i32::MAX as u32 {
                continue;
            }
            let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
                continue;
            };
            let Some((_, fields)) = stat.rsplit_once(") ") else {
                continue;
            };
            let fields: Vec<_> = fields.split_whitespace().collect();
            if fields.len() > 19 {
                if let Ok(parent) = fields[1].parse::<u32>() {
                    processes.insert(pid, (parent, fields[19].to_string()));
                }
            }
        }
        let mut owned = BTreeSet::from([root_pid]);
        loop {
            let next: Vec<_> = processes
                .iter()
                .filter_map(|(pid, (parent, _))| {
                    (owned.contains(parent) && !owned.contains(pid)).then_some(*pid)
                })
                .collect();
            if next.is_empty() {
                break;
            }
            owned.extend(next);
        }
        for pid in owned.into_iter().rev().filter(|pid| *pid != root_pid) {
            if pid <= 1 || pid > i32::MAX as u32 {
                continue;
            }
            let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
                continue;
            };
            let Some((_, fields)) = stat.rsplit_once(") ") else {
                continue;
            };
            if fields.split_whitespace().nth(19)
                == processes.get(&pid).map(|(_, start)| start.as_str())
            {
                // Positive, identity-checked descendants of the child we spawned.
                let _ = unsafe { libc::kill(pid as i32, libc::SIGKILL) };
            }
        }
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        // The unreaped child reserves its group leader PID. Verify the group
        // contains only that child and its descendants before a group signal.
        if let Ok(output) = Command::new("/bin/ps")
            .args(["-axo", "pid=,ppid=,pgid="])
            .output()
        {
            let snapshot = String::from_utf8_lossy(&output.stdout);
            if output.status.success()
                && owned_isolated_group(root_pid, &snapshot)
                && matches!(child.try_wait(), Ok(None))
            {
                let _ = unsafe { libc::kill(-(root_pid as i32), libc::SIGKILL) };
            }
        }
    }

    if child.id() > 1 && matches!(child.try_wait(), Ok(None)) {
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[cfg(target_os = "linux")]
#[test]
fn isolated_timeout_cleanup_stops_its_owned_child_and_descendant() {
    use std::io::{BufRead, BufReader};
    let mut child = Command::new("/bin/sh")
        .args(["-c", "sleep 60 & echo $!; wait"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let descendant: u32 = line.trim().parse().unwrap();
    assert!(descendant > 1);
    terminate_isolated_child(&mut child);
    assert!(child.try_wait().unwrap().is_some());
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    loop {
        let stat = std::fs::read_to_string(format!("/proc/{descendant}/stat"));
        if stat.is_err() || stat.unwrap().rsplit_once(") ").unwrap().1.starts_with("Z ") {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "owned descendant must stop"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(unix)]
fn owned_isolated_group(root: u32, snapshot: &str) -> bool {
    use std::collections::{BTreeMap, BTreeSet};
    if root <= 1 || root > i32::MAX as u32 {
        return false;
    }
    let mut processes = BTreeMap::new();
    for line in snapshot.lines().filter(|line| !line.trim().is_empty()) {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() != 3 {
            return false;
        }
        let (Ok(pid), Ok(parent), Ok(group)) = (
            fields[0].parse::<u32>(),
            fields[1].parse::<u32>(),
            fields[2].parse::<u32>(),
        ) else {
            return false;
        };
        if processes.insert(pid, (parent, group)).is_some() {
            return false;
        }
    }
    if processes.get(&root).map(|(_, group)| *group) != Some(root) {
        return false;
    }
    processes
        .iter()
        .filter(|(_, (_, group))| *group == root)
        .all(|(pid, _)| {
            if *pid <= 1 || *pid > i32::MAX as u32 {
                return false;
            }
            let mut current = *pid;
            let mut seen = BTreeSet::new();
            while current != root {
                if !seen.insert(current) {
                    return false;
                }
                let Some((parent, _)) = processes.get(&current) else {
                    return false;
                };
                current = *parent;
            }
            true
        })
}

#[cfg(unix)]
#[test]
fn isolated_timeout_group_requires_safe_id_and_owned_members() {
    assert!(owned_isolated_group(
        42,
        "42 7 42\n43 42 42\n44 43 42\n99 1 99\n"
    ));
    for root in [0, 1, u32::MAX] {
        assert!(!owned_isolated_group(root, "42 7 42"));
    }
    assert!(!owned_isolated_group(42, "42 7 42\n43 1 42"));
    assert!(!owned_isolated_group(42, "42 7 7\n43 42 42"));
    assert!(!owned_isolated_group(42, "43 42 42"));
    assert!(!owned_isolated_group(42, "42 7 42\n42 7 42"));
    assert!(!owned_isolated_group(42, "42 7 42\nmalformed"));
    assert!(!owned_isolated_group(42, "42 7 42\n43 44 42\n44 43 42"));
}
