//! MP-08/MP-11: owner-only, value-free grant visibility (local 443).
use super::*;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum UserDomainResource {
    Desktop { surface_id: String },
    BrowserTab { tab_id: String },
    AppView { view_id: String },
    Note { note_id: String },
    Capture { capture_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserDomainGrant {
    pub agent_id: String,
    pub session_id: String,
    pub kernel_id: String,
    pub resources: Vec<UserDomainResource>,
    pub since_ms: u64,
    pub focused: bool,
    pub idle_since_ms: Option<u64>,
    pub idle_timeout_seconds: u64,
    pub expiry_rule: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserDomainNotice {
    pub agent_id: String,
    pub resource: UserDomainResource,
    pub at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserDomainWindowAccess {
    pub kernel_id: String,
    pub kernel_name: String,
    pub focused_agent_kernel_id: Option<String>,
    pub reachable_by_focused_agent: bool,
}
