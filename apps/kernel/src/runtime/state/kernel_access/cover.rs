//! A covered process names a background process it started: the grant then covers that
//! process by identity (pid and start time) until it exits or the grant ends. A process
//! that forks twice changes its pid and can't be named.
use super::*;
use crate::local::{CoverKernelAccessProcessRequest, CoveredProcess};

/// The most processes one grant covers by name.
pub(crate) const MAX_COVERED: usize = 16;

impl KernelRuntimeState {
    /// Covers `request.pid` for the grant `peer` is already covered by. The target must be
    /// a live process the grant covers now (the holder, a covered process, or a descendant
    /// of either), of the kernel's user, never the kernel or anything it started. The cover
    /// adds no authority: it only keeps what the grant already reaches from lapsing when
    /// the target's parent exits.
    pub(crate) fn cover_kernel_access_process(
        &self,
        peer: &ProcessIdentity,
        bound: Option<&str>,
        request: &CoverKernelAccessProcessRequest,
    ) -> Result<(String, CoveredProcess), DaemonError> {
        let grant = self
            .access_grants_for(peer)
            .into_iter()
            .find(|grant| bound.is_none_or(|id| id == grant.summary.grant_id))
            .map(|grant| grant.summary.grant_id)
            .ok_or_else(|| {
                if bound.is_some() || self.access_ended_for(peer) {
                    Refusal::Ended.error()
                } else {
                    Refusal::NotCovered.error()
                }
            })?;
        let (target, _) = process::inspect(request.pid).map_err(|_| Refusal::CoverGone.error())?;
        if request.start.is_some_and(|start| start != target.start) {
            return Err(Refusal::CoverIdentityChanged.error());
        }
        let (kernel, _) = process::inspect(std::process::id())
            .map_err(|_| error("the kernel's own identity is unavailable"))?;
        if kernel.contains(&target) {
            return Err(Refusal::CoverKernelProcess.error());
        }
        let chain = process::ancestry(&target).map_err(|_| Refusal::CoverGone.error())?;
        let mut state = self
            .owned
            .kernel_access
            .lock()
            .expect("access state poisoned");
        let live = state
            .grants
            .get_mut(&grant)
            .ok_or_else(|| Refusal::Ended.error())?;
        if target.uid != peer.uid || live.covering(&chain).is_none() {
            return Err(Refusal::CoverOutside.error());
        }
        if !live.covered.contains(&target) {
            live.covered.retain(ProcessIdentity::alive);
            if live.covered.len() >= MAX_COVERED {
                return Err(Refusal::CoverLimit.error());
            }
            // Recorded before it is live: a cover that failed to persist is refused, and
            // a retry records it again.
            self.audit_cover(&live.summary, &target, "covered", None)?;
            live.covered.push(target.clone());
        }
        Ok((grant, covered_process(&target)))
    }

    /// Records that a covered process left the grant's set.
    pub(super) fn audit_cover(
        &self,
        grant: &KernelAccessGrant,
        process: &ProcessIdentity,
        outcome: &str,
        reason: Option<&str>,
    ) -> Result<(), DaemonError> {
        self.owned.durable_state_store.append_event(
            "kernel_access.grant",
            Some(grant.grant_id.clone()),
            serde_json::json!({ "outcome": outcome, "grant": grant, "process": process, "reason": reason }),
        )?;
        Ok(())
    }
}

pub(crate) fn covered_process(process: &ProcessIdentity) -> CoveredProcess {
    CoveredProcess {
        pid: process.pid,
        start: process.start,
        executable: process.executable.clone(),
    }
}
