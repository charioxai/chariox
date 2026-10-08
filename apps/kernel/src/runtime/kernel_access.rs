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
}

pub(crate) type AccessStore = Arc<Mutex<AccessState>>;

pub(crate) fn error(message: impl Into<String>) -> crate::error::DaemonError {
    crate::error::DaemonError::LocalTransport {
        operation: "kernel access",
        message: message.into(),
    }
}
