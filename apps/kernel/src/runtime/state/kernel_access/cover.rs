//! A covered process names a background process it started: the grant then covers that
//! process by identity (pid and start time) until it exits or the grant ends. A process
//! that forks twice changes its pid and can't be named.
use super::*;
use crate::local::{CoverKernelAccessProcessRequest, CoveredProcess};

/// The most processes one grant covers by name.
pub(crate) const MAX_COVERED: usize = 16;

/// The most processes already running under the holder that a grant covers by identity
/// the moment it lands, so they stay covered when their parent exits later.
pub(crate) const MAX_ADOPTED: usize = 48;

/// The audit reason of a cover taken from the holder's tree, so a restart can tell it from
/// a process named later.
pub(crate) const ADOPTED: &str = "adopted";

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
            .map_err(Refusal::error)?
            .into_iter()
            .find(|grant| bound.is_none_or(|id| id == grant.summary.grant_id))
            .map(|grant| grant.summary.grant_id)
            .ok_or_else(|| {
                match self.access_end_for(peer) {
                    Some(ended) => ended,
                    None if bound.is_some() => Refusal::Ended,
                    None => Refusal::NotCovered,
                }
                .error()
            })?;
        let (target, _) = process::inspect(request.pid).map_err(|error| {
            // Another user's process exists but is outside what any grant reaches.
            match error.kind() {
                std::io::ErrorKind::PermissionDenied => Refusal::CoverOutside,
                std::io::ErrorKind::NotFound => Refusal::CoverGone,
                _ => Refusal::Unconfirmed,
            }
            .error()
        })?;
        if request.start.is_some_and(|start| start != target.start) {
            return Err(Refusal::CoverIdentityChanged.error());
        }
        let (kernel, _) = process::inspect(std::process::id())
            .map_err(|_| error("the kernel's own identity is unavailable"))?;
        let chain = process::ancestry(&target).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                Refusal::CoverGone
            } else {
                Refusal::Unconfirmed
            }
            .error()
        })?;
        if chain.contains(&kernel) {
            return Err(Refusal::CoverKernelProcess.error());
        }
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
            live.covered.retain(|process| !process.gone());
            // The holder's tree at grant time is covered too (`adopted`); it does not use up
            // the limit of processes named later, and its exits do not add to it.
            let named = live
                .covered
                .iter()
                .filter(|process| !live.adopted.contains(process))
                .count();
            if named >= MAX_COVERED {
                return Err(Refusal::CoverLimit.error());
            }
            // Recorded before it is live: a cover that failed to persist is refused, and
            // a retry records it again.
            self.audit_cover(&live.summary, &target, "covered", None)?;
            live.covered.push(target.clone());
        }
        Ok((grant, covered_process(&target)))
    }

    /// The processes already running under `holder` when its grant lands. They are
    /// reached by ancestry now; covering them by identity keeps them covered after the
    /// parent that links them to the holder exits (the shell call that started a
    /// background program ends). It adds no authority: who may hold access is unchanged,
    /// and only what the holder's tree already is at this moment is covered. The kernel
    /// and what it started are never covered.
    pub(super) fn cover_running_descendants(
        &self,
        grant: &KernelAccessGrant,
        holder: &ProcessIdentity,
    ) -> Vec<ProcessIdentity> {
        let Ok((kernel, _)) = process::inspect(std::process::id()) else {
            return Vec::new();
        };
        let mut covered = Vec::new();
        for target in process::descendants(holder) {
            if covered.len() >= MAX_ADOPTED {
                break;
            }
            let reached = process::ancestry(&target)
                .is_ok_and(|chain| chain.contains(holder) && !chain.contains(&kernel));
            // Recorded before it is live, as for a named cover.
            if target.uid == holder.uid
                && reached
                && self.audit_cover(grant, &target, "covered", Some(ADOPTED)).is_ok()
            {
                covered.push(target);
            }
        }
        covered
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
