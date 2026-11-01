//! MP-08 / MP-09 / MP-10 / MP-11: an outside agent asks the owner for a Vault value.
//! It learns the link and the outcome, never the value.
use super::*;

/// Protocol 929: the owner types the value in a connected client or on the link's page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestVaultSecretRequest {
    pub credential_id: String,
    /// What the owner is asked for, in the agent's words (shown after the kernel's own sentence).
    pub label: String,
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetVaultSecretStatusRequest {
    pub interaction_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultSecretStatus {
    Pending,
    Stored,
    Cancelled,
    TimedOut,
}
