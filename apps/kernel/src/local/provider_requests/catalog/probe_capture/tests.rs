#[cfg(unix)]
use super::OUTPUT_LIMIT;
use super::{capture, CaptureError};
use std::process::Command;
use std::time::{Duration, Instant};

#[cfg(unix)]
fn shell(script: &str) -> Command {
    let mut command = Command::new("sh");
    command.args(["-c", script]);
    command
}

#[cfg(unix)]
#[test]
fn captures_both_streams_and_exit_status() {
    let output = capture(
        shell("printf version; printf stats >&2; exit 7"),
        Duration::from_secs(1),
    )
    .unwrap();
    assert_eq!(output.stdout, b"version");
    assert_eq!(output.stderr, b"stats");
    assert_eq!(output.status.code(), Some(7));
}

#[cfg(unix)]
#[test]
fn caps_stdout_and_stderr_together() {
    // Finite output from each stream individually fits, but their sum does not.
    let script = format!(
        "head -c {} /dev/zero; head -c {} /dev/zero >&2",
        OUTPUT_LIMIT / 2 + 1,
        OUTPUT_LIMIT / 2 + 1
    );
    let error = capture(shell(&script), Duration::from_secs(5)).unwrap_err();
    assert!(matches!(error, CaptureError::OutputLimit));
}

#[cfg(unix)]
#[test]
fn a_flooding_probe_fails_without_waiting_for_its_deadline() {
    let started = Instant::now();
    let error = capture(
        shell("while :; do printf 'noisy status'; done"),
        Duration::from_secs(5),
    )
    .unwrap_err();
    assert!(matches!(error, CaptureError::OutputLimit));
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[cfg(unix)]
#[test]
fn stalled_version_and_stats_are_bounded() {
    for args in [vec!["--version"], vec!["stats", "--format", "json"]] {
        let mut command = shell("sleep 30");
        command.args(args);
        let started = Instant::now();
        let error = capture(command, Duration::from_millis(80)).unwrap_err();
        assert!(matches!(error, CaptureError::TimedOut));
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod escaped {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "chariox-probe-capture-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&root).unwrap();
            Self(root)
        }
        fn command(&self, stalled_parent: bool) -> Command {
            // The child test acknowledges setsid before its original parent
            // exits. It keeps stderr open while stdout is redirected to null.
            let script = format!("\"$1\" --ignored --exact \"$2\" --test-threads=1 > /dev/null & while [ ! -s \"$3\" ]; do sleep .005; done; {}", if stalled_parent { "sleep 30" } else { "printf ok" });
            let mut command = shell(&script);
            command
                .arg("parent")
                .arg(std::env::current_exe().unwrap())
                .arg(test_name("escaped_writer_helper"))
                .arg(self.0.join("pid"))
                .env("CHARIOX_PROBE_TEST_PID", self.0.join("pid"))
                .env("CHARIOX_PROBE_TEST_MARKER", self.0.join("survived"));
            command
        }

        fn assert_stopped(&self) {
            let pid: i32 = std::fs::read_to_string(self.0.join("pid"))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            let deadline = Instant::now() + Duration::from_millis(100);
            while running(pid) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(!running(pid), "escaped writer is still running");
            std::thread::sleep(Duration::from_millis(400));
            assert!(
                !self.0.join("survived").exists(),
                "escaped writer executed after capture completed"
            );
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            // The capture operation owns and stops the helper through its exact
            // output pipes, including error paths. A saved PID alone is never
            // authority to signal a possibly reused process during cleanup.
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn successful_parent_stops_an_escaped_pipe_writer() {
        let fixture = Fixture::new();
        let started = Instant::now();
        let output = capture(fixture.command(false), Duration::from_secs(2)).unwrap();
        assert_eq!(output.stdout, b"ok");
        assert!(output.status.success());
        assert!(started.elapsed() < Duration::from_secs(1));
        fixture.assert_stopped();
    }

    #[test]
    fn timed_out_parent_stops_an_escaped_pipe_writer() {
        let fixture = Fixture::new();
        let started = Instant::now();
        let error = capture(fixture.command(true), Duration::from_millis(80)).unwrap_err();
        assert!(matches!(error, CaptureError::TimedOut));
        assert!(started.elapsed() < Duration::from_secs(1));
        fixture.assert_stopped();
    }

    fn test_name(name: &str) -> String {
        format!("{}::{name}", module_path!().split_once("::").unwrap().1)
    }

    fn running(pid: i32) -> bool {
        if super::super::platform::signal_pid(pid.into()).is_err() {
            return false;
        }
        #[cfg(target_os = "linux")]
        {
            std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .map(|value| {
                    !value
                        .rsplit_once(')')
                        .unwrap()
                        .1
                        .trim_start()
                        .starts_with('Z')
                })
                .unwrap_or(false)
        }
        #[cfg(target_os = "macos")]
        unsafe {
            libc::kill(pid, 0) == 0
        }
    }

    #[test]
    #[ignore = "subprocess fixture selected by escaped-writer tests"]
    fn escaped_writer_helper() {
        let pid = std::env::var_os("CHARIOX_PROBE_TEST_PID").unwrap();
        let marker = std::env::var_os("CHARIOX_PROBE_TEST_MARKER").unwrap();
        assert!(unsafe { libc::setsid() } >= 0);
        std::fs::write(pid, std::process::id().to_string()).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        std::fs::write(marker, "survived").unwrap();
        loop {
            eprintln!("escaped writer");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn repeated_timeouts_leave_no_reader_threads() {
        // Run alone in a subprocess so unrelated concurrently running tests
        // cannot change the thread count while this regression measures it.
        if std::env::var_os("CHARIOX_PROBE_THREAD_CHECK").is_none() {
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    &test_name("repeated_timeouts_leave_no_reader_threads"),
                    "--test-threads=1",
                ])
                .env("CHARIOX_PROBE_THREAD_CHECK", "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
            return;
        }
        let before = std::fs::read_dir("/proc/self/task").unwrap().count();
        for _ in 0..10 {
            let error = capture(shell("sleep 30"), Duration::from_millis(10)).unwrap_err();
            assert!(matches!(error, CaptureError::TimedOut));
        }
        assert_eq!(
            std::fs::read_dir("/proc/self/task").unwrap().count(),
            before,
            "reader threads escaped capture ownership"
        );
    }
}

#[cfg(windows)]
#[test]
fn windows_captures_both_streams_and_times_out() {
    let mut quick = Command::new("cmd.exe");
    quick.args(["/C", "echo version & echo stats >&2"]);
    let output = capture(quick, Duration::from_secs(2)).unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("version"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("stats"));
    let mut stalled = Command::new("cmd.exe");
    stalled.args(["/C", "ping -n 30 127.0.0.1 > nul"]);
    let started = Instant::now();
    assert!(matches!(
        capture(stalled, Duration::from_millis(80)),
        Err(CaptureError::TimedOut)
    ));
    assert!(started.elapsed() < Duration::from_secs(2));
}

// MP-08 / MP-10 / MP-11: owning capture retains redirected descendants as well as pipe writers.
#[cfg(target_os = "linux")]
#[test]
fn review0212_timeout_settles_descendants_with_both_streams_redirected() {
    let root = crate::test_support::TestWorktree::new("review0212-redirected");
    let pid_path = root.path().join("child-pid");
    let script = format!(
        "sleep 30 >/dev/null 2>&1 & child=$!; printf '%s' \"$child\" > '{}'; wait",
        pid_path.display()
    );
    let error = capture(shell(&script), Duration::from_millis(100)).unwrap_err();
    assert!(matches!(error, CaptureError::TimedOut));
    let pid = std::fs::read_to_string(&pid_path)
        .unwrap()
        .parse::<i32>()
        .unwrap();
    assert!(pid > 1);
    let running_now = || {
        std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
            !stat
                .rsplit_once(')')
                .unwrap()
                .1
                .trim_start()
                .starts_with('Z')
        })
    };
    for _ in 0..20 {
        if !running_now() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let running = running_now();
    if running {
        // Pin this known fixture child before settling a fail-first leak; never signal a group.
        use std::os::fd::{AsRawFd, FromRawFd};
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        assert!(fd >= 0);
        let pinned = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd as i32) };
        unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                pinned.as_raw_fd(),
                libc::SIGKILL,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            );
        }
    }
    assert!(
        !running,
        "redirected owned descendant survived capture timeout"
    );
}

