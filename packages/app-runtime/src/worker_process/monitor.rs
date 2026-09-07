use super::record::CONTINUE;
use super::{PreparedWorker, WorkerError, WorkerExit, WorkerLimits};
use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    mem::MaybeUninit,
    os::{fd::AsRawFd, unix::net::UnixStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::SyncSender,
        Arc,
    },
    time::{Duration, Instant},
};

/// Owns both waitpid authority and reservations, including while unwinding.
pub(super) struct Child {
    pid: libc::pid_t,
    prepared: PreparedWorker,
    reaped: bool,
    lost_wait_authority: bool,
}
impl Child {
    pub fn new(pid: libc::pid_t, prepared: PreparedWorker) -> Self {
        Self {
            pid,
            prepared,
            reaped: false,
            lost_wait_authority: false,
        }
    }
    fn exited(&self) -> io::Result<bool> {
        let mut info = MaybeUninit::<libc::siginfo_t>::zeroed();
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                self.pid as libc::id_t,
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
    fn reap(&mut self) -> io::Result<i32> {
        // The unreaped direct child still owns this PID/PGID, avoiding PID reuse
        // races. Domain termination also reaches bubblewrap's nested session.
        self.prepared.domain.terminate(self.pid);
        if !self.lost_wait_authority {
            unsafe {
                libc::kill(-self.pid, libc::SIGKILL);
                libc::kill(self.pid, libc::SIGKILL);
            }
        }
        let mut status = 0;
        let result = loop {
            let result = unsafe { libc::waitpid(self.pid, &mut status, 0) };
            if result == self.pid {
                break Ok(status);
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                break Err(error);
            }
        };
        // The platform implementation cannot release the aggregate reservation
        // until all owned descendants are gone, even if direct wait failed.
        self.prepared.domain.reap_domain_blocking();
        self.reaped = true;
        result
    }
}
impl Drop for Child {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.reap();
        }
    }
}

