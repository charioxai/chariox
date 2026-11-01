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
fn ordinary_user_spawns_from_a_non_dumpable_kernel() {
    use std::os::unix::process::CommandExt;
    const ENV: &str = "CHARIOX_NON_DUMPABLE_SPAWN_REGRESSION_CHILD";
    if std::env::var_os(ENV).is_some() {
        // The kernel does this at startup; it makes /proc/self files root-owned.
        assert_eq!(unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0) }, 0);
        let output = super::command("/bin/cat")
            .arg("/proc/self/oom_score_adj")
            .output()
            .expect("an ordinary user's non-dumpable kernel must be able to spawn");
        assert!(output.status.success());
        let mut child = spawn_command(Command::new("/bin/true"))
            .expect("spawn_command must not depend on /proc/self write access");
        assert!(child.wait().unwrap().success());
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let status = rt
            .block_on(async { super::async_command("/bin/true").status().await })
            .expect("async spawns must not depend on /proc/self write access");
        assert!(status.success());
        return;
    }
    let mut probe = Command::new("/proc/self/exe");
    probe
        .args([
            "ordinary_user_spawns_from_a_non_dumpable_kernel",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(ENV, "1");
    unsafe {
        probe.pre_exec(|| {
            // Root keeps CAP_DAC_OVERRIDE and would hide the defect.
            if libc::geteuid() == 0
                && (libc::setgroups(0, std::ptr::null()) != 0
                    || libc::setgid(65534) != 0
                    || libc::setuid(65534) != 0)
            {
                return Err(std::io::Error::last_os_error());
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
        "an unprivileged non-dumpable parent must still spawn children"
    );
}

#[test]
#[ignore = "privileged drill requires CAP_SYS_RESOURCE; see docs/PROCESS_RESOURCES.md"]
fn spawned_trees_are_killable_when_the_kernel_is_oom_protected() {
    check_inherited_oom_policy(-1000);
}

#[test]
fn spawned_trees_preserve_a_positive_oom_policy() {
    check_inherited_oom_policy(350);
}

fn check_inherited_oom_policy(inherited: i32) {
    use std::os::unix::process::CommandExt;
    const ENV: &str = "CHARIOX_OOM_PROTECTION_REGRESSION_CHILD";
    let expected = inherited.max(0).to_string();
    if std::env::var_os(ENV).is_some() {
        assert_eq!(
            std::fs::read_to_string("/proc/self/oom_score_adj")
                .unwrap()
                .trim(),
            inherited.to_string()
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
            format!("{expected}\n{expected}\n"),
            "owned trees must not inherit the kernel's OOM immunity"
        );
        let output = super::command("/bin/cat")
            .arg("/proc/self/oom_score_adj")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, format!("{expected}\n").as_bytes());
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
        assert!(output.status.success());
        assert_eq!(output.stdout, format!("{expected}\n").as_bytes());
        let pair = portable_pty::native_pty_system()
            .openpty(portable_pty::PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut cmd = CommandBuilder::new("/bin/sh");
        // The positive-policy child has dropped uid; inherited HOME may be /root.
        cmd.cwd("/");
        cmd.args([
            "-c",
            "cat /proc/self/oom_score_adj; /bin/sh -c 'cat /proc/self/oom_score_adj'",
        ]);
        let mut child = spawn_pty(pair.slave, cmd).unwrap();
        let mut reader = std::io::BufReader::new(pair.master.try_clone_reader().unwrap());
        for _ in 0..2 {
            let mut line = String::new();
            std::io::BufRead::read_line(&mut reader, &mut line).unwrap();
            assert_eq!(line.trim(), expected);
        }
        assert!(child.wait().unwrap().success());
        assert_eq!(
            std::fs::read_to_string("/proc/self/oom_score_adj")
                .unwrap()
                .trim(),
            inherited.to_string(),
            "the kernel retains its own policy"
        );
        // Exercise the provider/native shell entry itself, without a command
        // constructor clearing protection before the shell gets to inspect it.
        let domain =
            crate::runtime::process_resources::PreparedDomain::new("process-spawn-test").unwrap();
        let (program, args) =
            domain.command("/bin/cat".into(), vec!["/proc/self/oom_score_adj".into()]);
        let output = Command::new(program).args(args).output().unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, format!("{expected}\n").as_bytes());
        return;
    }
    // /proc/self/exe remains executable after dropping uid even when this
    // worktree/build directory is inside a root-only home.
    let mut probe = Command::new("/proc/self/exe");
    probe
        .args([
            if inherited < 0 {
                "spawned_trees_are_killable_when_the_kernel_is_oom_protected"
            } else {
                "spawned_trees_preserve_a_positive_oom_policy"
            },
            "--nocapture",
            "--test-threads=1",
            "--include-ignored",
        ])
        .env(ENV, "1");
    let adjustment = format!("{inherited}\n");
    unsafe {
        probe.pre_exec(move || {
            let fd = libc::open(
                c"/proc/self/oom_score_adj".as_ptr(),
                libc::O_WRONLY | libc::O_CLOEXEC,
            );
            if fd < 0 {
                return Err(std::io::Error::last_os_error());
            }
            let written = libc::write(fd, adjustment.as_ptr().cast(), adjustment.len());
            let error = std::io::Error::last_os_error();
            libc::close(fd);
            if written != adjustment.len() as isize {
                return Err(error);
            }
            // A privileged launcher establishes oom_score_adj_min. Drop all
            // uid/gid privilege so lowering a positive policy really is denied.
            if inherited >= 0
                && libc::geteuid() == 0
                && (libc::setgroups(0, std::ptr::null()) != 0
                    || libc::setgid(65534) != 0
                    || libc::setuid(65534) != 0)
            {
                return Err(std::io::Error::last_os_error());
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
        "kernel must launch killable trees without lowering its policy"
    );
}