// MP-08 / MP-10 / MP-11: synchronized helpers close both capture pipes.
#[cfg(unix)]
#[test]
fn review0821_redirected_descendants_stop_after_success_and_timeout() {
    for stalled in [false, true] {
        let root = crate::test_support::TestWorktree::new("review0821-redirected");
        let mut command = shell(
            r#"sh -c 'echo $$ > "$1/pid"; sleep .4; echo survived > "$1/survived"' helper "$1" >/dev/null 2>&1 & while [ ! -s "$1/pid" ]; do sleep .005; done; if [ "$2" = stall ]; then wait; else printf ok; fi"#,
        );
        command
            .arg("launcher")
            .arg(root.path())
            .arg(if stalled { "stall" } else { "success" });
        let result = capture(
            command,
            if stalled {
                Duration::from_millis(100)
            } else {
                Duration::from_secs(2)
            },
        );
        // The helper is finite even on RED; do not leak test subprocesses.
        std::thread::sleep(Duration::from_millis(500));
        assert!(
            !root.path().join("survived").exists(),
            "redirected descendant executed after capture completed (stalled={stalled})"
        );
        if stalled {
            assert!(matches!(result, Err(CaptureError::TimedOut)));
        } else {
            assert_eq!(result.unwrap().stdout, b"ok");
        }
    }
}

// MP-08 / MP-10 / MP-11: group membership proves ownership only in a private session.
#[cfg(unix)]
#[test]
fn review0821_probe_group_rejects_unrelated_siblings() {
    use std::{os::unix::process::CommandExt, process::Stdio};
    let mut command = shell("sleep 1");
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut probe = super::platform::Process::spawn(&mut command).unwrap();
    let pid = probe.child.id();
    assert!(pid > 1 && pid <= i32::MAX as u32);
    let mut sibling = shell("sleep .1");
    unsafe {
        sibling.pre_exec(move || {
            if libc::setpgid(0, pid as i32) < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    let rejected = match sibling.spawn() {
        Err(error) => error.raw_os_error() == Some(libc::EPERM),
        Ok(mut child) => {
            // This fixture is ours; do not let it enter probe cleanup's ownership set.
            assert!(child.id() > 1);
            let _ = child.kill();
            child.wait().unwrap();
            false
        }
    };
    probe.stop().unwrap();
    assert!(
        rejected,
        "unrelated sibling could join the probe's cleanup group"
    );
}
