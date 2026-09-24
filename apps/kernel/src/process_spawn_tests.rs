use super::*;
use std::time::Duration;
use wait_timeout::ChildExt;

struct PanickingPty;

impl SlavePty for PanickingPty {
    fn spawn_command(
        &self,
        _command: CommandBuilder,
    ) -> anyhow::Result<Box<dyn portable_pty::Child + Send + Sync>> {
        panic!("synthetic PTY backend failure");
    }
}

#[test]
fn panicking_pty_does_not_disable_future_spawns() {
    const CHILD_ENV: &str = "CHARIOX_SPAWN_PANIC_REGRESSION_CHILD";
    if std::env::var_os(CHILD_ENV).is_some() {
        // Exercise the external PTY backend boundary, not the owner queue.
        assert!(spawn_pty(Box::new(PanickingPty), CommandBuilder::new("/bin/true")).is_err());
        let mut child = spawn_command(Command::new("/bin/true"))
            .expect("a failed PTY backend must not disable subsequent process launches");
        assert!(child.wait().unwrap().success());
        return;
    }

    // A failing baseline permanently poisons its owner; isolate it from all
    // other tests and bound even a broken reply-channel implementation.
    let mut probe = Command::new(std::env::current_exe().unwrap())
        .args([
            "panicking_pty_does_not_disable_future_spawns",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_ENV, "1")
        .spawn()
        .unwrap();
    let status = probe.wait_timeout(Duration::from_secs(10)).unwrap();
    if status.is_none() {
        let _ = probe.kill();
        let _ = probe.wait();
    }
    assert!(
        status.is_some_and(|status| status.success()),
        "process spawning must remain usable after a PTY backend panic"
    );
}
