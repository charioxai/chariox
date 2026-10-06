use std::fs::File;
use std::process::{Child, Command, ExitStatus, Stdio};
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
    let mut child = command
        .spawn()
        .expect("isolated environment test should start");
    let status = wait_for_environment_test(&mut child, Duration::from_secs(180)).unwrap();
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

fn wait_for_environment_test(
    child: &mut Child,
    timeout: Duration,
) -> std::io::Result<Option<ExitStatus>> {
    let status = child.wait_timeout(timeout)?;
    if status.is_none() {
        // The Child came from our spawn; never broaden cleanup to its group.
        assert!(child.id() > 1, "never signal a reserved PID");
        child.kill()?;
        child.wait()?;
    }
    Ok(status)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::os::unix::process::{CommandExt, ExitStatusExt};

    struct OwnedChild(Child);

    impl Drop for OwnedChild {
        fn drop(&mut self) {
            assert!(self.0.id() > 1, "never signal a reserved PID");
            let _ = self.0.kill();
            self.0.wait().expect("owned fixture child should be reaped");
        }
    }

    fn sleeping_child(group: i32) -> OwnedChild {
        assert!(group == 0 || group > 1, "invalid fixture process group");
        let mut command = Command::new("sleep");
        command.arg("60").process_group(group);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        OwnedChild(command.spawn().expect("owned fixture child should start"))
    }

    #[test]
    fn mp11_environment_timeout_kills_and_reaps_only_the_owned_child() {
        let mut child = sleeping_child(0);
        let group = i32::try_from(child.0.id()).unwrap();
        assert!(group > 1);
        let mut sibling = sleeping_child(group);
        let owned = [child.0.id(), sibling.0.id()];
        // Fail-first containment: even the old group-kill branch may reach only
        // these two spawned children. Verify every group member before invoking it.
        for entry in std::fs::read_dir("/proc").unwrap().flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<i32>().ok())
            else {
                continue;
            };
            if pid <= 1 || unsafe { libc::getpgid(pid) } != group {
                continue;
            }
            assert!(
                owned.contains(&(pid as u32)),
                "fixture group has a foreign member"
            );
            let stat = std::fs::read_to_string(entry.path().join("stat")).unwrap();
            let fields: Vec<_> = stat
                .rsplit_once(')')
                .unwrap()
                .1
                .split_whitespace()
                .collect();
            assert_eq!(fields[1].parse::<u32>().unwrap(), std::process::id());
        }
        for pid in owned {
            assert!(pid > 1);
            assert_eq!(unsafe { libc::getpgid(pid as i32) }, group);
        }

        assert!(
            wait_for_environment_test(&mut child.0, Duration::from_millis(20))
                .unwrap()
                .is_none()
        );
        assert_eq!(
            child.0.try_wait().unwrap().unwrap().signal(),
            Some(libc::SIGKILL)
        );
        assert!(
            sibling
                .0
                .wait_timeout(Duration::from_millis(50))
                .unwrap()
                .is_none(),
            "timeout must leave the other owned process in the same group alive"
        );
    }

    #[test]
    fn mp11_environment_wait_preserves_a_successful_exit() {
        let mut command = Command::new("true");
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = OwnedChild(command.spawn().unwrap());
        assert!(
            wait_for_environment_test(&mut child.0, Duration::from_secs(5))
                .unwrap()
                .unwrap()
                .success()
        );
    }
}
