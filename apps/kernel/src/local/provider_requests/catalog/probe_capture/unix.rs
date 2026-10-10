use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command};

#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod pipe_owners;
#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod pipe_owners;

pub(super) trait Pipe: AsRawFd {}
impl<T: AsRawFd> Pipe for T {}

pub(super) fn available(_pipe: &impl Pipe) -> io::Result<usize> {
    // O_NONBLOCK makes the read itself the readiness check on Unix.
    Ok(8192)
}

pub(super) struct Process {
    pub(super) child: Child,
    stopped: bool,
}

impl Process {
    pub(super) fn spawn(command: &mut Command) -> io::Result<Self> {
        command.process_group(0);
        let process = Self {
            child: command.spawn()?,
            stopped: false,
        };
        for fd in process.pipe_fds() {
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(process)
    }

    fn pipe_fds(&self) -> [RawFd; 2] {
        [
            self.child
                .stdout
                .as_ref()
                .expect("captured stdout")
                .as_raw_fd(),
            self.child
                .stderr
                .as_ref()
                .expect("captured stderr")
                .as_raw_fd(),
        ]
    }

    pub(super) fn stop(&mut self) -> io::Result<()> {
        if self.stopped {
            return Ok(());
        }
        signal_pid(self.child.id().into())?;
        // Exact pipe ownership still identifies escaped writers after setsid
        // or reparenting. Names and parent PIDs alone cannot establish ownership.
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let cleanup = if pipe_writers_open(&self.pipe_fds())? {
            pipe_owners::terminate(&self.pipe_fds())
        } else {
            Ok(())
        };
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let cleanup = Ok(());
        // MP-08 / MP-10 / MP-11: never signal an unverified process group.
        // The Child handle owns this unreaped PID; pipe ownership pins other writers.
        if self.child.try_wait()?.is_none() {
            signal_pid(self.child.id().into())?;
            let _ = self.child.kill();
        }
        let reaped = self.child.wait().map(|_| ());
        self.stopped = true;
        cleanup.and(reaped)
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

// MP-08 / MP-10 / MP-11: a signal target must fit a positive process ID.
pub(super) fn signal_pid(pid: i64) -> io::Result<i32> {
    if pid > 1 && pid <= i32::MAX as i64 {
        Ok(pid as i32)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unsafe probe signal target",
        ))
    }
}
#[cfg(test)]
mod signal_tests {
    #[test]
    fn rejects_unsafe_probe_signal_targets() {
        for pid in [-1, 0, 1, i32::MAX as i64 + 1] {
            assert!(
                super::signal_pid(pid).is_err(),
                "unsafe PID {pid} must never reach a signal syscall"
            );
        }
        assert_eq!(super::signal_pid(32).unwrap(), 32);
    }
}

// MP-08 / MP-10 / MP-11: POLLHUP means every writer is closed, even with buffered data.
// Ordinary successful version probes need no process-table enumeration.
fn pipe_writers_open(fds: &[RawFd]) -> io::Result<bool> {
    for fd in fds {
        let mut poll = libc::pollfd {
            fd: *fd,
            events: libc::POLLIN,
            revents: 0,
        };
        if unsafe { libc::poll(&mut poll, 1, 0) } < 0 {
            return Err(io::Error::last_os_error());
        }
        if poll.revents & libc::POLLNVAL != 0 {
            return Err(io::Error::from_raw_os_error(libc::EBADF));
        }
        if poll.revents & libc::POLLHUP == 0 {
            return Ok(true);
        }
    }
    Ok(false)
}
