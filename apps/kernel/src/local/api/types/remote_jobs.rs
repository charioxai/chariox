use super::*;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OffloadPolicy {
    #[default]
    Off,
    WhenBusy,
    AlwaysHeavy,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetRemoteJobPolicyRequest {
    pub session_id: String,
    #[serde(default)]
    pub agent_id: Option<String>,
    /// The project's own setting instead of the Room's or an agent's; not with `agent_id`.
    #[serde(default)]
    pub project: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetRemoteJobPolicyRequest {
    pub session_id: String,
    #[serde(default)]
    pub agent_id: Option<String>,
    /// The project's own setting instead of the Room's or an agent's; not with `agent_id`.
    #[serde(default)]
    pub project: bool,
    pub policy: Option<OffloadPolicy>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteJobPolicy {
    pub policy: Option<OffloadPolicy>,
    pub effective_policy: OffloadPolicy,
    pub inherited_policy: OffloadPolicy,
    pub source: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunSessionProcessRemotelyRequest {
    pub session_id: String,
    pub process_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteJobEstimate {
    pub amount_micros: u64,
    pub currency: String,
    pub kind: String,
    pub memory_mib: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minutes: Option<u64>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteJobPlacement {
    pub machine_label: String,
    pub address: Option<String>,
}
