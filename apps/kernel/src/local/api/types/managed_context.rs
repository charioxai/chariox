use super::*;

pub use crate::managed_context::outbound_service::{
    ManagedContextOutboundOperationPhase, ManagedContextOutboundOperationStatus,
    ManagedContextTransferTarget, ManagedContextTransferTicket,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartManagedContextTransferRequest {
    pub ticket: ManagedContextTransferTicket,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetManagedContextTransferStatusRequest {
    pub context_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetManagedContextLaunchTargetRequest {
    pub context_id: String,
    pub plan_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedContextLaunchTarget {
    pub environment_id: String,
    pub kernel_id: String,
    pub context_id: String,
    pub plan_digest: String,
    pub development: ManagedContextDevelopmentLaunchTarget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ManagedContextDevelopmentLaunchTarget {
    Empty,
    FromSource {
        project_id: String,
        destination_root: String,
        primary_repository_id: String,
        repositories: Vec<ManagedContextRepositoryLaunchTarget>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedContextRepositoryLaunchTarget {
    pub repository_id: String,
    pub role: crate::managed_context::development::DevelopmentRepositoryRole,
    pub target_directory: String,
    pub workspace_path: String,
    pub head_sha: String,
}
