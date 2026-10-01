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

/// Protocol 394: what a passkey prompt asks the owner to authorize. Critical
/// approvals only, for now; `/sudo` and access grants add their kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PasskeyPromptKind {
    /// A kernel decision whose approve choice is marked `requires_passkey`.
    CriticalApproval,
}

/// Protocol 394: a kernel-owned pending interaction that needs the Chariox
/// passkey, projected as a popup to every terminal connected as its owner,
/// attached to the session or not. Every field is what the kernel itself
/// established. It is answered with `RespondToInteraction` on `session_id`
/// and `interaction_id`: the approve choice with the passkey, or the refuse
/// choice without it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasskeyPrompt {
    pub kind: PasskeyPromptKind,
    pub session_id: String,
    /// The session's alias, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_alias: Option<String>,
    pub interaction_id: String,
    pub title: String,
    pub message: String,
    pub approve_choice_id: String,
    pub refuse_choice_id: String,
    pub requested_at_ms: u64,
    pub expires_at_ms: u64,
}
