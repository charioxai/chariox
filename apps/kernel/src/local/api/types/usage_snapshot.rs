use super::*;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetProviderUsageSnapshotRequest {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetAgentUsageSnapshotRequest {
    pub session_id: String,
    pub agent_id: String,
}

/// Usage references contain safe account identity, never profile locations or credentials.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderUsageAccount {
    /// Opaque identity shared by reports of the same provider account.
    pub account_id: String,
    pub auth_state: crate::account_profile::ProviderAccountAuthState,
    pub services: Vec<crate::account_profile::ProviderAccountService>,
    pub provider: String,
    pub profile_id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_summary: Option<String>,
    pub usage: crate::account_profile::ProviderAccountUsageSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentContextUsage {
    pub used_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentExecutionRoute {
    pub target: String,
    pub target_kernel_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via_kernel_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentUsageSnapshot {
    pub session_id: String,
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<AgentContextUsage>,
    pub route: AgentExecutionRoute,
}
