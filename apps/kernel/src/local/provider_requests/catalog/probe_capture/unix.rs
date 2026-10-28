use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, Command, ExitStatus};

#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod pipe_owners;

#[cfg(target_os = "linux")]
#[path = "descendants-linux.rs"]
mod descendants;
#[cfg(target_os = "macos")]
#[path = "descendants-macos.rs"]
mod descendants;
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
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    descendants: Vec<descendants::Descendant>,
}

impl Process {
    pub(super) fn spawn(command: &mut Command) -> io::Result<Self> {
        // MP-08 / MP-10 / MP-11: a separate session prevents unrelated
        // siblings joining the group used to discover reparented helpers.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        let process = Self {
            child: command.spawn()?,
            stopped: false,
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            descendants: Vec::new(),
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

    pub(super) fn observe_descendants(&mut self) -> io::Result<()> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        descendants::observe(Some(self.child.id()), &mut self.descendants, false)?;
        Ok(())
    }

    // MP-08 / MP-10 / MP-11: retain the leader PID until owned-group discovery
    // completes. Reaping it first loses the boundary for reparented helpers.
    pub(super) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let pid = signal_pid(self.child.id().into())?;
            let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
            if unsafe {
                libc::waitid(
                    libc::P_PID,
                    pid as _,
                    info.as_mut_ptr(),
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            } < 0
            {
                return Err(io::Error::last_os_error());
            }
            let info = unsafe { info.assume_init() };
            #[cfg(target_os = "linux")]
            let (pid, status) = unsafe { (info.si_pid(), info.si_status()) };
            #[cfg(target_os = "macos")]
            let (pid, status) = (info.si_pid, info.si_status);
            if pid == 0 {
                return Ok(None);
            }
            return Ok(Some(ExitStatus::from_raw(
                if info.si_code == libc::CLD_EXITED {
                    status << 8
                } else if info.si_code == libc::CLD_DUMPED {
                    status | 0x80
                } else {
                    status
                },
            )));
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        self.child.try_wait()
    }

    pub(super) fn stop(&mut self) -> io::Result<()> {
        if self.stopped {
            return Ok(());
        }
        signal_pid(self.child.id().into())?;
        let running = self.try_wait()?.is_none();
        // MP-08 / MP-10 / MP-11: retain descendants independently of their
        // streams. Freeze verified owners before the last ancestry scans so
        // cleanup cannot race new children from a running launcher.
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let tracked = (|| {
            if running {
                let pid = signal_pid(self.child.id().into())?;
                unsafe {
                    libc::kill(pid, libc::SIGSTOP);
                }
            }
            let mut result = Ok(());
            for pass in 0..3 {
                if let Err(error) =
                    descendants::observe(Some(self.child.id()), &mut self.descendants, pass == 0)
                {
                    result = Err(error);
                }
                for child in &self.descendants {
                    if let Err(error) = child.signal(libc::SIGSTOP) {
                        result = Err(error);
                    }
                }
            }
            for child in &self.descendants {
                if let Err(error) = child.signal(libc::SIGKILL) {
                    result = Err(error);
                }
            }
            result
        })();
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let tracked = Ok(());
        if running {
            signal_pid(self.child.id().into())?;
            let _ = self.child.kill();
        }
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
        // Pipe ownership pins escaped writers. Do not signal a numeric process
        // group: an exited launcher may already have lost that group's ownership.
        if self.child.id() <= 1 {
            return Err(io::Error::other("refusing to signal a protected process"));
        }
        let _ = self.child.kill();
        let reaped = self.child.wait().map(|_| ());
        self.stopped = true;
        tracked.and(cleanup).and(reaped)
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
