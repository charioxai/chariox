use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum AccountKernelsRequest {
    List {},
    Unlink { target_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountKernel {
    /// The stable account directory row ID used by Settings to unlink this kernel.
    pub target_id: String,
    pub kernel_id: String,
    pub name: Option<String>,
    pub status: AccountKernelStatus,
    /// RFC 3339 timestamp of the last heartbeat, if the kernel has reported one.
    pub last_seen_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountKernelStatus {
    Online,
    Offline,
    Stale,
    Draining,
}
