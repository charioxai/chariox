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
    #[serde(default)]
    pub machine_alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollCloudRelayLoginRequest {
    pub api_url: String,
    pub device_code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogoutCloudRelayRequest {
    #[serde(default)]
    pub revoke_client: bool,
    #[serde(default)]
    pub revoke_machine: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairCloudRelayClientRequest {
    pub client_id: String,
    #[serde(default)]
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairCloudRelayMachineRequest {
    pub machine_id: String,
    #[serde(default)]
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectCloudRelayRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueCloudRelayClientTokenRequest {
    pub target_daemon_alias: String,
    pub client_id: String,
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateCloudSessionInviteRequest {
    pub session_id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub expires_in_ms: Option<u64>,
    #[serde(default)]
    pub max_uses: Option<u32>,
    #[serde(default)]
    pub collaboration_level: crate::session::CollaborationLevel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShowCloudSessionInviteRequest {
    pub invite_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptCloudSessionInviteRequest {
    pub invite_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevokeCloudSessionInviteRequest {
    pub session_id: String,
    pub invite_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListCloudSessionMembersRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListCloudCollaboratorsRequest;

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
    pub machine_alias: Option<String>,
    #[serde(default)]
    pub machine_credential: Option<String>,
    #[serde(default)]
    pub cloud_session_token: Option<String>,
    #[serde(default)]
    pub cloud_session_expires_at_ms: Option<u64>,
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
pub struct CloudSessionInvite {
    pub invite_id: String,
    pub invite_token: String,
    pub session_id: String,
    pub account_id: String,
    pub created_by_user_id: String,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub max_uses: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudSessionInviteDetails {
    pub invite_id: String,
    pub session_id: String,
    pub account_id: String,
    pub created_by_user_id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub max_uses: Option<u32>,
    pub used_count: u32,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudSessionInviteAcceptance {
    pub session_id: String,
    pub account_id: String,
    pub user_id: String,
    pub invited_by_user_id: String,
    pub joined_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudSessionMember {
    pub user_id: String,
    pub email: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub invited_by_user_id: Option<String>,
    pub joined_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudCollaborator {
    pub user_id: String,
    pub email: String,
    #[serde(default)]
    pub display_name: Option<String>,
    pub last_collaborated_at: String,
    pub shared_session_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayStatus {
    pub configured: bool,
    pub connected: bool,
    pub relay_url: Option<String>,
    pub relay_token_configured: bool,
    pub daemon_id: String,
    pub daemon_alias: Option<String>,
    pub machine_id: String,
    pub machine_alias: Option<String>,
}
