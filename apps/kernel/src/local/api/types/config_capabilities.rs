use super::*;
use std::fmt;
use zeroize::Zeroize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetUserConfigRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetUserConfigSchemaRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetUserConfigValueRequest {
    pub path: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetWorkspaceLiveSyncModeRequest {
    pub session_id: String,
    pub mode: crate::config::WorkspaceLiveSyncMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsetUserConfigValueRequest {
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserConfigProviderReloadSummary {
    pub reloaded: u32,
    pub deferred: u32,
    pub unaffected: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserConfigMutationEffect {
    pub kind: String,
    pub path: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_reload: Option<UserConfigProviderReloadSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetCredentialSecretRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteCredentialSecretRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetCredentialVaultStatusRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockCredentialVaultRequest;

/// The Vault passphrase the owner typed. Redacted in `Debug` and zeroed on drop.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VaultPassphrase(String);

impl VaultPassphrase {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for VaultPassphrase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("VaultPassphrase([REDACTED])")
    }
}

impl Drop for VaultPassphrase {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// Unlocks the kernel's existing Vault from the owner's own client, with no
/// session or agent. It never creates a Vault.
/// How long an unlocked Vault stays open. The owner picks one on every unlock;
/// `credential_vault.unlock_policy` only decides which one is preselected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum VaultUnlockDuration {
    /// The next operation, then the Vault closes again.
    Once,
    Minutes {
        minutes: u64,
    },
    UntilKernelStops,
}

/// Where a Vault unlock was typed. An unlock from any of them answers the open unlock
/// cards of agents as "unlocked", and the card's answer names the surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultUnlockSurface {
    /// An agent's unlock card in a Room.
    Card,
    VaultWindow,
    /// A Settings popup that needs the Vault open.
    Settings,
    /// The classic Vault sidebar panel.
    Sidebar,
    /// The command line or shell.
    Terminal,
}

impl VaultUnlockSurface {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Card => "card",
            Self::VaultWindow => "vault_window",
            Self::Settings => "settings",
            Self::Sidebar => "sidebar",
            Self::Terminal => "terminal",
        }
    }
}

/// The durations a client offers when it asks for the Vault passphrase, and
/// the one it preselects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultUnlockOptions {
    pub durations: Vec<VaultUnlockDuration>,
    pub default: VaultUnlockDuration,
}

impl VaultUnlockDuration {
    /// The lease behind the owner's choice. "Once" ends with the agent's
    /// operation when an agent asked, and with the owner client's next
    /// operation otherwise.
    pub(crate) fn lease(
        self,
        config: &crate::config::UserCredentialVaultConfig,
        for_agent_operation: bool,
    ) -> crate::secret::VaultUnlockLease {
        use crate::secret::VaultUnlockLease;
        match self {
            Self::Once if for_agent_operation => VaultUnlockLease::Operation,
            Self::Once => VaultUnlockLease::SingleUse,
            Self::Minutes { minutes } => {
                VaultUnlockLease::TtlMinutes(minutes.clamp(1, config.max_ttl_minutes.max(1)))
            }
            Self::UntilKernelStops => VaultUnlockLease::KernelShutdown,
        }
    }
}

