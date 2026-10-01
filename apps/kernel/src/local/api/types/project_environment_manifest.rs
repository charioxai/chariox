//! MP-08: Kernel-owned Project environment metadata; values are never projected.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetProjectEnvironmentManifestRequest {
    pub project_id: String,
}
