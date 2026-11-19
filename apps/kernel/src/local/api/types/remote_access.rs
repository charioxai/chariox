use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListRemoteMachinesRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListRemoteMachineKernelsRequest {
    pub machine_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApproveRemoteMachineRequest {
    pub machine_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForgetRemoteMachineRequest {
    pub machine_ref: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairingInviteIntent {
    Client,
    Machine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalType {
    Cli,
    Web,
    Ios,
    Android,
}

impl TerminalType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Web => "web",
            Self::Ios => "ios",
            Self::Android => "android",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatePairingInviteRequest {
    pub intent: PairingInviteIntent,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub expires_in_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_type: Option<TerminalType>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateTerminalPairingLinkRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_type: Option<TerminalType>,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub expires_in_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinTerminalPairingLinkRequest {
    pub pairing_link: String,
    #[serde(default)]
    pub terminal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_type: Option<TerminalType>,
    #[serde(default)]
    pub alias: Option<String>,
    /// The receiving CLI's persistent relay key. New clients receive a fresh
    /// relay token bound to this key after proving possession in the request.
    #[serde(default)]
    pub public_key_thumbprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinPairingInviteRequest {
    pub invite_token: String,
    #[serde(default)]
    pub subject_id: Option<String>,
    #[serde(default)]
    pub public_key_thumbprint: Option<String>,
    #[serde(default)]
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingInviteRecord {
    pub intent: PairingInviteIntent,
    pub invite_id: String,
    pub invite_token: String,
    pub relay_url: String,
    pub target_daemon_id: String,
    #[serde(default)]
    pub target_daemon_alias: Option<String>,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingJoinRecord {
    pub intent: PairingInviteIntent,
    pub subject_id: String,
    pub relay_url: String,
    pub target_daemon_id: String,
    #[serde(default)]
    pub alias: Option<String>,
    pub public_key_thumbprint: String,
    pub paired_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListPairedClientsRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListTerminalsRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordPairedClientRequest {
    pub client_id: String,
    pub public_key_thumbprint: String,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_type: Option<TerminalType>,
    #[serde(default)]
    pub paired_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevokePairedClientRequest {
    pub client_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedClientRecord {
    pub client_id: String,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_type: Option<TerminalType>,
    pub public_key_thumbprint: String,
    pub paired_at_ms: u64,
    pub revoked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalRecord {
    pub terminal_id: String,
    pub terminal_type: TerminalType,
    #[serde(default)]
    pub alias: Option<String>,
    pub paired_at_ms: u64,
    pub revoked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalPairingLinkRecord {
    pub terminal_id: String,
    pub pairing_link: String,
    pub pairing_code: String,
    pub invite_id: String,
    pub relay_url: String,
    pub target_daemon_id: String,
    #[serde(default)]
    pub target_daemon_alias: Option<String>,
    pub terminal_type: TerminalType,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteMachineTrustStatus {
    Approved,
    Pending,
}

/// A listed machine is reachable now or was a moment ago; one that has been gone longer is an
/// [`OfflineMachineRecord`] and is never part of a list a person picks from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteMachinePresence {
    Online,
    Reconnecting,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteMachineRecord {
    pub machine_id: String,
    #[serde(default)]
    pub machine_alias: Option<String>,
    pub display_name: String,
    pub trust_status: RemoteMachineTrustStatus,
    pub presence: RemoteMachinePresence,
    pub pending: bool,
    pub kernel_count: usize,
    #[serde(default)]
    pub available_providers: Vec<String>,
    #[serde(default)]
    pub provider_accounts: Vec<chariox_relay::protocol::RelayProviderAccountSummary>,
    /// The machine's live kernels, each with the protocol it registered with at the relay, so a
    /// kernel's update changes the machine list that clients follow.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kernels: Vec<RemoteMachineKernel>,
    /// The Cloud directory the machine belongs to (its registry record's), omitted for a machine
    /// of a private relay: clients apply the account's membership only to directory machines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,
}

/// One live kernel of a listed machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteMachineKernel {
    pub kernel_id: String,
    /// Absent for a kernel older than the field: clients show its protocol as unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_protocol: Option<u32>,
}

/// A machine this kernel saw before and that has been gone past the reconnecting window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfflineMachineRecord {
    pub machine_id: String,
    pub display_name: String,
    pub last_seen_ms: u64,
    /// As `RemoteMachineRecord::directory`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,
}

/// The machines this kernel projects: listed ones, and the offline ones kept apart.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteMachineInventory {
    pub machines: Vec<RemoteMachineRecord>,
    pub offline_machines: Vec<OfflineMachineRecord>,
}