impl VaultUnlockOptions {
    /// What every unlock question offers; the configured policy only
    /// preselects one of the durations.
    pub(crate) fn for_config(config: &crate::config::UserCredentialVaultConfig) -> Self {
        use crate::config::CredentialVaultUnlockPolicy;
        let longest = config.max_ttl_minutes.max(1);
        let default_minutes = config.default_ttl_minutes.clamp(1, longest);
        let mut minutes = vec![30.min(longest), 60.min(longest), default_minutes];
        minutes.sort_unstable();
        minutes.dedup();
        let mut durations = vec![VaultUnlockDuration::Once];
        durations.extend(
            minutes
                .into_iter()
                .map(|minutes| VaultUnlockDuration::Minutes { minutes }),
        );
        durations.push(VaultUnlockDuration::UntilKernelStops);
        let default = match config.unlock_policy {
            CredentialVaultUnlockPolicy::Always => VaultUnlockDuration::Once,
            CredentialVaultUnlockPolicy::KernelInit => VaultUnlockDuration::UntilKernelStops,
            CredentialVaultUnlockPolicy::Ttl => VaultUnlockDuration::Minutes {
                minutes: default_minutes,
            },
        };
        Self { durations, default }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnlockCredentialVaultRequest {
    pub passphrase: VaultPassphrase,
    pub duration: VaultUnlockDuration,
    /// Where the owner typed it; never `card`, which is answered through its interaction.
    pub surface: VaultUnlockSurface,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManageCredentialVaultRequest {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallMcpServerRequest {
    pub workspace_id: Option<String>,
    pub config: CharioxMcpServerConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateMcpServerRequest {
    pub workspace_id: Option<String>,
    pub config: CharioxMcpServerConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UninstallMcpServerRequest {
    pub workspace_id: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportMcpServersRequest {
    pub workspace_id: Option<String>,
    pub provider: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportProviderCapabilitiesRequest {
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapabilityImportReport {
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub providers: Vec<String>,
    pub summary: ProviderCapabilityImportSummary,
    #[serde(default)]
    pub mcps: Vec<ProviderCapabilityImportEntry>,
    #[serde(default)]
    pub skills: Vec<ProviderCapabilityImportEntry>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapabilityImportSummary {
    #[serde(default)]
    pub candidates: usize,
    #[serde(default)]
    pub imported: usize,
    #[serde(default)]
    pub updated: usize,
    #[serde(default)]
    pub already_installed: usize,
    #[serde(default)]
    pub deduped: usize,
    #[serde(default)]
    pub skipped: usize,
    #[serde(default)]
    pub errors: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapabilityImportEntry {
    pub kind: String,
    pub name: String,
    pub provider: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    pub action: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub duplicates: Vec<ProviderCapabilityImportDuplicate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapabilityImportDuplicate {
    pub provider: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetMcpServerRequest {
    pub workspace_id: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetSkillRequest {
    pub workspace_id: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterEnvironmentRequest {
    pub workspace_id: Option<String>,
    pub config: CharioxEnvironmentConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoveEnvironmentRequest {
    pub workspace_id: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetEnvironmentRequest {
    pub workspace_id: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListEnvironmentsRequest {
    pub workspace_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidateScriptRequest {
    pub workspace_id: Option<String>,
    pub source_path: PathBuf,
    pub environment: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterScriptRequest {
    pub workspace_id: Option<String>,
    pub source_path: PathBuf,
    pub environment: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoveScriptRequest {
    pub workspace_id: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetScriptRequest {
    pub workspace_id: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListScriptsRequest {
    pub workspace_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterCredentialRequest {
    pub source_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpsertCredentialRequest {
    pub credential: UserCredentialConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoveCredentialRequest {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetCredentialRequest {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListCredentialsRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterConnectorRequest {
    pub source_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpsertConnectorRequest {
    pub connector: CharioxConnectorDefinition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterConnectorAdapterRequest {
    pub source_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoveConnectorAdapterRequest {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetConnectorAdapterRequest {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListConnectorAdaptersRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoveConnectorRequest {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetConnectorRequest {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListConnectorsRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestConnectorRequest {
    pub name: String,
    pub operation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
    #[serde(default)]
    pub input: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpsertSkillRequest {
    pub workspace_id: Option<String>,
    pub source: SkillInstallSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SkillInstallSource {
    Content { skill_md: String },
    Url { url: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallSkillRequest {
    pub workspace_id: Option<String>,
    pub source_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateSkillRequest {
    pub workspace_id: Option<String>,
    pub source_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UninstallSkillRequest {
    pub workspace_id: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportSkillsRequest {
    pub workspace_id: Option<String>,
    pub provider: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionKind {
    Mcp,
    Skill,
    Script,
    Connector,
    App,
}

impl From<ExtensionKind> for crate::extension::ExtensionKind {
    fn from(value: ExtensionKind) -> Self {
        match value {
            ExtensionKind::Mcp => Self::Mcp,
            ExtensionKind::Skill => Self::Skill,
            ExtensionKind::Script => Self::Script,
            ExtensionKind::Connector => Self::Connector,
            ExtensionKind::App => Self::App,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantAgentExtensionRequest {
    pub workspace_id: Option<String>,
    pub agent_ref: String,
    pub kind: ExtensionKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_safety: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevokeAgentExtensionRequest {
    pub agent_ref: String,
    pub kind: ExtensionKind,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncRemoteExtensionManifestRequest {
    pub agent_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListHomeExtensionAuditRequest {
    pub agent_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListMcpServersRequest {
    pub workspace_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListSkillsRequest {
    pub workspace_id: Option<String>,
}

/// Owner controls for the same bound entries used by agents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum OwnerVaultRequest {
    Create {
        name: String,
        kind: String,
        use_binding: crate::config::VaultUseBinding,
        value: VaultPassphrase,
    },
    Generate {
        name: String,
        kind: String,
        use_binding: crate::config::VaultUseBinding,
    },
    Reveal {
        id: String,
    },
    Activity {
        id: String,
    },
    Replace {
        id: String,
        value: VaultPassphrase,
    },
    Delete {
        id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerVaultActivity {
    pub at_ms: u64,
    pub event: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum OwnerVaultResult {
    Saved {
        id: String,
    },
    Deleted {
        id: String,
    },
    Revealed {
        id: String,
        value: VaultPassphrase,
    },
    Activity {
        id: String,
        events: Vec<OwnerVaultActivity>,
    },
}
