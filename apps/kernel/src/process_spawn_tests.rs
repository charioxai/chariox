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

#[test]
fn spawned_trees_are_killable_when_the_kernel_is_oom_protected() {
    use std::os::unix::process::CommandExt;
    const ENV: &str = "CHARIOX_OOM_PROTECTION_REGRESSION_CHILD";
    if std::env::var_os(ENV).is_some() {
        assert_eq!(
            std::fs::read_to_string("/proc/self/oom_score_adj")
                .unwrap()
                .trim(),
            "-1000"
        );
        let mut command = Command::new("/bin/sh");
        command.args([
            "-c",
            "cat /proc/self/oom_score_adj; /bin/sh -c 'cat /proc/self/oom_score_adj'",
        ]);
        command.stdout(std::process::Stdio::piped());
        let output = spawn_command(command).unwrap().wait_with_output().unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            "0\n0\n",
            "owned trees must not inherit the kernel's OOM immunity"
        );
        let output = super::command("/bin/cat")
            .arg("/proc/self/oom_score_adj")
            .output()
            .unwrap();
        assert_eq!(output.stdout, b"0\n");
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let output = rt
            .block_on(async {
                super::async_command("/bin/cat")
                    .arg("/proc/self/oom_score_adj")
                    .output()
                    .await
            })
            .unwrap();
        assert_eq!(output.stdout, b"0\n");
        let pair = portable_pty::native_pty_system()
            .openpty(portable_pty::PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.args([
            "-c",
            "cat /proc/self/oom_score_adj; /bin/sh -c 'cat /proc/self/oom_score_adj'",
        ]);
        let mut child = spawn_pty(pair.slave, cmd).unwrap();
        let mut reader = std::io::BufReader::new(pair.master.try_clone_reader().unwrap());
        for _ in 0..2 {
            let mut line = String::new();
            std::io::BufRead::read_line(&mut reader, &mut line).unwrap();
            assert_eq!(line.trim(), "0");
        }
        assert!(child.wait().unwrap().success());
        assert_eq!(
            std::fs::read_to_string("/proc/self/oom_score_adj")
                .unwrap()
                .trim(),
            "-1000",
            "the kernel retains its own protection"
        );
        return;
    }
    let mut probe = Command::new(std::env::current_exe().unwrap());
    probe
        .args([
            "spawned_trees_are_killable_when_the_kernel_is_oom_protected",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(ENV, "1");
    unsafe {
        probe.pre_exec(|| {
            let fd = libc::open(
                c"/proc/self/oom_score_adj".as_ptr(),
                libc::O_WRONLY | libc::O_CLOEXEC,
            );
            if fd < 0 {
                return Err(std::io::Error::last_os_error());
            }
            let written = libc::write(fd, b"-1000".as_ptr().cast(), 5);
            let error = std::io::Error::last_os_error();
            libc::close(fd);
            if written != 5 {
                return Err(error);
            }
            Ok(())
        });
    }
    let mut child = probe.spawn().unwrap();
    let status = child.wait_timeout(Duration::from_secs(10)).unwrap();
    if status.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    assert!(
        status.is_some_and(|s| s.success()),
        "protected kernel must launch killable trees"
    );
}
