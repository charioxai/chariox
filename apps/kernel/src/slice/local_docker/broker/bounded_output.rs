//! Small trusted local helpers: bounded output without reader threads, and
//! process-group termination before reaping releases the leader PID/PGID.
use std::io::{self, Read};
use std::mem::MaybeUninit;
use std::os::{fd::AsRawFd, unix::process::CommandExt};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::time::{Duration, Instant};

const LIMIT: usize = 64 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(10);

struct OwnedChild {
    child: Child,
    finished: bool,
}

impl OwnedChild {
    fn observe(&self) -> io::Result<bool> {
        let mut info = MaybeUninit::<libc::siginfo_t>::zeroed();
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                self.child.id() as libc::id_t,
                info.as_mut_ptr(),
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result == 0 {
            Ok(unsafe { info.assume_init().si_pid() } != 0)
        } else {
            Err(io::Error::last_os_error())
        }
    }

    fn finish(&mut self) -> io::Result<ExitStatus> {
        self.finish_with(|pid| unsafe {
            libc::kill(-pid, libc::SIGKILL);
            // Also reaches the direct child if a trusted helper changed group.
            libc::kill(pid, libc::SIGKILL);
        })
    }

    fn finish_with(&mut self, signal: impl FnOnce(libc::pid_t)) -> io::Result<ExitStatus> {
        loop {
            match self.observe() {
                Ok(_) => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    // In particular ECHILD means the PID may have been reused.
                    // Never signal it, its group, or retry from Drop afterward.
                    self.finished = true;
                    return Err(error);
                }
            }
        }
        // waitid(WNOWAIT) preserves the leader as our unreaped child, even when
        // exited. Terminate descendants while that PID/PGID is still reserved.
        signal(self.child.id() as libc::pid_t);
        let status = self.child.wait();
        self.finished = true;
        status
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.finish();
        }
    }
}

fn require_wait_ownership() -> io::Result<()> {
    let mut signal = MaybeUninit::<libc::sigaction>::zeroed();
    if unsafe { libc::sigaction(libc::SIGCHLD, std::ptr::null(), signal.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let signal = unsafe { signal.assume_init() };
    if signal.sa_sigaction == libc::SIG_IGN || signal.sa_flags & libc::SA_NOCLDWAIT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "migration helper wait ownership unavailable",
        ));
    }
    Ok(())
}

fn nonblocking(stream: &impl AsRawFd) -> io::Result<()> {
    let fd = stream.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn drain(stream: &mut impl Read, bytes: &mut Vec<u8>) -> io::Result<bool> {
    let mut buffer = [0; 4096];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(count) => {
                if bytes.len() + count > LIMIT {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "migration command output exceeds its bound",
                    ));
                }
                bytes.extend_from_slice(&buffer[..count]);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
            Err(error) => return Err(error),
        }
    }
}

pub(super) fn run(command: &mut Command, duration: Duration) -> io::Result<Output> {
    require_wait_ownership()?;
    let mut owner = OwnedChild {
        child: command
            .process_group(0)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?,
        finished: false,
    };
    let mut stdout_pipe = owner.child.stdout.take().expect("stdout was piped");
    let mut stderr_pipe = owner.child.stderr.take().expect("stderr was piped");
    nonblocking(&stdout_pipe)?;
    nonblocking(&stderr_pipe)?;
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let started = Instant::now();
    loop {
        drain(&mut stdout_pipe, &mut stdout)?;
        drain(&mut stderr_pipe, &mut stderr)?;
        match owner.observe() {
            Ok(true) => break,
            Ok(false) if started.elapsed() >= duration => {
                owner.finish()?;
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "migration command timed out",
                ));
            }
            Ok(false) => {}
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                owner.finished = true;
                return Err(error);
            }
        }
        std::thread::sleep(POLL_INTERVAL.min(duration.saturating_sub(started.elapsed())));
    }
    let status = owner.finish()?;
    // Pipes inherited by descendants that left the group must not block this
    // helper forever. Closing our pipe handles does not signal unknown PIDs.
    let draining = Instant::now();
    loop {
        let stdout_closed = drain(&mut stdout_pipe, &mut stdout)?;
        let stderr_closed = drain(&mut stderr_pipe, &mut stderr)?;
        if stdout_closed && stderr_closed {
            return Ok(Output {
                status,
                stdout,
                stderr,
            });
        }
        if draining.elapsed() >= Duration::from_secs(1) {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "migration helper descendant retained its output pipe",
            ));
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn exited_leader_is_reaped_only_after_descendant_pipe_cleanup() {
        let started = Instant::now();
        let result = run(
            Command::new("/bin/sh").args(["-c", "sleep 30 & printf 'ready'; exit 7"]),
            Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(result.status.code(), Some(7));
        assert_eq!(result.stdout, b"ready");
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn timeout_terminates_the_owned_group_without_waiting_for_sleep() {
        let started = Instant::now();
        let error = run(
            Command::new("/bin/sh").args(["-c", "sleep 30 & wait"]),
            Duration::from_millis(30),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn lost_wait_authority_never_signals_the_former_pid_or_group() {
        let child = Command::new("/bin/sh")
            .args(["-c", "exit 0"])
            .process_group(0)
            .spawn()
            .unwrap();
        let mut owner = OwnedChild {
            child,
            finished: false,
        };
        // Model a different component reaping the leader outside this owner.
        let mut status = 0;
        assert_eq!(
            unsafe { libc::waitpid(owner.child.id() as libc::pid_t, &mut status, 0) },
            owner.child.id() as libc::pid_t
        );
        let signaled = Cell::new(false);
        let error = owner.finish_with(|_| signaled.set(true)).unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::ECHILD));
        assert!(!signaled.get());
        assert!(owner.finished);
    }

    #[test]
    fn excessive_output_is_bounded_and_terminates_the_helper() {
        let error = run(
            Command::new("/bin/sh").args(["-c", "while :; do printf '0123456789abcdef'; done"]),
            Duration::from_secs(2),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
