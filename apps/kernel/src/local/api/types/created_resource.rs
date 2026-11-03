//! MP-08 / MP-09 / MP-10 / MP-11 AU-11: metadata only; secrets stay in the Vault.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResourceRevokeMethod {
    Http {
        request: crate::secret::CredentialHttpRequest,
    },
    Url {
        url: String,
    },
    Unavailable {
        reason: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreatedResourceState {
    Active,
    Revoking,
    Revoked,
    RevokeFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreatedResource {
    pub id: String,
    pub project_id: String,
    pub service: String,
    pub kind: String,
    pub name: String,
    pub url: Option<String>,
    pub credential_id: Option<String>,
    pub created_by: String,
    pub created_at_ms: u64,
    pub revoke: ResourceRevokeMethod,
    pub state: CreatedResourceState,
    pub revoked_at_ms: Option<u64>,
    pub last_http_status: Option<u16>,
}
