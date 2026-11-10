//! Process owner for the native WASM worker. The worker enters R5's OS
//! containment itself before its ready record; `verify_before_continue` is the
//! last gate before *any* guest instantiation.
use super::*;
use crate::worker_process::ResourceDomain;

pub(super) struct WasmDomain {
    descriptors: [File; 5],
    data: Dir,
    _runtime: WasmRuntime,
    _release: VerifiedReleaseLease,
    process: OwnedProcess,
}
impl WasmDomain {
    pub(super) fn new(
        descriptors: [File; 5],
        data: Dir,
        runtime: WasmRuntime,
        release: VerifiedReleaseLease,
    ) -> Self {
        Self {
            descriptors,
            data,
            _runtime: runtime,
            _release: release,
            process: OwnedProcess::new(None),
        }
    }
}
impl ResourceDomain for WasmDomain {
    fn private_data_directory(&self) -> Result<File, WorkerError> {
        self.data.0.try_clone().map_err(|_| WorkerError::Io)
    }
    fn setup_descriptors(&self) -> &[File] {
        &self.descriptors
    }
    fn verify_before_continue(&mut self, pid: libc::pid_t) -> Result<(), WorkerError> {
        self.process.verify(pid)
    }
    fn terminate(&mut self, pid: libc::pid_t) {
        self.process.terminate(pid);
    }

    fn reap_domain_blocking(&mut self) {}
}

/// Shared exact-PID ownership for native contained workers. Seccomp/Seatbelt
/// forbids child processes, so reaping the owned child empties the domain.
pub(in crate::worker_process) struct OwnedProcess {
    pid: Option<libc::pid_t>,
    deadline: Option<std::time::Instant>,
}
impl OwnedProcess {
    pub(in crate::worker_process) fn new(deadline: Option<std::time::Instant>) -> Self {
        Self {
            pid: None,
            deadline,
        }
    }
    pub(in crate::worker_process) fn verify(
        &mut self,
        pid: libc::pid_t,
    ) -> Result<(), WorkerError> {
        if pid <= 1 || self.pid.is_some_and(|owned| owned != pid) {
            return Err(WorkerError::Identity);
        }
        self.pid = Some(pid);
        Ok(())
    }
    pub(in crate::worker_process) fn check(
        &self,
        pid: libc::pid_t,
        now: std::time::Instant,
    ) -> Result<(), WorkerError> {
        if self.pid != Some(pid) {
            return Err(WorkerError::Identity);
        }
        if self.deadline.is_some_and(|deadline| now >= deadline) {
            return Err(WorkerError::StartupTimeout);
        }
        Ok(())
    }
    pub(in crate::worker_process) fn terminate(&mut self, pid: libc::pid_t) {
        if pid > 1 && self.pid.is_none_or(|owned| owned == pid) {
            // The common monitor still owns waitpid, preventing PID reuse.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }
}
