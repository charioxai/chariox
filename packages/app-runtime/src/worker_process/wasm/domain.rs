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
    pid: Option<libc::pid_t>,
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
            pid: None,
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
        if pid <= 1 {
            return Err(WorkerError::Identity);
        }
        self.pid = Some(pid);
        // The ready record arrives only after the worker's containment entry
        // succeeded; a failed entry exits before it. WASM limits alone are not
        // OS containment.
        Ok(())
    }
    fn terminate(&mut self, pid: libc::pid_t) {
        if pid > 1 && self.pid.is_none_or(|owned| owned == pid) {
            // The common monitor retains waitpid ownership until this call.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }
    fn reap_domain_blocking(&mut self) {}
}
