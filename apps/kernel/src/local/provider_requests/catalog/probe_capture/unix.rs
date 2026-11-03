use std::io;
use std::os::fd::{AsFd, AsRawFd, RawFd};
use std::time::{Duration, Instant};
#[path = "unix/reaper.rs"]
mod reaper;
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
    child: Option<Child>,
    stdout: Option<std::process::ChildStdout>,
    stderr: Option<std::process::ChildStderr>,
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
            child: Some(command.spawn()?),
            stdout: None,
            stderr: None,
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

    fn child(&self) -> &Child {
        self.child.as_ref().expect("owned probe child")
    }
    pub(super) fn child_mut(&mut self) -> &mut Child {
        self.child.as_mut().expect("owned probe child")
    }

    fn pipe_fds(&self) -> [RawFd; 2] {
        [
            self.child()
                .stdout
                .as_ref()
                .expect("captured stdout")
                .as_raw_fd(),
            self.child()
                .stderr
                .as_ref()
                .expect("captured stderr")
                .as_raw_fd(),
        ]
    }

    pub(super) fn observe_descendants(&mut self) -> io::Result<()> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        descendants::observe(Some(self.child().id()), &mut self.descendants, false)?;
        Ok(())
    }

    // MP-08 / MP-10 / MP-11: retain the leader PID until owned-group discovery
    // completes. Reaping it first loses the boundary for reparented helpers.
    pub(super) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let pid = signal_pid(self.child().id().into())?;
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
            Ok(Some(ExitStatus::from_raw(
                if info.si_code == libc::CLD_EXITED {
                    status << 8
                } else if info.si_code == libc::CLD_DUMPED {
                    status | 0x80
                } else {
                    status
                },
            )))
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        self.child_mut().try_wait()
    }

    pub(super) fn stdout_mut(&mut self) -> &mut std::process::ChildStdout {
        if self.stopped {
            self.stdout.as_mut().expect("captured stdout")
        } else {
            self.child_mut().stdout.as_mut().expect("captured stdout")
        }
    }
    pub(super) fn stderr_mut(&mut self) -> &mut std::process::ChildStderr {
        if self.stopped {
            self.stderr.as_mut().expect("captured stderr")
        } else {
            self.child_mut().stderr.as_mut().expect("captured stderr")
        }
    }
    pub(super) fn stop(&mut self) -> io::Result<()> {
        self.stop_with_termination(|child| child.kill())
    }
    fn stop_with_termination(
        &mut self,
        terminate: impl FnOnce(&mut Child) -> io::Result<()> + Send + 'static,
    ) -> io::Result<()> {
        if self.stopped {
            return Ok(());
        }
        let pid = signal_pid(self.child().id().into())?;
        let running = self.try_wait()?.is_none();
        if running {
            // Freeze the owned unreaped leader before deferred discovery. No
            // unverified group signal is used, and no filesystem scan occurs.
            unsafe {
                libc::kill(pid, libc::SIGSTOP);
            }
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        for child in &self.descendants {
            let _ = child.signal(libc::SIGSTOP);
        }
        let pipes = [
            self.child()
                .stdout
                .as_ref()
                .expect("captured stdout")
                .as_fd()
                .try_clone_to_owned(),
            self.child()
                .stderr
                .as_ref()
                .expect("captured stderr")
                .as_fd()
                .try_clone_to_owned(),
        ]
        .into_iter()
        .collect::<io::Result<Vec<_>>>();
        self.stopped = true;
        let mut child = self.child.take().expect("owned probe child");
        let (pipes, error) = match pipes {
            Ok(pipes) => {
                self.stdout = child.stdout.take();
                self.stderr = child.stderr.take();
                (pipes, None)
            }
            Err(error) => (Vec::new(), Some(error)),
        };
        let done = reaper::defer(reaper::Pending {
            child,
            pipes,
            terminate: running.then(|| Box::new(terminate) as reaper::Termination),
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            descendants: std::mem::take(&mut self.descendants),
            cleaned: false,
        });
        // Discovery and pipe-owner scans can be slow on a busy host too.
        // Keep them outside deadline settlement, alongside any delayed reap.
        let deadline = Instant::now() + Duration::from_millis(100);
        while !done.load(std::sync::atomic::Ordering::Acquire) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        error.map_or(Ok(()), Err)
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

#[cfg(test)]
mod deadline_tests {
    use super::*;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    // MP-08 / MP-10 / MP-11: termination can be requested while a worker
    // remains unreapable. An injected delayed termination acknowledgement
    // models that seam; it is supplementary to real mounted-I/O acceptance.
    #[test]
    fn review0634_settlement_does_not_wait_for_termination_acknowledgement() {
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "exec sleep 1 >/dev/null 2>&1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut process = Process::spawn(&mut command).unwrap();
        let pid = process.child().id();
        let close_deadline = Instant::now() + Duration::from_secs(1);
        while pipe_writers_open(&process.pipe_fds()).unwrap() && Instant::now() < close_deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!pipe_writers_open(&process.pipe_fds()).unwrap());
        let began = Instant::now();
        let requested = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let observed = requested.clone();
        process
            .stop_with_termination(move |child| {
                observed.store(true, std::sync::atomic::Ordering::Release);
                // Resume the owned leader that cleanup froze; leave it alive
                // until sleep finishes, as if SIGKILL cannot yet complete I/O.
                let pid = signal_pid(child.id().into())?;
                if unsafe { libc::kill(pid, libc::SIGCONT) } < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            })
            .unwrap();
        let elapsed = began.elapsed();
        drop(process);
        assert!(
            reaper::owns_child(pid),
            "kernel lost the unreaped worker's ownership"
        );
        // Ensure the fixture's outstanding child eventually exits on both
        // sources, even when the settlement latency assertion is RED.
        let deadline = Instant::now() + Duration::from_secs(3);
        while reaper::owns_child(pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            !reaper::owns_child(pid),
            "owned delayed worker was not reaped"
        );
        assert!(requested.load(std::sync::atomic::Ordering::Acquire));
        assert!(!crate::runtime::process_health::process_running(pid));
        assert!(
            elapsed < Duration::from_millis(300),
            "operation waited for reap: {elapsed:?}"
        );
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn r6_environment_timeout_discovers_redirected_session_after_last_scan() {
        use std::os::fd::{FromRawFd, OwnedFd};
        let root = crate::test_support::TestWorktree::new("r6-late-probe-fork");
        let ready = root.path().join("ready");
        let go = root.path().join("go");
        let escaped = root.path().join("escaped");
        let mut command = Command::new("/usr/bin/python3");
        command
            .args([
                "-c",
                r#"
import os,sys,time
ready,go,escaped=sys.argv[1:]
open(ready,'w').close()
while not os.path.exists(go): time.sleep(.005)
if os.fork()==0:
    os.setsid()
    null=os.open('/dev/null',os.O_RDWR)
    os.dup2(null,1); os.dup2(null,2)
    with open(escaped+'.tmp','w') as file: file.write(str(os.getpid()))
    os.rename(escaped+'.tmp',escaped)
    time.sleep(30)
else:
    time.sleep(30)
"#,
            ])
            .args([&ready, &go, &escaped])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut process = Process::spawn(&mut command).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready.exists() {
            assert!(Instant::now() < deadline, "probe must reach fork barrier");
            std::thread::sleep(Duration::from_millis(5));
        }
        process.observe_descendants().unwrap();
        std::fs::write(go, "fork after the final observation").unwrap();
        while !escaped.exists() {
            assert!(Instant::now() < deadline, "late helper must start");
            std::thread::sleep(Duration::from_millis(5));
        }
        let pid = std::fs::read_to_string(escaped)
            .unwrap()
            .parse::<i32>()
            .unwrap();
        assert!(pid > 1 && pid != std::process::id() as i32);
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        assert!(fd >= 0);
        let fd = unsafe { OwnedFd::from_raw_fd(fd as i32) };
        process
            .stop_with_termination(|child| {
                child.kill()?;
                child.wait()?;
                Ok(())
            })
            .unwrap();
        let mut poll = libc::pollfd {
            fd: fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let deadline = Instant::now() + Duration::from_secs(3);
        while unsafe { libc::poll(&mut poll, 1, 0) } == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        let survived = unsafe { libc::poll(&mut poll, 1, 0) } == 0;
        if survived {
            // Clean up the exact PID owned by this fixture even on the red source.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
        assert!(
            !survived,
            "late redirected session escaped deferred probe cleanup"
        );
    }
}
