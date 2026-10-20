use super::*;

/// Protocol 568: whom a request forwarded by a machine gateway acts for. The
/// target kernel makes its own human-only decision from it; `grant_id` is audit
/// text there, because the grant lives on the origin kernel only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnBehalfOf {
    pub origin_kernel: String,
    pub grant_id: String,
    pub harness: String,
}

/// Protocol 568: the machines this kernel's gateway can forward to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListGatewayMachinesRequest {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayMachineStatus {
    Online,
    Offline,
}

/// Where the machine list comes from (the inventory source).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayMachineVia {
    Hosted,
    SelfHosted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayMachine {
    /// What `--machine` accepts: the kernel alias, else the machine alias, else the kernel id.
    pub name: String,
    pub kernel_id: String,
    pub machine_id: String,
    #[serde(default)]
    pub machine_alias: Option<String>,
    pub status: GatewayMachineStatus,
    /// Wall clock of the last inventory refresh that listed it online.
    pub last_seen_ms: u64,
    /// Whether the kernel speaks the gateway `on_behalf_of` contract.
    pub gateway_ready: bool,
    pub via: GatewayMachineVia,
}
