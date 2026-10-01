use super::*;

/// Protocol 393: how the kernel identified a connection, from the fixed
/// vocabulary of the kernel access plan (section 9.1). It is assigned when a
/// connection is admitted, recorded on its commands' caller and in traces, and
/// named by audit events such as `critical_approval.passkey`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelConnectionClass {
    /// The kernel's local token on TCP loopback, or a relay client with a
    /// user id (web, remote TUI).
    Terminal,
    /// A Unix socket peer holding an access grant. Not assigned yet.
    ExternalAgent,
    /// An agent the kernel launched, by its per-run runtime MCP bearer.
    KernelAgent,
    /// The host controller of a managed or hosted worker, by
    /// `CHARIOX_KERNEL_LOCAL_AUTH_TOKEN(_FILE)`.
    Host,
    /// Another kernel or a hosted service, by its relay identity and keys.
    RelayPeer,
    /// Neither a token nor a grant.
    Unauthenticated,
}

impl KernelConnectionClass {
    /// Whether a `passkey` from this class may reach verification. Only
    /// terminals may submit one; unauthenticated connections keep their
    /// current treatment until enforcement.
    pub fn may_submit_passkey(self) -> bool {
        matches!(self, Self::Terminal | Self::Unauthenticated)
    }
}
