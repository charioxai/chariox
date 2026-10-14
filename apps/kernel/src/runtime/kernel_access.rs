//! Process-bound access policy. Grants contain no transferable credential.
pub(crate) mod process;
pub(crate) mod requester;
/// Routing identity for kernel-wide access popups; never a session or grant scope.
pub(crate) const ACCESS_INTERACTION_SCOPE: &str = "kernel-access";
use crate::local::KernelAccessGrant;
use process::ProcessIdentity;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Clone)]
pub(crate) struct Grant {
    pub(crate) summary: KernelAccessGrant,
    pub(crate) holder: ProcessIdentity,
    pub(crate) deadline: Instant,
    pub(crate) notice: Instant,
    pub(crate) notice_sent: bool,
}

#[derive(Default)]
pub(crate) struct AccessState {
    pub(crate) grants: BTreeMap<String, Grant>,
    pub(crate) pending: BTreeMap<(u32, u64, String), KernelAccessGrant>,
    pub(crate) generation: u64,
    pub(crate) last_denial: Option<Instant>,
    pub(crate) suppressed_denials: u64,
    /// Holders whose grant lapsed or was revoked, so a later request from them
    /// reads "access ended" rather than "never covered". Bounded, no secrets.
    pub(crate) ended: Vec<ProcessIdentity>,
}

const ENDED_HOLDERS: usize = 32;

impl AccessState {
    pub(crate) fn remember_ended(&mut self, holder: &ProcessIdentity) {
        self.ended.retain(|known| known != holder && known.alive());
        self.ended.push(holder.clone());
        if self.ended.len() > ENDED_HOLDERS {
            self.ended.remove(0);
        }
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
}

impl Refusal {
    pub(crate) const ALL: [Self; 4] = [
        Self::HumanOnly,
        Self::NotCovered,
        Self::Ended,
        Self::AlreadyGranted,
    ];

    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::HumanOnly => "kernel_access_human_only",
            Self::NotCovered => "kernel_access_not_covered",
            Self::Ended => "kernel_access_ended",
            Self::AlreadyGranted => "kernel_access_already_granted",
        }
    }

    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::HumanOnly => "Only you can do this, in a Chariox terminal. An external agent can't.",
            Self::NotCovered => "This process has no access. Only the agent you approved, and the programs it starts, are covered.",
            Self::Ended => "Access ended. Ask for new access if you still need it.",
            Self::AlreadyGranted => "This process already has access.",
        }
    }

    /// Classifies a policy error. Only the lifecycle constants mean "ended";
    /// every other policy refusal is an owner-only action.
    pub(crate) fn of(error: &crate::error::DaemonError) -> Self {
        Self::exact(error).unwrap_or(match error {
            crate::error::DaemonError::LocalTransport { message, .. }
                if matches!(
                    message.as_str(),
                    "grant revoked or expired" | "sudo turn ended or was revoked"
                ) =>
            {
                Self::Ended
            }
            _ => Self::HumanOnly,
        })
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
