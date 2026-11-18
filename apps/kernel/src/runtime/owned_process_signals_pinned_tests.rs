use super::*;

/// Restrict this disposable thread only. The parent can always reap its child.
fn reject_numeric_pid_signals() -> io::Result<()> {
    let mut filter = [
        libc::sock_filter {
            code: (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16,
            jt: 0,
            jf: 0,
            k: 0,
        },
        libc::sock_filter {
            code: (libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16,
            jt: 0,
            jf: 1,
            k: libc::SYS_kill as u32,
        },
        libc::sock_filter {
            code: (libc::BPF_RET | libc::BPF_K) as u16,
            jt: 0,
            jf: 0,
            k: libc::SECCOMP_RET_ERRNO | libc::EPERM as u32,
        },
        libc::sock_filter {
            code: (libc::BPF_RET | libc::BPF_K) as u16,
            jt: 0,
            jf: 0,
            k: libc::SECCOMP_RET_ALLOW,
        },
    ];
    let program = libc::sock_fprog {
        len: filter.len() as u16,
        filter: filter.as_mut_ptr(),
    };
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0
        || unsafe { libc::prctl(libc::PR_SET_SECCOMP, libc::SECCOMP_MODE_FILTER, &program) } != 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[test]
fn individual_owned_signals_do_not_use_reusable_numeric_pids() {
    let mut outcomes = Vec::new();
    for operation in [
        "signal_owned_processes",
        "kill_child",
        "kill_owned_processes",
    ] {
        let mut child = crate::process_spawn::command("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let mut owner = OwnedProcessSignals::for_child(&child).unwrap();
        let result = std::thread::spawn(move || {
            reject_numeric_pid_signals()?;
            match operation {
                "signal_owned_processes" => owner.signal_owned_processes(libc::SIGSTOP),
                "kill_child" => owner.kill_child().map(|_| ()),
                _ => owner.kill_owned_processes(),
            }
        })
        .join();
        // Cleanup runs outside the restricted thread, including after a test failure.
        let _ = child.kill();
        child.wait().unwrap();
        outcomes.push((operation, result.unwrap()));
    }
    assert!(
        outcomes.iter().all(|(_, result)| result.is_ok()),
        "individual signals must address pinned process handles: {outcomes:?}"
    );
}

/// A job that left the command's tree (its own session, its parent gone) before anything looked
/// at the tree is still the command's: the launch claim in the command's environment owns it.
#[test]
fn a_job_that_left_the_tree_before_any_snapshot_is_ended_with_its_claimed_launch() {
    use std::io::Read as _;
    use std::os::unix::process::CommandExt as _;
    let mut command = crate::process_spawn::command("sh");
    command
        .args([
            "-c",
            "(setsid sh -c 'echo $$; exec sleep 60' &); exec sleep 60",
        ])
        .env(CLAIM_VARIABLE, "claimed-launch-test")
        .stdout(std::process::Stdio::piped());
    // SAFETY: setsid is async-signal-safe and runs before exec.
    unsafe {
        command.pre_exec(|| match libc::setsid() {
            -1 => Err(io::Error::last_os_error()),
            _ => Ok(()),
        });
    }
    let mut child = command.spawn().unwrap();
    let mut owner = OwnedProcessSignals::for_session_child(&child).unwrap();
    let mut line = Vec::new();
    let mut stdout = child.stdout.take().unwrap();
    let mut byte = [0];
    while stdout.read(&mut byte).unwrap() == 1 && byte[0] != b'\n' {
        line.push(byte[0]);
    }
    let orphan: i32 = String::from_utf8(line).unwrap().trim().parse().unwrap();
    // The subshell that started it is gone: it is no descendant and not in the command's session.
    std::thread::sleep(std::time::Duration::from_millis(300));
    let _ = owner.kill_group();
    let _ = owner.kill_owned_processes();
    let _ = child.wait();
    let ended = (0..100).any(|_| {
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::read_to_string(format!("/proc/{orphan}/stat")).map_or(true, |stat| {
            stat.rsplit_once(')')
                .is_some_and(|(_, rest)| rest.trim_start().starts_with('Z'))
        })
    });
    if !ended {
        // SAFETY: the live process this test read from its own child's output; never a group.
        unsafe { libc::kill(orphan, libc::SIGKILL) };
    }
    assert!(ended, "the detached job outlived its command's cleanup");
}
