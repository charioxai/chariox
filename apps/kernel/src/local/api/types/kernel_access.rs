use super::*;

/// Protocol 402: how the kernel identified a connection, from the fixed
/// vocabulary of the kernel access plan (section 9.1). It is assigned when a
/// connection is admitted, recorded on its commands' caller and in traces, and
/// named by audit events such as `critical_approval.passkey`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelConnectionClass {
    /// The kernel's local token on TCP loopback, or a relay client with a
    /// user id (web, remote TUI).
    Terminal,
    /// An OS-verified Unix socket peer holding a local-kernel access grant.
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
    /// terminals may submit one.
    pub fn may_submit_passkey(self) -> bool {
        matches!(self, Self::Terminal)
    }
}

/// Protocol 403: what a passkey prompt asks the owner to authorize. Critical
/// approvals only, for now; `/sudo` and access grants add their kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PasskeyPromptKind {
    /// A kernel decision whose approve choice is marked `requires_passkey`.
    CriticalApproval,
    AccessGrant,
    AccessExtension,
    Sudo,
}

/// Protocol 403: a kernel-owned pending interaction that needs the Chariox
/// passkey, projected as a popup to every terminal connected as its owner,
/// attached to the session or not. Every field is what the kernel itself
/// established. It is answered with `RespondToInteraction` on `session_id`
/// and `interaction_id`: the approve choice with the passkey, or the refuse
/// choice without it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasskeyPrompt {
    /// Protocol 470: present for access_grant/access_extension; absent on older kernels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requester: Option<KernelAccessRequester>,
    pub kind: PasskeyPromptKind,
    /// Protocol 451: access decisions use `kernel-access`, a routing id without a session.
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
    /// Present only for access decisions. The terminal may choose a different term.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifetime_minutes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_lifetime_minutes: Option<u32>,
}

/// Protocol 470: OS-established identity of the grant holder, never display-text parsing.
/// Process start is an opaque decimal string to avoid JavaScript integer truncation
/// (Linux start ticks or macOS process unique ID); exec version is macOS's version,
/// zero on Linux. Harness identifies a configured executable, not vendor attestation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelAccessRequester {
    pub executable: String,
    pub pid: u32,
    pub process_start_id: String,
    pub process_exec_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_harness: Option<KernelAccessProviderHarness>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelAccessProviderHarness {
    Codex,
    Claude,
    Opencode,
}

/// Protocol 451: an OS-verified Unix peer asks for local-kernel access. No passkey or token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestKernelAccessRequest {
    pub holder_pid: u32,
    #[serde(default)]
    pub lifetime_minutes: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListKernelAccessGrantsRequest {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevokeKernelAccessGrantRequest {
    /// None revokes all grants owned by the caller's user.
    pub grant_id: Option<String>,
}

/// Public metadata only. This id is a revoke handle, never an access credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelAccessGrant {
    pub grant_id: String,
    pub owner_user_id: String,
    pub holder_pid: u32,
    pub holder_executable: String,
    pub lifetime_minutes: u32,
    pub expires_at_ms: u64,
}

/// Protocol 415: a grant holder requests a human-authorized turn. Identity and
/// session are resolved by the kernel; credentials are never accepted here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestKernelSudoRequest {
    pub agent_id: String,
    pub prompt: String,
}

/// Protocol 413: public attribution for an ephemeral, one-turn authorization.
/// `entry_id` is a revoke handle, never a credential. No time expiry applies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelSudoTurn {
    pub entry_id: String,
    pub session_id: String,
    pub agent_id: String,
    pub owner_user_id: String,
    pub terminal_id: String,
    /// Protocol 415: OS-established requester attribution for external entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requester: Option<KernelAccessGrant>,
    pub prompt_id: Option<String>,
    pub provider_run_id: Option<String>,
}
