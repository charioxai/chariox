// MP-08 / MP-10 / MP-11: CLI diagnostics must precede runtime admission.
use std::{
    fs,
    net::TcpListener,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("remove owned startup scratch");
    }
}

pub fn assert_no_startup(binary: &str, args: &[&str], success: bool, expected: &str) {
    // Test explicit CHARIOX_HOME and the HOME/XDG defaults, with a free and an
    // occupied port. Occupied-port diagnostics must have the same exit/result.
    for explicit_home in [false, true] {
        for occupied in [false, true] {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let scratch = Scratch(std::env::temp_dir().join(format!(
                "chariox-cli-startup-{}-{nonce}",
                std::process::id()
            )));
            fs::create_dir(&scratch.0).unwrap();
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let port = listener.local_addr().unwrap().port();
            let reserved = occupied.then_some(listener);
            let mut command = Command::new(binary);
            command
                .args(args)
                .env_clear()
                .env("HOME", scratch.0.join("home"))
                .env("XDG_CONFIG_HOME", scratch.0.join("config"))
                .env("XDG_STATE_HOME", scratch.0.join("state"))
                .env("XDG_RUNTIME_DIR", scratch.0.join("runtime"))
                .env("CHARIOX_LOG_DIR", scratch.0.join("logs"))
                .env("CHARIOX_KERNEL_HOST", "127.0.0.1")
                .env("CHARIOX_KERNEL_PORT", port.to_string())
                .env("CHARIOX_RELAY_HOST", "127.0.0.1")
                .env("CHARIOX_RELAY_PORT", port.to_string())
                // A rejected argument must not depend on Bun or trigger a build.
                .env("BUN_BIN", scratch.0.join("missing-bun"))
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            if explicit_home {
                command.env("CHARIOX_HOME", scratch.0.join("chariox"));
            }
            let mut child = command.spawn().unwrap();
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut bound_socket = false;
            let mut timed_out = false;
            loop {
                if !occupied && TcpListener::bind(("127.0.0.1", port)).is_err() {
                    bound_socket = true;
                }
                if child.try_wait().unwrap().is_some() {
                    break;
                }
                if Instant::now() >= deadline {
                    // Only signal our directly spawned child, never a process group.
                    assert!(child.id() > 1, "refuse to signal a reserved PID");
                    child.kill().unwrap();
                    timed_out = true;
                    break;
                }
                thread::sleep(Duration::from_millis(2));
            }
            let output = child.wait_with_output().unwrap();
            drop(reserved);
            assert!(!bound_socket, "{binary} {args:?} bound a socket");
            assert_eq!(
                fs::read_dir(&scratch.0).unwrap().count(),
                0,
                "{binary} {args:?} created state/log/config files"
            );
            assert!(!timed_out, "{binary} {args:?} started instead of exiting");
            assert_eq!(
                output.status.success(),
                success,
                "{binary} {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let message = if success {
                &output.stdout
            } else {
                &output.stderr
            };
            assert!(
                String::from_utf8_lossy(message).contains(expected),
                "{binary} {args:?} missing {expected:?}: {}",
                String::from_utf8_lossy(message)
            );
            if success {
                assert!(
                    output.stderr.is_empty(),
                    "help/version wrote startup diagnostics"
                );
            }
        }
    }
}
