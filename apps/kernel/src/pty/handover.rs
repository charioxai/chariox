//! A PTY that outlives one kernel program: its reader can be held at a chunk
//! boundary for a hot reload, and the next program adopts the master and the
//! child it inherited.

use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// What the next program needs to adopt a running PTY. The master fd stays open
/// across `exec`; the child stays this process's child because the PID is kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PtyHandover {
    pub master_fd: i32,
    pub pid: u32,
    pub process_group: Option<i32>,
    /// The memory domain the child joined, which it keeps.
    pub cgroup: Option<std::path::PathBuf>,
    /// The agent whose tools may leave jobs behind, so the new program hands them on the same way.
    pub agent: Option<super::PtyAgent>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Gate {
    #[default]
    Open,
    HoldRequested,
    Held,
    Finished,
}

/// Lets a hot reload stop a PTY reader between two reads, so no byte is read
/// into memory that the next program would not receive.
#[derive(Debug, Default)]
pub(crate) struct ReaderGate {
    state: Mutex<Gate>,
    changed: Condvar,
}

impl ReaderGate {
    fn lock(&self) -> std::sync::MutexGuard<'_, Gate> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Reader side, between reads: wait here while a hot reload holds the output.
    pub(crate) fn checkpoint(&self) {
        let mut state = self.lock();
        if *state == Gate::HoldRequested {
            *state = Gate::Held;
            self.changed.notify_all();
        }
        while *state == Gate::Held {
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    pub(crate) fn finish(&self) {
        *self.lock() = Gate::Finished;
        self.changed.notify_all();
    }

    /// True once the reader is held (or has ended) within `timeout`.
    pub(crate) fn hold(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut state = self.lock();
        if *state == Gate::Open {
            *state = Gate::HoldRequested;
        }
        while *state == Gate::HoldRequested {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            state = self
                .changed
                .wait_timeout(state, deadline - now)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
        true
    }

    pub(crate) fn release(&self) {
        let mut state = self.lock();
        if matches!(*state, Gate::HoldRequested | Gate::Held) {
            *state = Gate::Open;
        }
        self.changed.notify_all();
    }
}

#[cfg(unix)]
pub(super) use adopted::{AdoptedChild, AdoptedMaster};

#[cfg(unix)]
mod adopted {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    use portable_pty::{Child, ChildKiller, ExitStatus, MasterPty, PtySize};

    /// The master end of a PTY inherited from the previous kernel program.
    pub(in crate::pty) struct AdoptedMaster {
        fd: OwnedFd,
    }

    impl AdoptedMaster {
        /// # Safety
        /// `fd` must be an open PTY master that nothing else owns.
        pub(in crate::pty) unsafe fn from_raw(fd: i32) -> Self {
            Self {
                fd: unsafe { OwnedFd::from_raw_fd(fd) },
            }
        }

        fn file(&self) -> Result<std::fs::File, anyhow::Error> {
            Ok(self.fd.try_clone()?.into())
        }
    }

    impl MasterPty for AdoptedMaster {
        fn resize(&self, size: PtySize) -> Result<(), anyhow::Error> {
            let size = libc::winsize {
                ws_row: size.rows,
                ws_col: size.cols,
                ws_xpixel: size.pixel_width,
                ws_ypixel: size.pixel_height,
            };
            if unsafe { libc::ioctl(self.fd.as_raw_fd(), libc::TIOCSWINSZ, &size) } != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            Ok(())
        }

        fn get_size(&self) -> Result<PtySize, anyhow::Error> {
            let mut size: libc::winsize = unsafe { std::mem::zeroed() };
            if unsafe { libc::ioctl(self.fd.as_raw_fd(), libc::TIOCGWINSZ, &mut size) } != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            Ok(PtySize {
                rows: size.ws_row,
                cols: size.ws_col,
                pixel_width: size.ws_xpixel,
                pixel_height: size.ws_ypixel,
            })
        }

        fn try_clone_reader(&self) -> Result<Box<dyn std::io::Read + Send>, anyhow::Error> {
            Ok(Box::new(self.file()?))
        }

        fn take_writer(&self) -> Result<Box<dyn std::io::Write + Send>, anyhow::Error> {
            Ok(Box::new(self.file()?))
        }

        fn process_group_leader(&self) -> Option<libc::pid_t> {
            match unsafe { libc::tcgetpgrp(self.fd.as_raw_fd()) } {
                pid if pid > 0 => Some(pid),
                _ => None,
            }
        }

        fn as_raw_fd(&self) -> Option<std::os::fd::RawFd> {
            Some(self.fd.as_raw_fd())
        }
    }

    /// A PTY child started by the previous kernel program: still this
    /// process's child, so it is waited for and signalled by PID.
    #[derive(Debug)]
    pub(in crate::pty) struct AdoptedChild {
        pid: u32,
        status: Option<ExitStatus>,
    }

    impl AdoptedChild {
        pub(in crate::pty) fn new(pid: u32) -> Self {
            Self { pid, status: None }
        }

        fn reap(&mut self, options: libc::c_int) -> std::io::Result<Option<ExitStatus>> {
            if let Some(status) = &self.status {
                return Ok(Some(status.clone()));
            }
            let mut raw = 0;
            let reaped = unsafe { libc::waitpid(self.pid as libc::pid_t, &mut raw, options) };
            if reaped < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if reaped == 0 {
                return Ok(None);
            }
            let status = if libc::WIFSIGNALED(raw) {
                let signal = libc::WTERMSIG(raw);
                let name = unsafe { std::ffi::CStr::from_ptr(libc::strsignal(signal)) };
                ExitStatus::with_signal(&name.to_string_lossy())
            } else {
                ExitStatus::with_exit_code(libc::WEXITSTATUS(raw) as u32)
            };
            self.status = Some(status.clone());
            Ok(Some(status))
        }
    }

    impl ChildKiller for AdoptedChild {
        fn kill(&mut self) -> std::io::Result<()> {
            if self.status.is_some() {
                return Ok(());
            }
            if unsafe { libc::kill(self.pid as libc::pid_t, libc::SIGKILL) } != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        }

        fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
            Box::new(AdoptedChild::new(self.pid))
        }
    }

    impl Child for AdoptedChild {
        fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
            self.reap(libc::WNOHANG)
        }

        fn wait(&mut self) -> std::io::Result<ExitStatus> {
            loop {
                match self.reap(0) {
                    Ok(Some(status)) => return Ok(status),
                    Ok(None) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(error) => return Err(error),
                }
            }
        }

        fn process_id(&self) -> Option<u32> {
            Some(self.pid)
        }
    }
}
