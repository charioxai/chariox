//! The same resource owner remains alive through native reap and broker drain.
//! Field drop order closes view handles before releasing the helper lease, and
//! removes the empty cgroup only after helper reclamation has been requested.
use super::{cgroup, inspection, plan, Result};
use crate::worker_process::ResourceDomain;
use crate::{
    release_store::VerifiedReleaseLease, runtime_enrollment::EnrolledRuntime,
    worker_process::storage_linux,
};
use std::{fs::File, time::Instant};

pub(super) struct Domain {
    pub _roots: [plan::Binding; 4],
    pub _libraries: Vec<(String, plan::Binding)>,
    /// FD5 is cgroup.procs; FD6 is the pinned bubblewrap executable. The native
    /// entry joins the cgroup before exec/fork, then closes both setup channels.
    pub setup: [File; 2],
    pub observer: inspection::Observer,
    pub storage: Option<storage_linux::Lease>,
    pub _runtime: Option<EnrolledRuntime>,
    pub _release: Option<VerifiedReleaseLease>,
    pub leaf: cgroup::Lease,
}

impl ResourceDomain for Domain {
    fn private_data_directory(&self) -> Result<File> {
        if self.storage.is_none() {
            return Err(super::WorkerError::Preparation);
        }
        self._roots[1]
            .object
            .try_clone()
            .map_err(|_| super::WorkerError::Preparation)
    }
    fn setup_descriptors(&self) -> &[File] {
        &self.setup
    }
    fn verify_before_continue(&mut self, pid: libc::pid_t) -> Result<()> {
        self.leaf.verify_limits()?;
        self.observer
            .verify(pid, &self.leaf.members()?, &self.leaf.path)
    }
    fn check_running(&mut self, _pid: libc::pid_t, _now: Instant) -> Result<()> {
        self.leaf.check_running()
    }
    fn terminate(&mut self, _pid: libc::pid_t) {
        self.leaf.terminate();
    }
    fn reap_domain_blocking(&mut self) {
        self.leaf.reap_blocking();
    }
}
