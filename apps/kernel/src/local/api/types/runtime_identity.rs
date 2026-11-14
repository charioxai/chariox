use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObserveKernelRuntimeIdentityRequest {
    pub environment_id: String,
    pub expected_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KernelRuntimeIdentityObservationAcknowledgement {
    pub environment_id: String,
    pub generation: u64,
    pub observed_at: String,
}
