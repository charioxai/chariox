use std::fs::File;
use std::process::{Command, Stdio};
use std::time::Duration;
use wait_timeout::ChildExt;

const CHILD_TEST: &str = "CHARIOX_ISOLATED_ENV_TEST";
#[cfg(unix)]
const REAP_ATTEMPTS: u32 = 100;

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
    // Provider harnesses the test starts run in sessions of their own, so no
    // group signal reaches them; every descendant inherits this claim instead.
    #[cfg(unix)]
    let claim = root.path().display().to_string();
    #[cfg(unix)]
    command.env(
        crate::runtime::owned_process_signals::CLAIM_VARIABLE,
        &claim,
    );
    command
        .env(CHILD_TEST, name)
        .stdout(Stdio::from(File::create(&stdout).unwrap()))
        .stderr(Stdio::from(File::create(&stderr).unwrap()));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // MP-11: retain an exclusive-session launch witness for safe timeout cleanup.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
    }
    let mut child = command
        .spawn()
        .expect("isolated environment test should start");
    #[cfg(unix)]
    let mut signals =
        crate::runtime::owned_process_signals::OwnedProcessSignals::for_child(&child).ok();
    let status = child.wait_timeout(Duration::from_secs(180)).unwrap();
    if status.is_none() {
        let pid = i32::try_from(child.id()).expect("isolated child PID fits the signal API");
        assert!(pid > 1, "refusing to signal a reserved isolated child PID");
        #[cfg(unix)]
        if let Some(signals) = signals.as_mut() {
            let _ = signals.kill_group();
            // Detached descendants may survive even a successful group signal.
            let _ = signals.kill_owned_processes();
        }
        // The std Child handle still owns this unreaped, validated positive PID.
        let _ = child.kill();
        let _ = child.wait();
    }
    #[cfg(unix)]
    end_claimed_descendants(&claim);
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

/// The isolated test has ended, passed or not: whatever it started and left
/// running (provider servers, detached helpers) must not outlive it.
#[cfg(unix)]
fn end_claimed_descendants(claim: &str) {
    use crate::runtime::owned_process_signals::OwnedProcessSignals;
    for _ in 0..REAP_ATTEMPTS {
        match OwnedProcessSignals::end_claimed_tree(claim, None) {
            Ok(true) => return,
            Ok(false) => std::thread::sleep(Duration::from_millis(20)),
            Err(error) => panic!("isolated test descendants could not be ended: {error}"),
        }
    }
    panic!("isolated test descendants are still running after {REAP_ATTEMPTS} attempts");
}

#[cfg(all(test, unix))]
mod tests {
    use std::process::Command;
    use std::time::{Duration, Instant};

    use super::super::{detached_job, environment_test_isolated};

    /// The job sleeps this many seconds, so the owner finds it by its arguments.
    fn sleep_seconds(owner: u32) -> u32 {
        200_000 + owner % 100_000
    }

    fn detached_sleeper_running(seconds: u32) -> bool {
        let listing = Command::new("ps")
            .args(["-eo", "stat=,args="])
            .output()
            .expect("ps should list processes");
        String::from_utf8_lossy(&listing.stdout)
            .lines()
            .any(|line| {
                let mut fields = line.split_whitespace();
                fields.next().is_some_and(|stat| !stat.starts_with('Z'))
                    && fields.eq(["sleep", seconds.to_string().as_str()])
            })
    }

    #[test]
    fn an_isolated_test_leaves_no_detached_session_running() {
        let isolated = environment_test_isolated();
        let owner = if isolated {
            std::os::unix::process::parent_id()
        } else {
            std::process::id()
        };
        let seconds = sleep_seconds(owner);
        if super::isolate_environment_test() {
            assert!(
                !detached_sleeper_running(seconds),
                "the isolated test left `sleep {seconds}` running in a session of its own"
            );
            return;
        }
        let started = Command::new("sh")
            .arg("-c")
            .arg(detached_job("/dev/null", seconds))
            .status()
            .expect("detached job should start");
        assert!(started.success());
        let deadline = Instant::now() + Duration::from_secs(10);
        while !detached_sleeper_running(seconds) {
            assert!(Instant::now() < deadline, "the detached job should start");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
