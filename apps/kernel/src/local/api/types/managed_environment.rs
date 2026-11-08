use super::*;

// MP-08 / MP-11: selection metadata only; credential bytes never cross IPC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreflightProviderAccountPortabilityRequest {
    pub provider_accounts: ManagedEnvironmentProviderAccounts,
    /// Names this launch, so the transfer start that carries the same key finds its unlock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObserveManagedEnvironmentPreReimageRequest {
    pub environment_id: String,
    pub expected_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedEnvironmentPreReimageObservationAcknowledgement {
    pub environment_id: String,
    pub generation: u64,
    pub observed_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ManagedEnvironmentProviderAccounts {
    None,
    Selected {
        accounts: Vec<ManagedEnvironmentProviderAccountSelection>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedEnvironmentProviderAccountSelection {
    pub provider: String,
    pub account_profile: String,
}
