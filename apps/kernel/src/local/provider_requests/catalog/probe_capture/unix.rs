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
        // Exact pipe ownership still identifies escaped writers after setsid
        // or reparenting. Names and parent PIDs alone cannot establish ownership.
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let cleanup = pipe_owners::terminate(&self.pipe_fds());
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let cleanup = Ok(());
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        let _ = self.child.kill();
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
