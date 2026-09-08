//! The same resource owner is retained from pre-exec admission through an empty
//! cgroup. Its private inputs must eventually come from enrolled artifact and
//! storage leases; the hosted fixture is not a production enrollment factory.
use super::{cgroup, inspection, plan, Result};
use crate::worker_process::ResourceDomain;
use std::{fs::File, time::Instant};

pub(super) struct Domain {
    pub leaf: cgroup::Lease,
    pub observer: inspection::Observer,
    /// FD5 is cgroup.procs; FD6 is the pinned bubblewrap executable. The native
    /// entry joins the cgroup before exec/fork, then closes both setup channels.
    pub setup: [File; 2],
    pub _roots: [plan::Binding; 4],
    pub _libraries: Vec<(String, plan::Binding)>,
}

impl ResourceDomain for Domain {
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
