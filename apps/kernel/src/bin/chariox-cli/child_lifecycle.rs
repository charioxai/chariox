//! MP-08/MP-11: the launcher owns its TUI until the child has been reaped.
use std::process::{Command, ExitStatus};

#[cfg(unix)]
pub(super) fn run(command: Command) -> std::io::Result<ExitStatus> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(run_unix(command))
}

#[cfg(unix)]
async fn run_unix(mut command: Command) -> std::io::Result<ExitStatus> {
    use tokio::signal::unix::{signal, SignalKind};

    // Install before spawning so a signal cannot orphan a just-started child.
    let mut terminate = signal(SignalKind::terminate())?;
    let mut hangup = signal(SignalKind::hangup())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    let parent_pid = std::process::id();
    command.env("CHARIOX_CLI_LAUNCHER_PID", parent_pid.to_string());
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        // Only async-signal-safe libc calls between fork and exec. Check the
        // parent again after prctl to close the parent-death registration race.
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::getppid() as u32 != parent_pid {
                    return Err(std::io::Error::from_raw_os_error(libc::ECHILD));
                }
                Ok(())
            });
        }
    }
    let mut child = tokio::process::Command::from(command)
        .kill_on_drop(true)
        .spawn()?;
    let mut received = tokio::select! {
        status = child.wait() => return status,
        _ = terminate.recv() => libc::SIGTERM,
        _ = hangup.recv() => libc::SIGHUP,
        _ = interrupt.recv() => libc::SIGINT,
    };
    loop {
        // Signal only this retained, unreaped child, never a process group.
        // wait() cannot reap concurrently with this synchronous section.
        if let Some(pid) = child.id().and_then(|pid| i32::try_from(pid).ok()) {
            if pid > 1 && unsafe { libc::kill(pid, received) } != 0 {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::ESRCH) {
                    kill_and_reap(&mut child).await?;
                    return Err(error);
                }
            }
        }
        if received != libc::SIGINT {
            return match tokio::time::timeout(std::time::Duration::from_millis(1500), child.wait())
                .await
            {
                Ok(status) => status,
                Err(_) => kill_and_reap(&mut child).await,
            };
        }
        // SIGINT runs draft/archive/detach RPC cleanup under the client's own
        // deadlines. TERM/HUP still provide bounded settlement if it stalls.
        received = tokio::select! {
            status = child.wait() => return status,
            _ = terminate.recv() => libc::SIGTERM,
            _ = hangup.recv() => libc::SIGHUP,
            _ = interrupt.recv() => libc::SIGINT,
        };
    }
}

#[cfg(unix)]
async fn kill_and_reap(child: &mut tokio::process::Child) -> std::io::Result<ExitStatus> {
    if let Some(pid) = child.id() {
        if i32::try_from(pid).ok().is_none_or(|pid| pid <= 1) {
            return Err(std::io::Error::other("invalid CLI child PID"));
        }
        child.kill().await?;
    }
    child.wait().await
}

#[cfg(not(unix))]
pub(super) fn run(mut command: Command) -> std::io::Result<ExitStatus> {
    command.status()
}

#[cfg(all(test, unix))]
mod tests {
    use super::run;
    use std::process::Command;
    use std::sync::Mutex;

    // Tokio broadcasts Unix signals to every runtime in this test process.
    static SIGNAL_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn reaps_child_and_preserves_its_exit_status() {
        let _guard = SIGNAL_TEST_LOCK.lock().unwrap();
        let mut command = Command::new("sh");
        command.args(["-c", "trap 'exit 99' INT; sleep 1; exit 7"]);
        assert_eq!(run(command).unwrap().code(), Some(7));
    }

    #[test]
    fn forwards_interrupt_and_waits_for_the_child_handler() {
        let _guard = SIGNAL_TEST_LOCK.lock().unwrap();
        let parent = std::process::id();
        assert!(parent > 1);
        let mut command = Command::new("sh");
        command.args([
            "-c",
            "trap 'exit 42' INT; test \"$PPID\" -eq \"$1\" && test \"$1\" -gt 1 || exit 43; sleep 0.1; kill -s INT \"$1\"; sleep 1; exit 44",
            "owned-signal-child",
            &parent.to_string(),
        ]);
        assert_eq!(run(command).unwrap().code(), Some(42));
    }

    #[test]
    fn interrupt_allows_cleanup_beyond_the_termination_deadline() {
        let _guard = SIGNAL_TEST_LOCK.lock().unwrap();
        let parent = std::process::id();
        assert!(parent > 1);
        let mut command = Command::new("sh");
        command.args([
            "-c",
            "trap 'sleep 2; exit 42' INT; test \"$PPID\" -eq \"$1\" && test \"$1\" -gt 1 || exit 43; sleep 0.1; kill -s INT \"$1\"; sleep 1; exit 44",
            "owned-signal-child",
            &parent.to_string(),
        ]);
        assert_eq!(run(command).unwrap().code(), Some(42));
    }

    #[test]
    fn termination_can_still_stop_interrupt_cleanup() {
        let _guard = SIGNAL_TEST_LOCK.lock().unwrap();
        let parent = std::process::id();
        assert!(parent > 1);
        let mut command = Command::new("sh");
        command.args([
            "-c",
            "trap '' INT; trap 'exit 43' TERM; test \"$PPID\" -eq \"$1\" && test \"$1\" -gt 1 || exit 44; sleep 0.1; kill -s INT \"$1\"; sleep 1; kill -s TERM \"$1\"; sleep 1; exit 45",
            "owned-signal-child",
            &parent.to_string(),
        ]);
        assert_eq!(run(command).unwrap().code(), Some(43));
    }

    #[test]
    fn child_receives_exact_launcher_parent_identity() {
        let _guard = SIGNAL_TEST_LOCK.lock().unwrap();
        let mut command = Command::new("sh");
        command.args(["-c", "test \"$CHARIOX_CLI_LAUNCHER_PID\" = \"$PPID\""]);
        assert!(run(command).unwrap().success());
    }
}
