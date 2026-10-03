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
            if let Ok(pid) = std::fs::read_to_string(self.0.join("pid")) {
                if let Ok(pid) = pid.trim().parse::<i32>() {
                    unsafe {
                        libc::kill(-pid, libc::SIGKILL);
                    }
                }
            }
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