struct Logs {
    tails: [VecDeque<u8>; 2],
    closed: [bool; 2],
    second: Instant,
    bytes: usize,
}
impl Logs {
    fn drain(
        &mut self,
        stream: &mut UnixStream,
        index: usize,
        limits: WorkerLimits,
    ) -> Result<(), WorkerError> {
        if self.second.elapsed() >= Duration::from_secs(1) {
            self.second = Instant::now();
            self.bytes = 0;
        }
        let mut buffer = [0; 8192];
        // Bound work per poll so a continuously writable log never starves
        // startup deadlines/cancellation, even below its rate allowance.
        for _ in 0..4 {
            match stream.read(&mut buffer) {
                Ok(0) => {
                    self.closed[index] = true;
                    break;
                }
                Ok(count) => {
                    self.bytes += count;
                    for byte in &buffer[..count] {
                        if self.tails[index].len() == limits.log_tail_bytes {
                            self.tails[index].pop_front();
                        }
                        self.tails[index].push_back(*byte);
                    }
                    if self.bytes > limits.log_bytes_per_second {
                        return Err(WorkerError::LogLimit);
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return Err(WorkerError::Io),
            }
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run(
    mut child: Child,
    mut control: UnixStream,
    mut stdout: UnixStream,
    mut stderr: UnixStream,
    wake: UnixStream,
    cancelled: Arc<AtomicBool>,
    record: Vec<u8>,
    ready: Vec<u8>,
    limits: WorkerLimits,
    deadline: Instant,
    started: SyncSender<Result<(), WorkerError>>,
) -> WorkerExit {
    let mut logs = Logs {
        tails: Default::default(),
        closed: [false; 2],
        second: Instant::now(),
        bytes: 0,
    };
    let mut record_sent = 0;
    let mut reply = Vec::with_capacity(ready.len() + 1);
    let mut verified = false;
    let mut continued = 0;
    let mut running = false;
    let failure = loop {
        if cancelled.load(Ordering::Acquire) {
            break Some(WorkerError::Cancelled);
        }
        if !running && Instant::now() >= deadline {
            break Some(WorkerError::StartupTimeout);
        }
        if let Err(error) = logs
            .drain(&mut stdout, 0, limits)
            .and_then(|_| logs.drain(&mut stderr, 1, limits))
        {
            break Some(error);
        }
        match child.exited() {
            Ok(true) => {
                break if running {
                    None
                } else {
                    Some(WorkerError::EarlyExit)
                }
            }
            Ok(false) => {}
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                // Another kernel component must never reap this owner's PID.
                // If that contract is violated, do not signal a recycled PID;
                // the separately owned resource domain still cleans its tree.
                if error.raw_os_error() == Some(libc::ECHILD) {
                    child.lost_wait_authority = true;
                }
                break Some(WorkerError::Io);
            }
        }
        if !running {
            if let Err(error) = write_some(&mut control, &record, &mut record_sent) {
                break Some(error);
            }
            if record_sent == record.len() && reply.len() < ready.len() {
                let mut buffer = [0; 512];
                let capacity = (ready.len() + 1 - reply.len()).min(buffer.len());
                match control.read(&mut buffer[..capacity]) {
                    Ok(0) => break Some(WorkerError::EarlyExit),
                    Ok(count) => {
                        reply.extend_from_slice(&buffer[..count]);
                        if !ready.starts_with(&reply) {
                            break Some(WorkerError::Identity);
                        }
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                        ) => {}
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset
                        ) =>
                    {
                        break Some(WorkerError::EarlyExit)
                    }
                    Err(_) => break Some(WorkerError::Io),
                }
            }
            if reply == ready {
                if !verified {
                    if child
                        .prepared
                        .domain
                        .verify_before_continue(child.pid)
                        .is_err()
                    {
                        break Some(WorkerError::ResourceDomain);
                    }
                    verified = true;
                }
                if cancelled.load(Ordering::Acquire) {
                    break Some(WorkerError::Cancelled);
                }
                if Instant::now() >= deadline {
                    break Some(WorkerError::StartupTimeout);
                }
                if let Err(error) = write_some(&mut control, CONTINUE, &mut continued) {
                    break Some(error);
                }
                if continued == CONTINUE.len() {
                    running = true;
                    let _ = control.shutdown(std::net::Shutdown::Both);
                    if started.send(Ok(())).is_err() {
                        break Some(WorkerError::Cancelled);
                    }
                }
            }
        }
        let control_events = if running {
            0
        } else if record_sent < record.len() || verified {
            libc::POLLOUT
        } else {
            libc::POLLIN
        };
        let mut poll = [
            libc::pollfd {
                fd: wake.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: if logs.closed[0] {
                    -1
                } else {
                    stdout.as_raw_fd()
                },
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: if logs.closed[1] {
                    -1
                } else {
                    stderr.as_raw_fd()
                },
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: if running { -1 } else { control.as_raw_fd() },
                events: control_events,
                revents: 0,
            },
        ];
        let result = unsafe { libc::poll(poll.as_mut_ptr(), poll.len() as libc::nfds_t, 25) };
        if result < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            break Some(WorkerError::Io);
        }
    };
    if !running {
        let _ = started.send(Err(failure.unwrap_or(WorkerError::EarlyExit)));
    }
    let status = child.reap();
    // One bounded final drain retains termination diagnostics without waiting
    // on a descendant that holds stdout open.
    let _ = logs.drain(&mut stdout, 0, limits);
    let _ = logs.drain(&mut stderr, 1, limits);
    let (code, signal) = status
        .as_ref()
        .map(|status| {
            (
                libc::WIFEXITED(*status).then(|| libc::WEXITSTATUS(*status)),
                libc::WIFSIGNALED(*status).then(|| libc::WTERMSIG(*status)),
            )
        })
        .unwrap_or_default();
    WorkerExit {
        code,
        signal,
        failure: failure.or_else(|| status.err().map(|_| WorkerError::Io)),
        stdout_tail: logs.tails[0].iter().copied().collect(),
        stderr_tail: logs.tails[1].iter().copied().collect(),
    }
}

fn write_some(
    stream: &mut UnixStream,
    bytes: &[u8],
    offset: &mut usize,
) -> Result<(), WorkerError> {
    if *offset == bytes.len() {
        return Ok(());
    }
    match stream.write(&bytes[*offset..]) {
        Ok(0) => Err(WorkerError::Io),
        Ok(count) => {
            *offset += count;
            Ok(())
        }
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) =>
        {
            Ok(())
        }
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset
            ) =>
        {
            Err(WorkerError::EarlyExit)
        }
        Err(_) => Err(WorkerError::Io),
    }
}
