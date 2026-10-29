//! Process-bound access policy. Grants contain no transferable credential.
pub(crate) mod process;
pub(crate) mod requester;
mod sudo_request;
pub(crate) use sudo_request::SudoRequestFailure;
/// Routing identity for kernel-wide access popups; never a session or grant scope.
pub(crate) const ACCESS_INTERACTION_SCOPE: &str = "kernel-access";
use crate::local::{CoveredProcess, KernelAccessGrant};
use process::ProcessIdentity;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The clock of a timed grant. A grant that lasts until its holder exits has none.
#[derive(Clone)]
pub(crate) struct GrantClock {
    pub(crate) deadline: Instant,
    pub(crate) notice: Instant,
    pub(crate) notice_sent: bool,
}

impl GrantClock {
    /// A clock for a grant of `minutes`, whose "extend?" notice comes
    /// `notice_minutes` before the deadline.
    pub(crate) fn new(minutes: u32, notice_minutes: u32) -> Self {
        let now = Instant::now();
        Self {
            deadline: now + Duration::from_secs(u64::from(minutes) * 60),
            notice: now
                + Duration::from_secs(u64::from(minutes.saturating_sub(notice_minutes)) * 60),
            notice_sent: false,
        }
    }
}

#[derive(Clone)]
pub(crate) struct Grant {
    pub(crate) summary: KernelAccessGrant,
    pub(crate) holder: ProcessIdentity,
    /// Background processes the grant covers by name once their parent is gone.
    pub(crate) covered: Vec<ProcessIdentity>,
    pub(crate) clock: Option<GrantClock>,
}

impl Grant {
    /// How far up `chain` (a process and its ancestors) the grant first reaches: the
    /// holder or a covered process.
    pub(crate) fn covering(&self, chain: &[ProcessIdentity]) -> Option<usize> {
        chain
            .iter()
            .position(|process| *process == self.holder || self.covered.contains(process))
    }

    /// The grant as clients list it: its summary with the covered processes that still run.
    pub(crate) fn listed(&self) -> KernelAccessGrant {
        KernelAccessGrant {
            covered: self
                .covered
                .iter()
                .filter(|process| process.alive())
                .map(|process| CoveredProcess {
                    pid: process.pid,
                    start: process.start,
                    executable: process.executable.clone(),
                })
                .collect(),
            ..self.summary.clone()
        }
    }

    /// A grant that lasts until its holder exits has no deadline, so a kernel
    /// restart does not end it while the holder lives.
    pub(crate) fn survives_restart(&self) -> bool {
        self.clock.is_none()
    }

    /// Why the grant has ended at `now`, if it has: its deadline passed or
    /// its holder (or a process reusing its identity) is gone.
    pub(crate) fn ended_reason(&self, now: Instant) -> Option<&'static str> {
        if self
            .clock
            .as_ref()
            .is_some_and(|clock| now >= clock.deadline)
        {
            Some("expiry")
        } else if !self.holder.alive() {
            Some("process_exit")
        } else {
            None
        }
    }
}

#[derive(Default)]
pub(crate) struct AccessState {
    pub(crate) grants: BTreeMap<String, Grant>,
    /// Admission wakes the idle pump so it can watch the new holder's exit.
    pub(crate) holder_changed: Arc<tokio::sync::Notify>,
    pub(crate) pending: BTreeMap<(u32, u64, String), KernelAccessGrant>,
    pub(crate) generation: u64,
    /// The kernel has begun to stop: event waits end their poll.
    pub(crate) stopping: bool,
    pub(crate) last_denial: Option<Instant>,
    pub(crate) suppressed_denials: u64,
    /// Holders and covered processes whose grant lapsed or was revoked, so a later request
    /// from them reads "access ended" rather than "never covered". Bounded, no secrets.
    pub(crate) ended: Vec<ProcessIdentity>,
    /// Requesters on other kernels that raised a sudo request here, by the grant id
    /// their sudo entries carry. They live while their origin kernel keeps renewing the
    /// lease and until it says its grant ended; silence ends them here without it.
    pub(crate) remote_requesters: BTreeMap<String, RemoteRequester>,
    /// Grants that ended in memory but whose durable end is not complete. They
    /// authorize nothing; a revoke or the next sweep finishes them.
    pub(crate) owed_ends: Vec<OwedEnd>,
}

/// A grant's end (`revoked` or `expired`) that still has to reach the store.
#[derive(Debug, Clone)]
pub(crate) struct OwedEnd {
    pub(crate) grant: KernelAccessGrant,
    /// Covered identities whose end has not yet reached the audit stream.
    pub(crate) covered: Vec<ProcessIdentity>,
    pub(crate) outcome: String,
    pub(crate) reason: String,
    /// The grant end is in the audit log; its inbox or process ends may remain.
    pub(crate) recorded: bool,
}

/// Who asked from another kernel: shown in the popup, and the liveness of its sudo entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RemoteRequester {
    pub(crate) origin_name: String,
    pub(crate) harness: String,
    /// The origin's last word that its grant lives, plus `REMOTE_REQUESTER_LEASE`.
    pub(crate) lease_until: Instant,
}

impl RemoteRequester {
    pub(crate) fn live(&self, now: Instant) -> bool {
        now < self.lease_until
    }
}

