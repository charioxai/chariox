use super::*;

// MP-08 / MP-11: selection metadata only; credential bytes never cross IPC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreflightProviderAccountPortabilityRequest {
    pub provider_accounts: ProviderAccountTransferSelection,
    /// Names this launch, so the transfer start that carries the same key finds its unlock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ProviderAccountTransferSelection {
    None,
    Selected {
        accounts: Vec<ProviderAccountTransferProfile>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderAccountTransferProfile {
    pub provider: String,
    pub account_profile: String,
}
