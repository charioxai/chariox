use super::{PtyManager, PtySpawnRequest};
use std::io::{BufRead, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
#[ignore = "requires Bubblewrap and Linux user namespaces; run explicitly on the managed host"]
fn sandboxed_pty_dies_with_owning_process() {
    const TEST: &str = "pty::sandbox_lifetime_tests::sandboxed_pty_dies_with_owning_process";
    const CHILD_ENV: &str = "CHARIOX_PTY_PARENT_DEATH_TEST_CHILD";
    if std::env::var_os(CHILD_ENV).is_some() {
        let mut manager = PtyManager::new();
        manager
            .spawn(PtySpawnRequest {
                process_key: TEST.into(),
                provider_run_id: TEST.into(),
                program: "/usr/bin/bwrap".into(),
                args: [
                    "--die-with-parent",
                    "--new-session",
                    "--unshare-user",
                    "--unshare-pid",
                    "--uid",
                    "0",
                    "--gid",
                    "0",
                    "--ro-bind",
                    "/",
                    "/",
                    "--",
                    "/bin/sh",
                    "-c",
                    "trap '' HUP; printf 'ready\\n'; exec sleep 30",
                ]
                .into_iter()
                .map(str::to_string)
                .collect(),
                env: Default::default(),
                env_remove: Vec::new(),
                working_directory: None,
                cols: 80,
                rows: 24,
            })
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut output = Vec::new();
        while Instant::now() < deadline {
            output.extend(
                manager
                    .drain_output(TEST)
                    .unwrap()
                    .into_iter()
                    .flat_map(|c| c.bytes),
            );
            if String::from_utf8_lossy(&output).contains("ready") {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(String::from_utf8_lossy(&output).contains("ready"));
        println!("SANDBOX_PID={}", manager.process_id(TEST).unwrap().unwrap());
        std::io::stdout().flush().unwrap();
        // The parent authorizes exit only after reading the ready child's PID.
        let mut release = String::new();
        std::io::stdin().read_line(&mut release).unwrap();
        assert_eq!(release.trim(), "exit");
        std::process::exit(0);
    }

    // Reap the orphan ourselves so the assertion can distinguish SIGKILL from
    // an unrelated PTY hangup. This test runs explicitly, serially, on Linux.
    struct Subreaper(i32);
    impl Drop for Subreaper {
        fn drop(&mut self) {
            unsafe {
                libc::prctl(libc::PR_SET_CHILD_SUBREAPER, self.0);
            }
        }
    }
    let mut previous = 0i32;
    assert_eq!(
        unsafe { libc::prctl(libc::PR_GET_CHILD_SUBREAPER, &mut previous) },
        0
    );
    let _subreaper = Subreaper(previous);
    assert_eq!(unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1) }, 0);
    let mut owner = Command::new(std::env::current_exe().unwrap())
        .args([
            TEST,
            "--exact",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_ENV, "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = owner.stdout.take().unwrap();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
        {
            if let Some(pid) = line.strip_prefix("SANDBOX_PID=") {
                let _ = sender.send(pid.parse::<i32>().unwrap());
                break;
            }
        }
    });
    let pid = receiver.recv_timeout(Duration::from_secs(10));
    if pid.is_err() {
        let _ = owner.kill();
    }
    if pid.is_ok() {
        writeln!(owner.stdin.take().unwrap(), "exit").unwrap();
    }
    let owner_status = owner.wait().unwrap();
    reader.join().unwrap();
    let pid = pid.expect("sandbox should become ready before owner exit");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut status = 0;
    let mut reaped = false;
    while Instant::now() < deadline {
        if unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) } == pid {
            reaped = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    if !reaped {
        unsafe {
            libc::kill(pid, libc::SIGKILL);
            libc::waitpid(pid, &mut status, 0);
        }
    }
    assert!(owner_status.success());
    assert!(reaped, "sandbox survived owning process exit");
    assert!(libc::WIFSIGNALED(status));
    assert_eq!(
        libc::WTERMSIG(status),
        libc::SIGKILL,
        "must be parent-death cleanup, not PTY hangup"
    );
}
