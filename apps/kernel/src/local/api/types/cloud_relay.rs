use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayStatusRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigureRelayRequest {
    pub relay_url: Option<String>,
    pub relay_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudRelayStatusRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartCloudRelayLoginRequest {
    pub api_url: String,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub client_alias: Option<String>,
    #[serde(default)]
    pub machine_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollCloudRelayLoginRequest {
    pub api_url: String,
    pub device_code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_access_denied: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogoutCloudRelayRequest {
    #[serde(default)]
    pub revoke_client: bool,
    #[serde(default)]
    pub revoke_machine: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectCloudRelayRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueCloudRelayClientTokenRequest {
    pub target_daemon_alias: String,
    pub client_id: String,
    #[serde(default)]
    pub session_id: Option<String>,
    /// When present, the issued client token is bound to this CLI relay key.
    #[serde(default)]
    pub public_key_thumbprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveKernelClientConnectionRequest {
    pub kernel_ref: String,
    #[serde(default)]
    pub machine_ref: Option<String>,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub public_key_thumbprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelClientConnection {
    pub relay_url: String,
    pub relay_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_daemon_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_daemon_alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_expires_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kernel_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudRelayProfile {
    pub api_url: String,
    pub email: String,
    pub account_id: String,
    pub user_id: String,
    pub account_slug: String,
    pub realm_id: String,
    pub relay_url: String,
    pub issuer_id: String,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub client_alias: Option<String>,
    #[serde(default)]
    pub machine_id: Option<String>,
    #[serde(default)]
    pub kernel_id: Option<String>,
    #[serde(default)]
    pub kernel_enrolled: bool,
    #[serde(default)]
    pub token_expires_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudRelayLoginStart {
    pub api_url: String,
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    pub expires_at: String,
    pub interval_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudRelayLoginPollStatus {
    AuthorizationPending,
    ExpiredToken,
    AccessDenied,
    Approved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudRelayLoginPoll {
    pub status: CloudRelayLoginPollStatus,
    #[serde(default)]
    pub interval_seconds: Option<u64>,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub profile: Option<CloudRelayProfile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudRelayRuntimeToken {
    pub relay_url: String,
    pub relay_token: String,
    pub token_expires_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayStatus {
    /// The answering kernel's protocol, independent of control-plane heartbeat freshness.
    pub local_daemon_protocol_version: u32,
    /// Native Linux process identity for independent signed-artifact observation.
    /// Legacy and non-Linux kernels provide no implied process proof.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_process_identity: Option<KernelRuntimeProcessIdentity>,
    /// Implemented control contracts, independent of branch-local protocol numbers.
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub configured: bool,
    pub connected: bool,
    pub relay_url: Option<String>,
    pub relay_token_configured: bool,
    pub daemon_id: String,
    pub machine_id: String,
    pub machine_alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelRuntimeProcessIdentity {
    pub pid: u32,
    pub linux_boot_id: String,
    pub start_time_ticks: String,
}

/// Shared by direct kernel status and relay discovery. These advertise support,
/// not authorization; every operation still checks its caller and home binding.
pub const RUNTIME_CONTROL_CAPABILITIES: &[&str] = &[
    "terminal_relay_authorization_renewal_v1",
    "exact_prompt_cancellation_v499",
    "owner_managed_context_transfer_v1",
    "kernel_gateway_v1",
    "room_terminal_v1",
    "room_exec_v1",
    "agent_deadline_v1",
    "outside_steer_events_v1",
    "framed_plaintext_v1",
    crate::provider_account_copy::PROVIDER_ACCOUNT_COPY_CAPABILITY,
    crate::provider_account_copy::PROVIDER_ACCOUNT_COPY_PULL_CAPABILITY,
];
