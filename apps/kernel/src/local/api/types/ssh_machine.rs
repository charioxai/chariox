//! MP-07 / MP-08 / MP-11: owner-managed SSH installation, local protocol 444.
use super::*;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddSshMachineRequest {
    pub host: String,
    #[serde(default)]
    pub install_id: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    /// Approved release ID from this kernel's operator-owned catalogue.
    #[serde(default)]
    pub release: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoveSshMachineRequest {
    pub install_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshMachineResult {
    pub install_id: String,
    pub status: String,
    pub kernel_id: Option<String>,
    pub machine_id: Option<String>,
    pub release_digest: String,
    pub state_retained: bool,
}
