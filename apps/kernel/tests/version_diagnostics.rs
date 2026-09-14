use std::fs;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should follow the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "chariox-version-diagnostics-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create version diagnostic scratch directory");
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run_version(binary: &str, home: &PathBuf, port: u16) -> Output {
    let mut child = Command::new(binary)
        .arg("--version")
        .env_clear()
        .env("CHARIOX_HOME", home)
        .env("CHARIOX_KERNEL_HOST", "127.0.0.1")
        .env("CHARIOX_KERNEL_PORT", port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn version diagnostic");
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if child.try_wait().expect("poll version diagnostic").is_some() {
            return child
                .wait_with_output()
                .expect("collect version diagnostic output");
        }
        if Instant::now() >= deadline {
            child.kill().expect("stop hung version diagnostic");
            child.wait().expect("reap hung version diagnostic");
            panic!("--version did not exit before the bootstrap timeout");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn assert_pre_bootstrap_version(binary: &str, program: &str) {
    let scratch = Scratch::new(program);
    let home = scratch.0.join("home");
    fs::create_dir(&home).expect("create empty CHARIOX_HOME");
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("reserve kernel port");
    let port = listener.local_addr().expect("read reserved port").port();

    let output = run_version(binary, &home, port);

    assert!(
        output.status.success(),
        "{program} --version failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("version stdout should be UTF-8"),
        format!("{program} {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(
        output.stderr.is_empty(),
        "version diagnostic wrote stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_dir(&home)
            .expect("read empty CHARIOX_HOME")
            .count(),
        0,
        "version diagnostic created runtime state"
    );
}

#[test]
fn kernel_version_exits_before_bootstrap() {
    assert_pre_bootstrap_version(env!("CARGO_BIN_EXE_chariox-kernel"), "chariox-kernel");
}

#[test]
fn managed_bootstrap_version_exits_before_bootstrap() {
    assert_pre_bootstrap_version(
        env!("CARGO_BIN_EXE_chariox-managed-bootstrap"),
        "chariox-managed-bootstrap",
    );
}
