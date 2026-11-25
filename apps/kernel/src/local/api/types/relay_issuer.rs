use super::*;

/// The home kernel's self-hosted relay issuer (M5.5 "Token Issuers").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayIssuerStatusRequest;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelayIssuerStep {
    Enable,
    StageRotation,
    ActivateRotation,
    Disable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigureRelayIssuerRequest {
    pub step: RelayIssuerStep,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayIssuerKeyRecord {
    pub key_id: String,
    pub public_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayIssuerRecord {
    pub issuer_id: String,
    pub realm_id: String,
    pub active_key: RelayIssuerKeyRecord,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_key: Option<RelayIssuerKeyRecord>,
    /// Exact value for the relay's `CHARIOX_RELAY_KERNEL_ISSUER_KEYS`.
    pub relay_issuer_keys: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelayClientTokenScope {
    /// Connect to and route packets to this kernel.
    Attach,
    /// Read this kernel's relay metadata only.
    Metadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueRelayClientTokenRequest {
    pub client_id: String,
    pub public_key_thumbprint: String,
    pub scope: RelayClientTokenScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifetime_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayClientToken {
    pub relay_url: String,
    pub relay_token: String,
    pub target_daemon_id: String,
    pub token_id: String,
    pub expires_at_ms: u64,
}
