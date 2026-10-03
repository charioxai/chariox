use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateDisposableWorkerRequest {
    pub client_request_id: String,
    pub home_kernel_id: String,
    pub home_relay_realm_id: String,
    pub region: String,
    pub compute_class: String,
    pub architecture: String,
    pub maximum_lifetime_seconds: u64,
    pub auto_stop_policy: ManagedEnvironmentAutoStopPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_plan: Option<ManagedEnvironmentContextPlanInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DisposableWorkerRequest {
    pub allocation_id: String,
    pub home_kernel_id: String,
    pub home_relay_realm_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisposableWorkerDesiredState {
    Running,
    Released,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisposableWorkerObservedState {
    Requested,
    Provisioning,
    Enrolling,
    Ready,
    Leased,
    Deleting,
    Deleted,
    Failed,
}

/// Public Cloud allocation metadata; never includes a provider credential or bootstrap grant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisposableWorkerAllocation {
    pub allocation_id: String,
    pub home_kernel_id: Option<String>,
    pub home_relay_realm_id: Option<String>,
    pub region: String,
    pub compute_class: String,
    pub architecture: String,
    pub provider_id: String,
    pub provider_profile_id: String,
    pub storage_monthly_minor_units: Option<u64>,
    pub managed_repository_root: String,
    pub runtime_release_digest: String,
    pub desired_state: DisposableWorkerDesiredState,
    pub observed_state: DisposableWorkerObservedState,
    pub desired_revision: u64,
    pub observed_revision: u64,
    pub runtime_machine_id: Option<String>,
    pub runtime_kernel_id: Option<String>,
    pub operation_id: String,
    pub operation_status: ManagedEnvironmentOperationStatus,
    pub expires_at: String,
    pub created_at: String,
    pub auto_stop_policy: ManagedEnvironmentAutoStopPolicy,
    pub auto_stop_deadline_at: Option<String>,
}
