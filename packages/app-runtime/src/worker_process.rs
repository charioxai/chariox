//! Blocking ownership of a native App worker and its existing SDK channel.
//!
//! This lifecycle component has **no public preparation constructor**. The
//! enrolled runtime verifier and platform provisioner must supply the held,
//! pinned objects and resource domain; that production integration is pending.
//! A private type or a native identity reply does not establish confinement.
//!
//! Spawn, wait, shutdown and Drop are blocking. The kernel must own this handle
//! on its bounded blocking service, never drop it on an async coordinator. A
//! cancellation clone is nonblocking; cancellation does not release admission.

mod monitor;
mod record;
mod spawn;

use crate::wire::{Channel, Sender};
use record::LaunchRecord;
use std::{
    ffi::CString,
    fs::File,
    io::Write,
    os::unix::net::UnixStream,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// Opaque, kernel-owned launch preparation. It deliberately cannot be made
/// from a client executable path, boolean `verified`, or an App manifest.
pub struct PreparedWorker {
    program: CString,
    arguments: Vec<CString>,
    record: LaunchRecord,
    // Open objects AND the domain's pinning lease survive through wait/reap.
    _objects: Vec<File>,
    domain: Box<dyn ResourceDomain>,
}

/// To be implemented by the enrolled platform provisioner, not by callers of
/// WorkerProcess. Linux must own its exact cgroup + bundled bubblewrap namespace
/// domain; a process group alone cannot contain bubblewrap's nested session.
trait ResourceDomain: Send {
    // Verification must be bounded and all cleanup methods nonpanicking.
    fn verify_before_continue(&mut self, launcher_pid: libc::pid_t) -> Result<(), WorkerError>;
    fn terminate(&mut self, launcher_pid: libc::pid_t);
    /// Must await an empty owned domain after the direct child is reaped,
    /// retaining all resource/pinning reservations until then.
    fn reap_domain_blocking(&mut self);
}

#[derive(Clone, Copy, Debug)]
pub struct WorkerLimits {
    pub startup_timeout: Duration,
    pub log_tail_bytes: usize,
    /// Combined stdout/stderr bytes allowed in each one-second interval.
    pub log_bytes_per_second: usize,
}
impl Default for WorkerLimits {
    fn default() -> Self {
        Self {
            startup_timeout: Duration::from_secs(15),
            log_tail_bytes: 16 * 1024,
            log_bytes_per_second: 128 * 1024,
        }
    }
}
impl WorkerLimits {
    fn validate(self) -> Result<Self, WorkerError> {
        if self.startup_timeout.is_zero()
            || self.startup_timeout > Duration::from_secs(15)
            || self.log_tail_bytes == 0
            || self.log_tail_bytes > 64 * 1024
            || self.log_bytes_per_second == 0
            || self.log_bytes_per_second > 1024 * 1024
        {
            Err(WorkerError::Preparation)
        } else {
            Ok(self)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum WorkerError {
    #[error("app_worker_preparation")]
    Preparation,
    #[error("app_worker_spawn")]
    Spawn,
    #[error("app_worker_startup_timeout")]
    StartupTimeout,
    #[error("app_worker_identity")]
    Identity,
    #[error("app_worker_resource_domain")]
    ResourceDomain,
    #[error("app_worker_io")]
    Io,
    #[error("app_worker_log_limit")]
    LogLimit,
    #[error("app_worker_exited_before_ready")]
    EarlyExit,
    #[error("app_worker_cancelled")]
    Cancelled,
    #[error("app_worker_supervisor")]
    Supervisor,
    #[error("app_worker_sdk_unavailable")]
    SdkUnavailable,
}

#[derive(Debug)]
pub struct WorkerExit {
    pub code: Option<i32>,
    pub signal: Option<i32>,
    pub failure: Option<WorkerError>,
    /// Bounded untrusted bytes, not safe terminal text or public error detail.
    pub stdout_tail: Vec<u8>,
    pub stderr_tail: Vec<u8>,
}

#[derive(Clone)]
pub struct WorkerCancellation {
    cancelled: Arc<AtomicBool>,
    wake: Arc<UnixStream>,
}
impl WorkerCancellation {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        let _ = (&*self.wake).write(&[1]);
    }
}

pub struct WorkerProcess {
    generation: String,
    sdk: Option<UnixStream>,
    cancellation: WorkerCancellation,
    monitor: Option<JoinHandle<WorkerExit>>,
}

impl WorkerProcess {
    pub fn spawn_blocking(
        prepared: PreparedWorker,
        limits: WorkerLimits,
    ) -> Result<Self, WorkerError> {
        let limits = limits.validate()?;
        let record = prepared.record.encode()?;
        let ready = prepared.record.ready();
        let generation = prepared.record.generation.clone();
        let (sdk, child_sdk) = UnixStream::pair().map_err(|_| WorkerError::Io)?;
        let (control, child_control) = UnixStream::pair().map_err(|_| WorkerError::Io)?;
        let (stdout, child_stdout) = UnixStream::pair().map_err(|_| WorkerError::Io)?;
        let (stderr, child_stderr) = UnixStream::pair().map_err(|_| WorkerError::Io)?;
        let (wake_read, wake_write) = UnixStream::pair().map_err(|_| WorkerError::Io)?;
        for stream in [&sdk, &control, &stdout, &stderr, &wake_read, &wake_write] {
            stream.set_nonblocking(true).map_err(|_| WorkerError::Io)?;
        }
        let cancellation = WorkerCancellation {
            cancelled: Arc::new(AtomicBool::new(false)),
            wake: Arc::new(wake_write),
        };
        let input = File::open("/dev/null").map_err(|_| WorkerError::Io)?;
        let stdout_file = File::from(std::os::fd::OwnedFd::from(child_stdout));
        let stderr_file = File::from(std::os::fd::OwnedFd::from(child_stderr));
        let sdk_file = File::from(std::os::fd::OwnedFd::from(child_sdk));
        let control_file = File::from(std::os::fd::OwnedFd::from(child_control));
        let deadline = Instant::now() + limits.startup_timeout;
        let pid = spawn::launch(
            &prepared.program,
            &prepared.arguments,
            &[&input, &stdout_file, &stderr_file, &sdk_file, &control_file],
        )
        .map_err(|_| WorkerError::Spawn)?;
        let child = monitor::Child::new(pid, prepared);
        drop((input, stdout_file, stderr_file, sdk_file, control_file));
        let (started_tx, started_rx) = mpsc::sync_channel(1);
        let cancelled = cancellation.cancelled.clone();
        let monitor = thread::Builder::new()
            .name("chariox-app-worker".into())
            .spawn(move || {
                monitor::run(
                    child, control, stdout, stderr, wake_read, cancelled, record, ready, limits,
                    deadline, started_tx,
                )
            })
            .map_err(|_| WorkerError::Spawn)?;
        let worker = Self {
            generation,
            sdk: Some(sdk),
            cancellation,
            monitor: Some(monitor),
        };
        match started_rx.recv_timeout(limits.startup_timeout + Duration::from_secs(1)) {
            Ok(Ok(())) => Ok(worker),
            Ok(Err(error)) => {
                drop(worker);
                Err(error)
            }
            Err(_) => {
                drop(worker);
                Err(WorkerError::Supervisor)
            }
        }
    }

    pub fn cancellation(&self) -> WorkerCancellation {
        self.cancellation.clone()
    }

    /// Call inside the kernel Tokio I/O runtime. This transfers the one SDK
    /// stream; it does not create another MCP server or identity authority.
    pub fn take_sdk_channel(&mut self) -> Result<Channel<tokio::net::UnixStream>, WorkerError> {
        tokio::runtime::Handle::try_current().map_err(|_| WorkerError::SdkUnavailable)?;
        let stream = self.sdk.take().ok_or(WorkerError::SdkUnavailable)?;
        let stream =
            tokio::net::UnixStream::from_std(stream).map_err(|_| WorkerError::SdkUnavailable)?;
        Channel::new(stream, self.generation.clone(), Sender::Worker)
            .map_err(|_| WorkerError::SdkUnavailable)
    }

    pub fn wait_blocking(mut self) -> Result<WorkerExit, WorkerError> {
        self.join()
    }
    pub fn shutdown_blocking(mut self) -> Result<WorkerExit, WorkerError> {
        self.cancellation.cancel();
        self.join()
    }
    fn join(&mut self) -> Result<WorkerExit, WorkerError> {
        self.monitor
            .take()
            .ok_or(WorkerError::Supervisor)?
            .join()
            .map_err(|_| WorkerError::Supervisor)
    }
}
impl Drop for WorkerProcess {
    fn drop(&mut self) {
        if self.monitor.is_some() {
            self.cancellation.cancel();
            // Never detach the owner of the process and its admission lease.
            let _ = self.join();
        }
    }
}

#[cfg(test)]
mod tests;
