use super::*;

/// Protocol 568: whom a request forwarded by a kernel gateway acts for. The
/// target kernel makes its own human-only decision from it; `grant_id` is audit
/// text there, because the grant lives on the origin kernel only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnBehalfOf {
    pub origin_kernel: String,
    /// What the origin kernel is called to its owner (`<machine>/<alias>`), shown in the
    /// target's paired clients; absent when the origin has no alias.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_name: Option<String>,
    pub grant_id: String,
    pub harness: String,
}

/// Protocol 568: the kernels this kernel's gateway can forward to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListGatewayKernelsRequest {}

/// Protocol 568, human-only: pair this kernel's gateway with a self-hosted kernel
/// through the terminal pairing link that kernel made. A scoped relay needs the
/// operator's CLIENT transport token as well; it is stored privately and never returned.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoinKernelLinkRequest {
    pub link: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay_token: Option<String>,
}

impl std::fmt::Debug for JoinKernelLinkRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JoinKernelLinkRequest")
            .finish_non_exhaustive()
    }
}

/// Protocol 568, human-only: drop a paired kernel from this kernel's gateway.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForgetGatewayKernelRequest {
    pub name: String,
}

/// Protocol 568: one attachment a gateway keeps alive for its agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayAttachmentRef {
    pub session_id: String,
    pub attachment_id: String,
}

/// Protocol 568: a terminal heartbeat for attachments a gateway opened for an
/// agent. A relay terminal's attachment lapses after about 30 s without one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeartbeatAttachmentsRequest {
    pub attachments: Vec<GatewayAttachmentRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayKernelStatus {
    Online,
    Offline,
    /// The kernel is reachable but its owner removed this gateway there: pair it again.
    Unpaired,
}

/// Where the kernel list comes from (the inventory source).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayKernelVia {
    Hosted,
    SelfHosted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayKernel {
    /// What `--kernel` accepts: the machine's name when it is the user's only kernel
    /// there, else `<machine>/<label>`.
    pub name: String,
    pub kernel_id: String,
    /// The kernel's own name within its machine (its alias, else its id).
    pub label: String,
    pub machine_id: String,
    #[serde(default)]
    pub machine_alias: Option<String>,
    pub status: GatewayKernelStatus,
    /// Wall clock of the last inventory refresh that listed it online.
    pub last_seen_ms: u64,
    /// Whether the kernel speaks the gateway `on_behalf_of` contract.
    pub gateway_ready: bool,
    pub via: GatewayKernelVia,
}