/// How long this kernel trusts a requester's origin to still hold its grant without hearing
/// from it: the bound on how long A's crash, restart or a partition can leave B's popup or
/// window running. The origin renews every `REMOTE_REQUESTER_RENEW_EVERY`, so a lease only
/// lapses after several renewals were lost.
pub(crate) const REMOTE_REQUESTER_LEASE: std::time::Duration = std::time::Duration::from_secs(45);
pub(crate) const REMOTE_REQUESTER_RENEW_EVERY: std::time::Duration =
    std::time::Duration::from_secs(10);

const ENDED_PROCESSES: usize = 256;

impl AccessState {
    /// Remembers the holder and the covered processes of an ended grant.
    pub(crate) fn remember_ended(&mut self, grant: &Grant) {
        let ended = std::iter::once(&grant.holder).chain(&grant.covered);
        self.ended.retain(|known| known.alive());
        for process in ended {
            self.ended.retain(|known| known != process);
            self.ended.push(process.clone());
        }
        let excess = self.ended.len().saturating_sub(ENDED_PROCESSES);
        self.ended.drain(..excess);
    }
}

pub(crate) type AccessStore = Arc<Mutex<AccessState>>;

pub(crate) fn error(message: impl Into<String>) -> crate::error::DaemonError {
    crate::error::DaemonError::LocalTransport {
        operation: "kernel access",
        message: message.into(),
    }
}

/// Why an external Unix peer's request was refused. Each reason has its own wire
/// code and one plain sentence for the person reading the agent's output; never
/// a PID, path or grant id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// A critical approval, passkey, secret or authority change: owner-only.
    HumanOnly,
    /// The peer is not the approved holder or one of its sub-processes.
    NotCovered,
    /// The grant expired, was revoked, or its holder exited.
    Ended,
    /// The peer's tree already holds an access grant.
    AlreadyGranted,
    /// A cover request named a process that has exited, or isn't a process.
    CoverGone,
    /// A cover request named a pid whose process start time differs from the one given.
    CoverIdentityChanged,
    /// A cover request named the kernel or a process the kernel started.
    CoverKernelProcess,
    /// A cover request named a process the grant doesn't already reach.
    CoverOutside,
    /// The grant already covers as many processes by name as it may.
    CoverLimit,
}

impl Refusal {
    pub(crate) const ALL: [Self; 9] = [
        Self::HumanOnly,
        Self::NotCovered,
        Self::Ended,
        Self::AlreadyGranted,
        Self::CoverGone,
        Self::CoverIdentityChanged,
        Self::CoverKernelProcess,
        Self::CoverOutside,
        Self::CoverLimit,
    ];

    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::HumanOnly => "kernel_access_human_only",
            Self::NotCovered => "kernel_access_not_covered",
            Self::Ended => "kernel_access_ended",
            Self::AlreadyGranted => "kernel_access_already_granted",
            Self::CoverGone => "kernel_access_cover_gone",
            Self::CoverIdentityChanged => "kernel_access_cover_identity_changed",
            Self::CoverKernelProcess => "kernel_access_cover_kernel_process",
            Self::CoverOutside => "kernel_access_cover_outside",
            Self::CoverLimit => "kernel_access_cover_limit",
        }
    }

    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::HumanOnly => "Only you can do this, in a Chariox terminal. An external agent can't.",
            Self::NotCovered => "This process has no access. Only the agent you approved, and the programs it starts, are covered.",
            Self::Ended => "Access ended. Ask for new access if you still need it.",
            Self::AlreadyGranted => "This process already has access.",
            Self::CoverGone => "That process is not running. Only a running process can be covered.",
            Self::CoverIdentityChanged => "That pid now belongs to a different process than the one you named.",
            Self::CoverKernelProcess => "The kernel and the programs it starts can't be covered.",
            Self::CoverOutside => "Only a process your access already reaches can be covered: the agent you approved, the programs it starts, and processes it covered before.",
            Self::CoverLimit => "Access already covers as many background processes as it may. Stop one first.",
        }
    }

    /// Classifies a policy error. Only the lifecycle constants mean "ended";
    /// every other policy refusal is an owner-only action.
    pub(crate) fn of(error: &crate::error::DaemonError) -> Self {
        Self::exact(error).unwrap_or(
            if Self::grant_ended(error)
                || matches!(error, crate::error::DaemonError::LocalTransport { message, .. }
                    if message == "sudo turn ended or was revoked")
            {
                Self::Ended
            } else {
                Self::HumanOnly
            },
        )
    }

    /// The grant was gone when a request that had already been admitted looked it
    /// up: the person reading the error must see "ended", not a retryable failure.
    pub(crate) fn grant_ended(error: &crate::error::DaemonError) -> bool {
        Self::exact(error) == Some(Self::Ended)
            || matches!(error, crate::error::DaemonError::LocalTransport { message, .. }
                if message == "grant revoked or expired")
    }

    /// The reason an error already carries, when it came from `Refusal::error`.
    pub(crate) fn exact(error: &crate::error::DaemonError) -> Option<Self> {
        Self::ALL.into_iter().find(|reason| {
            matches!(error, crate::error::DaemonError::LocalTransport { message, .. } if message == reason.message())
        })
    }

    pub(crate) fn error(self) -> crate::error::DaemonError {
        error(self.message())
    }

    pub(crate) fn transport_error(self) -> crate::transport::kernel_protocol::KernelTransportError {
        crate::transport::kernel_protocol::KernelTransportError {
            code: self.code().into(),
            message: self.message().into(),
            retryable: false,
        }
    }
}
