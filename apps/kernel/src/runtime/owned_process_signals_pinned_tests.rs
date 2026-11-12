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
        || unsafe { libc::prctl(libc::PR_SET_SECCOMP, libc::SECCOMP_MODE_FILTER, &program) }
            != 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[test]
fn individual_owned_signals_do_not_use_reusable_numeric_pids() {
    let mut outcomes = Vec::new();
    for operation in ["signal_owned_processes", "kill_child", "kill_owned_processes"] {
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
