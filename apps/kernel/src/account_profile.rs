//! Kernel-owned provider account profiles.
//!
//! Provider CLIs continue to own credential formats. This registry stores only
//! stable Chariox selection metadata and host-local provider root locators.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use base64::Engine as _;
use rand::{distributions::Alphanumeric, Rng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::DaemonError;

const REGISTRY_VERSION: u32 = 1;
const SUPPORTED_PROVIDERS: [&str; 3] = ["codex", "claude", "opencode"];
const MAX_MATERIALIZATION_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const MAX_MANAGED_CONTEXT_MATERIALIZATION_BYTES: usize = 16 * 1024 * 1024;
#[cfg(test)]
thread_local! {
    static FAIL_MANAGED_CONTEXT_ROLLBACK_CLEANUP_ONCE: std::cell::Cell<bool> = const {
        std::cell::Cell::new(false)
    };
    static FAIL_ACCOUNT_PROFILE_REGISTRY_PARENT_SYNC_ONCE: std::cell::Cell<bool> = const {
        std::cell::Cell::new(false)
    };
}

/// Provider accounts belong to the person operating the home kernel. Local
/// clients identify that person as `local`, while the owner's Cloud clients
/// use the configured Cloud user id. Only that configured id aliases the host
/// owner; collaborators retain independent account namespaces.
pub(crate) fn provider_account_authority_owner_user_id(
    config: &crate::config::DaemonConfig,
    runtime_owner_user_id: &str,
) -> String {
    if config
        .cloud_relay
        .as_ref()
        .is_some_and(|profile| profile.user_id == runtime_owner_user_id)
    {
        crate::session::DEFAULT_LOCAL_USER_ID.to_string()
    } else {
        runtime_owner_user_id.to_string()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderAccountMaterializationFile {
    pub relative_path: String,
    pub contents_base64: String,
}

impl std::fmt::Debug for ProviderAccountMaterializationFile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderAccountMaterializationFile")
            .field("relative_path", &self.relative_path)
            .field("contents_base64", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderAccountReplicaMetadata {
    pub owner_user_id: String,
    pub provider: String,
    pub profile_id: String,
    pub label: String,
    pub origin: ProviderAccountProfileOrigin,
    pub is_default: bool,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderAccountMaterialization {
    pub profile: ProviderAccountReplicaMetadata,
    pub files: Vec<ProviderAccountMaterializationFile>,
    pub generated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ManagedContextProviderAccountReceipt {
    pub context_id: String,
    pub package_sha256: String,
    pub materialization_sha256: String,
    pub provider: String,
    pub profile_id: String,
}

impl std::fmt::Debug for ProviderAccountMaterialization {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderAccountMaterialization")
            .field("profile", &self.profile)
            .field("file_count", &self.files.len())
            .field("generated_at_ms", &self.generated_at_ms)
            .finish()
    }
}

/// Ambient credential variables must never override the provider-native state
/// selected by an account profile. OpenCode supports many upstreams, so its
/// list intentionally covers the common official provider integrations.
pub(crate) fn provider_auth_env_vars(provider: &str) -> &'static [&'static str] {
    match crate::provider::canonical_provider_family(provider) {
        Some("codex") => &["OPENAI_API_KEY", "CODEX_API_KEY"],
        Some("claude") => &[
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_BASE_URL",
            "ANTHROPIC_CUSTOM_HEADERS",
        ],
        Some("opencode") => &[
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "AZURE_OPENAI_API_KEY",
            "OPENROUTER_API_KEY",
            "FIREWORKS_API_KEY",
            "GOOGLE_GENERATIVE_AI_API_KEY",
            "GEMINI_API_KEY",
            "GROQ_API_KEY",
            "MISTRAL_API_KEY",
            "COHERE_API_KEY",
            "DEEPSEEK_API_KEY",
            "XAI_API_KEY",
            "AWS_ACCESS_KEY_ID",
            "AWS_SECRET_ACCESS_KEY",
            "AWS_SESSION_TOKEN",
        ],
        _ => &[],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAccountProfileOrigin {
    Default,
    CharioxCreated,
    Linked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAccountAuthState {
    Unknown,
    NotConfigured,
    Authenticated,
    Expired,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAccountUsageAvailability {
    Available,
    Partial,
    Unavailable,
    Stale,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAccountUsageMeterKind {
    RollingLimit,
    CreditBalance,
    SpendLimit,
    TokenUsage,
    LocalCost,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAccountUsageMeterScope {
    Account,
    Workspace,
    Model,
    UpstreamProvider,
    Plan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAccountUsageMeterState {
    Healthy,
    Warning,
    Exhausted,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAccountMaterializationTargetKind {
    Worker,
    Slice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAccountMaterializationState {
    Materialized,
    Stale,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderAccountMaterializationStatus {
    pub target_kind: ProviderAccountMaterializationTargetKind,
    pub target_ref: String,
    pub state: ProviderAccountMaterializationState,
    pub observed_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderAccountUsageMeter {
    pub meter_id: String,
    pub label: String,
    pub kind: ProviderAccountUsageMeterKind,
    pub scope: ProviderAccountUsageMeterScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_duration_minutes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at_ms: Option<u64>,
    pub state: ProviderAccountUsageMeterState,
    pub source: String,
    pub observed_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderAccountUsageSnapshot {
    pub profile_id: String,
    pub provider: String,
    pub availability: ProviderAccountUsageAvailability,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub meters: Vec<ProviderAccountUsageMeter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at_ms: Option<u64>,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub management_url: Option<String>,
}

impl ProviderAccountUsageSnapshot {
    pub fn unavailable(profile_id: impl Into<String>, provider: impl Into<String>) -> Self {
        let provider = provider.into();
        Self {
            profile_id: profile_id.into(),
            provider: provider.clone(),
            availability: ProviderAccountUsageAvailability::Unavailable,
            meters: Vec::new(),
            observed_at_ms: None,
            source: "provider_not_observed".to_string(),
            management_url: match provider.as_str() {
                "codex" => Some("https://chatgpt.com/codex/settings/usage".to_string()),
                "claude" => Some("https://claude.ai/settings/usage".to_string()),
                "opencode" => Some("https://opencode.ai/zen".to_string()),
                _ => None,
            },
        }
    }
}

/// Safe account metadata projected to clients. Host-local paths are
/// deliberately absent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderAccountProfile {
    pub owner_user_id: String,
    pub provider: String,
    pub profile_id: String,
    pub label: String,
    pub origin: ProviderAccountProfileOrigin,
    pub is_default: bool,
    pub auth_state: ProviderAccountAuthState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detected_provider_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_validated_at_ms: Option<u64>,
    pub usage: ProviderAccountUsageSnapshot,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub materializations: Vec<ProviderAccountMaterializationStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "snake_case")]
pub(crate) enum ProviderAccountLocator {
    Codex {
        codex_home: PathBuf,
    },
    Claude {
        claude_config_dir: PathBuf,
    },
    Opencode {
        xdg_data_home: PathBuf,
        xdg_config_home: PathBuf,
        xdg_state_home: PathBuf,
        xdg_cache_home: PathBuf,
        opencode_config_dir: PathBuf,
    },
}

impl ProviderAccountLocator {
    fn managed(provider: &str, root: &Path) -> Result<Self, DaemonError> {
        match provider {
            "codex" => Ok(Self::Codex {
                codex_home: root.join("codex"),
            }),
            "claude" => Ok(Self::Claude {
                claude_config_dir: root.join("claude"),
            }),
            "opencode" => {
                let config = root.join("config");
                Ok(Self::Opencode {
                    xdg_data_home: root.join("data"),
                    xdg_config_home: config.clone(),
                    xdg_state_home: root.join("state"),
                    xdg_cache_home: root.join("cache"),
                    opencode_config_dir: config.join("opencode"),
                })
            }
            _ => Err(unsupported_provider(provider)),
        }
    }

    fn linked(provider: &str, root: PathBuf) -> Result<Self, DaemonError> {
        match provider {
            "codex" => Ok(Self::Codex { codex_home: root }),
            "claude" => Ok(Self::Claude {
                claude_config_dir: root,
            }),
            "opencode" => Self::managed(provider, &root),
            _ => Err(unsupported_provider(provider)),
        }
    }

    fn effective_default(provider: &str, home: &Path) -> Result<Self, DaemonError> {
        match provider {
            "codex" => Ok(Self::Codex {
                codex_home: std::env::var_os("CODEX_HOME")
                    .filter(|value| !value.is_empty())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join(".codex")),
            }),
            "claude" => Ok(Self::Claude {
                claude_config_dir: std::env::var_os("CLAUDE_CONFIG_DIR")
                    .filter(|value| !value.is_empty())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join(".claude")),
            }),
            "opencode" => {
                let data = effective_xdg("XDG_DATA_HOME", home.join(".local/share"));
                let config = effective_xdg("XDG_CONFIG_HOME", home.join(".config"));
                let state = effective_xdg("XDG_STATE_HOME", home.join(".local/state"));
                let cache = effective_xdg("XDG_CACHE_HOME", home.join(".cache"));
                let opencode_config_dir = std::env::var_os("OPENCODE_CONFIG_DIR")
                    .filter(|value| !value.is_empty())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| config.join("opencode"));
                Ok(Self::Opencode {
                    xdg_data_home: data,
                    xdg_config_home: config,
                    xdg_state_home: state,
                    xdg_cache_home: cache,
                    opencode_config_dir,
                })
            }
            _ => Err(unsupported_provider(provider)),
        }
    }

    fn roots(&self) -> Vec<&Path> {
        match self {
            Self::Codex { codex_home } => vec![codex_home],
            Self::Claude { claude_config_dir } => vec![claude_config_dir],
            Self::Opencode {
                xdg_data_home,
                xdg_config_home,
                xdg_state_home,
                xdg_cache_home,
                opencode_config_dir,
            } => vec![
                xdg_data_home,
                xdg_config_home,
                xdg_state_home,
                xdg_cache_home,
                opencode_config_dir,
            ],
        }
    }

    pub(crate) fn environment(&self) -> BTreeMap<String, String> {
        match self {
            Self::Codex { codex_home } => {
                BTreeMap::from([("CODEX_HOME".to_string(), codex_home.display().to_string())])
            }
            Self::Claude { claude_config_dir } => BTreeMap::from([(
                "CLAUDE_CONFIG_DIR".to_string(),
                claude_config_dir.display().to_string(),
            )]),
            Self::Opencode {
                xdg_data_home,
                xdg_config_home,
                xdg_state_home,
                xdg_cache_home,
                opencode_config_dir,
            } => BTreeMap::from([
                (
                    "XDG_DATA_HOME".to_string(),
                    xdg_data_home.display().to_string(),
                ),
                (
                    "XDG_CONFIG_HOME".to_string(),
                    xdg_config_home.display().to_string(),
                ),
                (
                    "XDG_STATE_HOME".to_string(),
                    xdg_state_home.display().to_string(),
                ),
                (
                    "XDG_CACHE_HOME".to_string(),
                    xdg_cache_home.display().to_string(),
                ),
                (
                    "OPENCODE_CONFIG_DIR".to_string(),
                    opencode_config_dir.display().to_string(),
                ),
            ]),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct StoredProviderAccountProfile {
    #[serde(flatten)]
    public: ProviderAccountProfile,
    locator: ProviderAccountLocator,
    #[serde(default)]
    materialized_replica: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    managed_context_replica: Option<ManagedContextReplicaBinding>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ManagedContextReplicaBinding {
    context_id: String,
    package_sha256: String,
    materialization_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    replaced_profile: Option<ReplacedProviderAccountProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous_default_profile_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ReplacedProviderAccountProfile {
    public: ProviderAccountProfile,
    locator: ProviderAccountLocator,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RegistryDocument {
    version: u32,
    #[serde(default)]
    profiles: Vec<StoredProviderAccountProfile>,
}

impl Default for RegistryDocument {
    fn default() -> Self {
        Self {
            version: REGISTRY_VERSION,
            profiles: Vec::new(),
        }
    }
}

#[derive(Clone)]
pub struct ProviderAccountProfileRegistry {
    path: PathBuf,
    document: Arc<RwLock<RegistryDocument>>,
}

impl ProviderAccountProfileRegistry {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, DaemonError> {
        let path = path.into();
        let document = if path.exists() {
            let bytes = fs::read(&path).map_err(registry_io("read account profile registry"))?;
            let document: RegistryDocument = serde_json::from_slice(&bytes).map_err(|error| {
                registry_error("read account profile registry", error.to_string())
            })?;
            if document.version != REGISTRY_VERSION {
                return Err(registry_error(
                    "read account profile registry",
                    format!(
                        "unsupported registry version {}, expected {REGISTRY_VERSION}",
                        document.version
                    ),
                ));
            }
            document
        } else {
            RegistryDocument::default()
        };
        Ok(Self {
            path,
            document: Arc::new(RwLock::new(document)),
        })
    }

    pub fn migrate_effective_defaults(
        &self,
        owner_user_id: &str,
        home: &Path,
    ) -> Result<Vec<ProviderAccountProfile>, DaemonError> {
        let mut document = self.write_document()?;
        let mut changed = false;
        for provider in SUPPORTED_PROVIDERS {
            if document.profiles.iter().any(|profile| {
                profile.public.owner_user_id == owner_user_id && profile.public.provider == provider
            }) {
                continue;
            }
            let locator = ProviderAccountLocator::effective_default(provider, home)?;
            create_private_roots(&locator)?;
            if let ProviderAccountLocator::Codex { codex_home } = &locator {
                enforce_codex_file_credentials(codex_home)?;
            }
            let profile = new_public_profile(
                owner_user_id,
                provider,
                "default",
                "Default",
                ProviderAccountProfileOrigin::Default,
                true,
            );
            document.profiles.push(StoredProviderAccountProfile {
                public: profile,
                locator,
                materialized_replica: false,
                managed_context_replica: None,
            });
            changed = true;
        }
        if changed {
            self.persist_locked(&document)?;
        }
        Ok(document
            .profiles
            .iter()
            .filter(|profile| profile.public.owner_user_id == owner_user_id)
            .map(|profile| profile.public.clone())
            .collect())
    }

    pub fn list(
        &self,
        owner_user_id: &str,
        provider: Option<&str>,
    ) -> Result<Vec<ProviderAccountProfile>, DaemonError> {
        let provider = provider.map(normalize_provider).transpose()?;
        let document = self.read_document()?;
        Ok(document
            .profiles
            .iter()
            .filter(|profile| {
                profile.public.owner_user_id == owner_user_id
                    && provider
                        .as_deref()
                        .is_none_or(|provider| profile.public.provider == provider)
            })
            .map(|profile| profile.public.clone())
            .collect())
    }

    pub(crate) fn list_all(&self) -> Result<Vec<ProviderAccountProfile>, DaemonError> {
        let document = self.read_document()?;
        Ok(document
            .profiles
            .iter()
            .map(|profile| profile.public.clone())
            .collect())
    }

    pub fn get(
        &self,
        owner_user_id: &str,
        provider: &str,
        profile_id: &str,
    ) -> Result<ProviderAccountProfile, DaemonError> {
        let provider = normalize_provider(provider)?;
        let document = self.read_document()?;
        resolve_stored_profile(&document, owner_user_id, provider, profile_id)
            .map(|profile| profile.public.clone())
    }

    pub fn update_observation(
        &self,
        owner_user_id: &str,
        provider: &str,
        profile_id: &str,
        auth_state: ProviderAccountAuthState,
        identity_summary: Option<String>,
        plan: Option<String>,
        detected_provider_version: Option<String>,
        usage: Option<ProviderAccountUsageSnapshot>,
    ) -> Result<ProviderAccountProfile, DaemonError> {
        let provider = normalize_provider(provider)?;
        let mut document = self.write_document()?;
        let profile =
            resolve_stored_profile_mut(&mut document, owner_user_id, provider, profile_id)?;
        let identity_changed = profile.public.identity_summary.is_some()
            && identity_summary.is_some()
            && profile.public.identity_summary != identity_summary;
        if identity_changed || auth_state != ProviderAccountAuthState::Authenticated {
            mark_profile_materializations_stale(&mut profile.public);
        }
        profile.public.auth_state = auth_state;
        profile.public.identity_summary = identity_summary;
        profile.public.plan = plan;
        profile.public.detected_provider_version = detected_provider_version;
        profile.public.last_validated_at_ms = Some(crate::session::unix_epoch_ms());
        if let Some(usage) = usage {
            profile.public.usage = usage;
        }
        let result = profile.public.clone();
        self.persist_locked(&document)?;
        Ok(result)
    }

    pub fn mark_logged_out(
        &self,
        owner_user_id: &str,
        provider: &str,
        profile_id: &str,
    ) -> Result<ProviderAccountProfile, DaemonError> {
        let provider = normalize_provider(provider)?;
        let mut document = self.write_document()?;
        let profile =
            resolve_stored_profile_mut(&mut document, owner_user_id, provider, profile_id)?;
        profile.public.auth_state = ProviderAccountAuthState::NotConfigured;
        profile.public.identity_summary = None;
        profile.public.plan = None;
        profile.public.last_validated_at_ms = Some(crate::session::unix_epoch_ms());
        profile.public.usage = ProviderAccountUsageSnapshot::unavailable(profile_id, provider);
        mark_profile_materializations_stale(&mut profile.public);
        let result = profile.public.clone();
        self.persist_locked(&document)?;
        Ok(result)
    }

    pub fn update_usage(
        &self,
        owner_user_id: &str,
        provider: &str,
        profile_id: &str,
        mut usage: ProviderAccountUsageSnapshot,
    ) -> Result<ProviderAccountProfile, DaemonError> {
        let provider = normalize_provider(provider)?;
        let mut document = self.write_document()?;
        let profile =
            resolve_stored_profile_mut(&mut document, owner_user_id, provider, profile_id)?;
        usage.profile_id = profile.public.profile_id.clone();
        usage.provider = provider.to_string();
        if usage.availability != ProviderAccountUsageAvailability::Unavailable {
            let mut merged = profile.public.usage.meters.clone();
            for meter in usage.meters.drain(..) {
                if let Some(existing) = merged
                    .iter_mut()
                    .find(|existing| existing.meter_id == meter.meter_id)
                {
                    *existing = meter;
                } else {
                    merged.push(meter);
                }
            }
            usage.meters = merged;
        }
        profile.public.usage = usage;
        let result = profile.public.clone();
        self.persist_locked(&document)?;
        Ok(result)
    }

    pub(crate) fn update_materialization_status(
        &self,
        owner_user_id: &str,
        provider: &str,
        profile_id: &str,
        status: ProviderAccountMaterializationStatus,
    ) -> Result<ProviderAccountProfile, DaemonError> {
        let provider = normalize_provider(provider)?;
        let mut document = self.write_document()?;
        let profile =
            resolve_stored_profile_mut(&mut document, owner_user_id, provider, profile_id)?;
        if let Some(existing) = profile.public.materializations.iter_mut().find(|existing| {
            existing.target_kind == status.target_kind && existing.target_ref == status.target_ref
        }) {
            *existing = status;
        } else {
            profile.public.materializations.push(status);
        }
        let result = profile.public.clone();
        self.persist_locked(&document)?;
        Ok(result)
    }

    pub(crate) fn resolve_environment(
        &self,
        owner_user_id: &str,
        provider: &str,
        profile_id: &str,
    ) -> Result<BTreeMap<String, String>, DaemonError> {
        let provider = normalize_provider(provider)?;
        let (origin, locator, materialized_replica) = {
            let document = self.read_document()?;
            let profile = resolve_stored_profile(&document, owner_user_id, provider, profile_id)?;
            (
                profile.public.origin,
                profile.locator.clone(),
                profile.materialized_replica,
            )
        };
        if crate::provider::managed_provider_isolation_required()
            && origin == ProviderAccountProfileOrigin::Linked
            && !materialized_replica
        {
            return Err(registry_error(
                "resolve account profile",
                "managed kernels cannot mount a host-linked provider account; transfer or materialize the account into the managed kernel first",
            ));
        }
        if origin == ProviderAccountProfileOrigin::Default {
            create_private_roots(&locator)?;
            if let ProviderAccountLocator::Codex { codex_home } = &locator {
                enforce_codex_file_credentials(codex_home)?;
            }
        }
        Ok(locator.environment())
    }

    pub fn create_managed(
        &self,
        owner_user_id: &str,
        provider: &str,
        label: &str,
    ) -> Result<ProviderAccountProfile, DaemonError> {
        let provider = normalize_provider(provider)?;
        let label = validate_label(label)?;
        let mut document = self.write_document()?;
        ensure_unique_label(&document, owner_user_id, provider, label)?;
        let profile_id = unique_profile_id(&document, owner_user_id, provider, label);
        let managed_root = self
            .path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("provider-accounts")
            .join(safe_path_component(owner_user_id))
            .join(provider)
            .join(&profile_id);
        let locator = ProviderAccountLocator::managed(provider, &managed_root)?;
        create_private_roots(&locator)?;
        if let ProviderAccountLocator::Codex { codex_home } = &locator {
            enforce_codex_file_credentials(codex_home)?;
        }
        let profile = new_public_profile(
            owner_user_id,
            provider,
            &profile_id,
            label,
            ProviderAccountProfileOrigin::CharioxCreated,
            false,
        );
        document.profiles.push(StoredProviderAccountProfile {
            public: profile.clone(),
            locator,
            materialized_replica: false,
            managed_context_replica: None,
        });
        self.persist_locked(&document)?;
        Ok(profile)
    }

    pub fn link_existing(
        &self,
        owner_user_id: &str,
        provider: &str,
        label: &str,
        path: &Path,
    ) -> Result<ProviderAccountProfile, DaemonError> {
        let provider = normalize_provider(provider)?;
        let label = validate_label(label)?;
        if crate::provider::managed_provider_isolation_required() {
            return Err(registry_error(
                "link account profile",
                "managed kernels cannot link arbitrary host provider-account paths; transfer or materialize the account instead",
            ));
        }
        let canonical = validate_linked_root(path)?;
        let mut document = self.write_document()?;
        ensure_unique_label(&document, owner_user_id, provider, label)?;
        let profile_id = unique_profile_id(&document, owner_user_id, provider, label);
        let profile = new_public_profile(
            owner_user_id,
            provider,
            &profile_id,
            label,
            ProviderAccountProfileOrigin::Linked,
            false,
        );
        let locator = ProviderAccountLocator::linked(provider, canonical)?;
        if let ProviderAccountLocator::Codex { codex_home } = &locator {
            enforce_codex_file_credentials(codex_home)?;
        }
        document.profiles.push(StoredProviderAccountProfile {
            public: profile.clone(),
            locator,
            materialized_replica: false,
            managed_context_replica: None,
        });
        self.persist_locked(&document)?;
        Ok(profile)
    }

    pub fn rename(
        &self,
        owner_user_id: &str,
        provider: &str,
        profile_id: &str,
        label: &str,
    ) -> Result<ProviderAccountProfile, DaemonError> {
        let provider = normalize_provider(provider)?;
        let label = validate_label(label)?;
        let mut document = self.write_document()?;
        ensure_unique_label_except(&document, owner_user_id, provider, label, profile_id)?;
        let profile =
            resolve_stored_profile_mut(&mut document, owner_user_id, provider, profile_id)?;
        profile.public.label = label.to_string();
        let result = profile.public.clone();
        self.persist_locked(&document)?;
        Ok(result)
    }

    pub fn set_default(
        &self,
        owner_user_id: &str,
        provider: &str,
        profile_id: &str,
    ) -> Result<ProviderAccountProfile, DaemonError> {
        let provider = normalize_provider(provider)?;
        let mut document = self.write_document()?;
        let resolved_id = resolve_stored_profile(&document, owner_user_id, provider, profile_id)?
            .public
            .profile_id
            .clone();
        for profile in &mut document.profiles {
            if profile.public.owner_user_id == owner_user_id && profile.public.provider == provider
            {
                profile.public.is_default = profile.public.profile_id == resolved_id;
            }
        }
        let result = resolve_stored_profile(&document, owner_user_id, provider, &resolved_id)?
            .public
            .clone();
        self.persist_locked(&document)?;
        Ok(result)
    }

    pub fn remove_registration(
        &self,
        owner_user_id: &str,
        provider: &str,
        profile_id: &str,
    ) -> Result<ProviderAccountProfile, DaemonError> {
        let provider = normalize_provider(provider)?;
        let mut document = self.write_document()?;
        let index = resolved_profile_index(&document, owner_user_id, provider, profile_id)?;
        let removed = document.profiles.remove(index);
        if removed.public.is_default {
            if let Some(next) = document.profiles.iter_mut().find(|profile| {
                profile.public.owner_user_id == owner_user_id && profile.public.provider == provider
            }) {
                next.public.is_default = true;
            }
        }
        self.persist_locked(&document)?;
        Ok(removed.public)
    }

    pub fn delete_managed_profile_data(
        &self,
        owner_user_id: &str,
        provider: &str,
        profile_id: &str,
        confirmation_profile_id: &str,
    ) -> Result<ProviderAccountProfile, DaemonError> {
        if profile_id != confirmation_profile_id {
            return Err(registry_error(
                "delete account profile",
                "destructive confirmation does not match profile id",
            ));
        }
        let provider = normalize_provider(provider)?;
        let mut document = self.write_document()?;
        let index = resolved_profile_index(&document, owner_user_id, provider, profile_id)?;
        let stored = &document.profiles[index];
        if stored.public.origin != ProviderAccountProfileOrigin::CharioxCreated {
            return Err(registry_error(
                "delete account profile",
                "only Chariox-created profile data can be deleted",
            ));
        }
        let mut roots = stored.locator.roots();
        roots.sort_by_key(|root| std::cmp::Reverse(root.components().count()));
        for root in roots {
            if root.exists() {
                remove_managed_root(root, &self.path)?;
            }
        }
        let removed = document.profiles.remove(index);
        if removed.public.is_default {
            if let Some(next) = document.profiles.iter_mut().find(|profile| {
                profile.public.owner_user_id == owner_user_id && profile.public.provider == provider
            }) {
                next.public.is_default = true;
            }
        }
        self.persist_locked(&document)?;
        Ok(removed.public)
    }

    pub(crate) fn export_materialization(
        &self,
        owner_user_id: &str,
        provider: &str,
        profile_id: &str,
    ) -> Result<ProviderAccountMaterialization, DaemonError> {
        let provider = normalize_provider(provider)?;
        let document = self.read_document()?;
        let stored = resolve_stored_profile(&document, owner_user_id, provider, profile_id)?;
        let mut files = Vec::new();
        match &stored.locator {
            ProviderAccountLocator::Codex { codex_home } => {
                collect_optional_file(codex_home, "auth.json", "auth.json", &mut files)?;
                collect_optional_file(codex_home, "config.toml", "config.toml", &mut files)?;
            }
            ProviderAccountLocator::Claude { claude_config_dir } => {
                for name in [".credentials.json", "settings.json", "stats-cache.json"] {
                    collect_optional_file(claude_config_dir, name, name, &mut files)?;
                }
                if stored.public.origin == ProviderAccountProfileOrigin::Default
                    && !files
                        .iter()
                        .any(|file| file.relative_path == ".credentials.json")
                {
                    collect_default_claude_keychain_credentials(&mut files)?;
                }
            }
            ProviderAccountLocator::Opencode {
                xdg_data_home,
                xdg_config_home,
                xdg_state_home,
                opencode_config_dir,
                ..
            } => {
                collect_optional_tree(
                    &xdg_data_home.join("opencode"),
                    "data/opencode",
                    &mut files,
                )?;
                collect_optional_tree(
                    &xdg_config_home.join("opencode"),
                    "config/opencode",
                    &mut files,
                )?;
                collect_optional_tree(
                    &xdg_state_home.join("opencode"),
                    "state/opencode",
                    &mut files,
                )?;
                if opencode_config_dir != &xdg_config_home.join("opencode") {
                    collect_optional_tree(opencode_config_dir, "opencode-config", &mut files)?;
                }
            }
        }
        Ok(ProviderAccountMaterialization {
            profile: ProviderAccountReplicaMetadata {
                owner_user_id: stored.public.owner_user_id.clone(),
                provider: stored.public.provider.clone(),
                profile_id: stored.public.profile_id.clone(),
                label: stored.public.label.clone(),
                origin: stored.public.origin,
                is_default: stored.public.is_default,
            },
            files,
            generated_at_ms: crate::session::unix_epoch_ms(),
        })
    }

    pub(crate) fn export_managed_context_materialization(
        &self,
        owner_user_id: &str,
        provider: &str,
        profile_id: &str,
    ) -> Result<ProviderAccountMaterialization, DaemonError> {
        let provider = normalize_provider(provider)?;
        let document = self.read_document()?;
        let stored = resolve_stored_profile(&document, owner_user_id, provider, profile_id)?;
        let mut files = Vec::new();
        match &stored.locator {
            ProviderAccountLocator::Codex { codex_home } => {
                validate_materialization_root(codex_home)?;
                collect_optional_file_bounded(
                    codex_home,
                    "auth.json",
                    "auth.json",
                    &mut files,
                    MAX_MANAGED_CONTEXT_MATERIALIZATION_BYTES,
                )?;
                require_materialization_file(&files, "auth.json", provider, profile_id)?;
            }
            ProviderAccountLocator::Claude { claude_config_dir } => {
                validate_materialization_root(claude_config_dir)?;
                collect_optional_file_bounded(
                    claude_config_dir,
                    ".credentials.json",
                    ".credentials.json",
                    &mut files,
                    MAX_MANAGED_CONTEXT_MATERIALIZATION_BYTES,
                )?;
                if stored.public.origin == ProviderAccountProfileOrigin::Default
                    && !files
                        .iter()
                        .any(|file| file.relative_path == ".credentials.json")
                {
                    collect_default_claude_keychain_credentials(&mut files)?;
                }
                require_materialization_file(&files, ".credentials.json", provider, profile_id)?;
            }
            ProviderAccountLocator::Opencode { xdg_data_home, .. } => {
                let auth_root = xdg_data_home.join("opencode");
                validate_materialization_root(&auth_root)?;
                collect_optional_file_bounded(
                    &auth_root,
                    "auth.json",
                    "data/opencode/auth.json",
                    &mut files,
                    MAX_MANAGED_CONTEXT_MATERIALIZATION_BYTES,
                )?;
                require_materialization_file(
                    &files,
                    "data/opencode/auth.json",
                    provider,
                    profile_id,
                )?;
            }
        }
        files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        Ok(ProviderAccountMaterialization {
            profile: ProviderAccountReplicaMetadata {
                owner_user_id: stored.public.owner_user_id.clone(),
                provider: stored.public.provider.clone(),
                profile_id: stored.public.profile_id.clone(),
                label: stored.public.label.clone(),
                origin: stored.public.origin,
                is_default: stored.public.is_default,
            },
            files,
            generated_at_ms: crate::session::unix_epoch_ms(),
        })
    }

    pub(crate) fn materialize_replica(
        &self,
        owner_user_id: &str,
        materialization: &ProviderAccountMaterialization,
    ) -> Result<ProviderAccountProfile, DaemonError> {
        self.materialize_replica_internal(owner_user_id, materialization, None)
    }

    pub(crate) fn materialize_managed_context_replica(
        &self,
        owner_user_id: &str,
        context_id: &str,
        package_sha256: &str,
        materialization: &ProviderAccountMaterialization,
    ) -> Result<ManagedContextProviderAccountReceipt, DaemonError> {
        let materialization_sha256 = provider_account_materialization_sha256(materialization)?;
        let profile = self.materialize_replica_internal(
            owner_user_id,
            materialization,
            Some(ManagedContextReplicaIntent {
                context_id,
                package_sha256,
                materialization_sha256: &materialization_sha256,
            }),
        )?;
        Ok(ManagedContextProviderAccountReceipt {
            context_id: context_id.to_string(),
            package_sha256: package_sha256.to_string(),
            materialization_sha256,
            provider: profile.provider,
            profile_id: profile.profile_id,
        })
    }

    fn materialize_replica_internal(
        &self,
        owner_user_id: &str,
        materialization: &ProviderAccountMaterialization,
        managed_context: Option<ManagedContextReplicaIntent<'_>>,
    ) -> Result<ProviderAccountProfile, DaemonError> {
        if materialization.profile.owner_user_id != owner_user_id {
            return Err(registry_error(
                "materialize account profile",
                "materialization owner does not match the execution lease owner",
            ));
        }
        let provider = normalize_provider(&materialization.profile.provider)?;
        let profile_id = validate_profile_id(&materialization.profile.profile_id)?;
        if managed_context.is_some() {
            validate_managed_context_materialization_shape(provider, materialization)?;
        }
        let managed_root = self
            .path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("provider-accounts")
            .join(safe_path_component(owner_user_id))
            .join(provider)
            .join(profile_id);
        let managed_parent = managed_root.parent().ok_or_else(|| {
            registry_error(
                "materialize account profile",
                "managed account profile has no parent directory",
            )
        })?;
        fs::create_dir_all(managed_parent).map_err(registry_io("materialize account profile"))?;
        set_private_dir_permissions(managed_parent)?;
        let staging_root = managed_context
            .map(|intent| managed_context_staging_root(&managed_root, intent))
            .unwrap_or_else(|| unique_sibling_path(&managed_root, "stage"));
        if managed_context.is_some() {
            cleanup_managed_context_work_root(&staging_root, &self.path)?;
        }
        let locator = ProviderAccountLocator::managed(provider, &managed_root)?;
        let staging_locator = ProviderAccountLocator::managed(provider, &staging_root)?;
        let mut decoded_files = Vec::with_capacity(materialization.files.len());
        let mut decoded_bytes = 0usize;
        for file in &materialization.files {
            let contents = base64::engine::general_purpose::STANDARD
                .decode(&file.contents_base64)
                .map_err(|error| {
                    registry_error("materialize account profile", error.to_string())
                })?;
            decoded_bytes = decoded_bytes.saturating_add(contents.len());
            let maximum_bytes = if managed_context.is_some() {
                MAX_MANAGED_CONTEXT_MATERIALIZATION_BYTES
            } else {
                MAX_MATERIALIZATION_BYTES
            };
            if decoded_bytes > maximum_bytes {
                return Err(registry_error(
                    "materialize account profile",
                    "provider account materialization exceeds its safety limit",
                ));
            }
            let destination = materialization_destination(&staging_locator, &file.relative_path)?;
            if decoded_files
                .iter()
                .any(|(existing, _)| existing == &destination)
            {
                return Err(registry_error(
                    "materialize account profile",
                    "provider account materialization contains a duplicate path",
                ));
            }
            decoded_files.push((destination, contents));
        }

        let stage_result = (|| {
            create_private_roots(&staging_locator)?;
            set_private_dir_permissions(&staging_root)?;
            for (destination, contents) in &decoded_files {
                atomic_write_private(destination, contents)?;
            }
            if let ProviderAccountLocator::Codex { codex_home } = &staging_locator {
                enforce_codex_file_credentials(codex_home)?;
            }
            sync_private_tree(&staging_root)
        })();
        if let Err(error) = stage_result {
            let _ = fs::remove_dir_all(&staging_root);
            return Err(error);
        }

        let mut document = match self.write_document() {
            Ok(document) => document,
            Err(error) => {
                let _ = fs::remove_dir_all(&staging_root);
                return Err(error);
            }
        };
        let original_document = document.clone();
        let existing_profile = document.profiles.iter().find(|stored| {
            stored.public.owner_user_id == owner_user_id
                && stored.public.provider == provider
                && stored.public.profile_id == profile_id
        });
        if let (Some(intent), Some(stored)) = (managed_context, existing_profile) {
            if stored
                .managed_context_replica
                .as_ref()
                .is_some_and(|binding| binding.matches(intent))
            {
                let _ = fs::remove_dir_all(&staging_root);
                return Ok(stored.public.clone());
            }
        }
        let replace_existing_replica = match existing_profile {
            Some(stored)
                if managed_context.is_some()
                    && managed_context_default_can_be_replaced(stored, materialization) =>
            {
                true
            }
            Some(stored) if managed_context.is_some() => {
                let _ = fs::remove_dir_all(&staging_root);
                return Err(registry_error(
                    "materialize managed account profile",
                    if stored.managed_context_replica.is_some() {
                        "provider account is already bound to another managed context"
                    } else {
                        "refusing to replace an existing provider account profile"
                    },
                ));
            }
            Some(stored) if !stored.materialized_replica => {
                let _ = fs::remove_dir_all(&staging_root);
                return Err(registry_error(
                    "materialize account profile",
                    "refusing to replace an authoritative local account profile",
                ));
            }
            Some(stored) if stored.managed_context_replica.is_some() => {
                let _ = fs::remove_dir_all(&staging_root);
                return Err(registry_error(
                    "materialize account profile",
                    "refusing to replace a managed-context provider account",
                ));
            }
            Some(_) => true,
            None => false,
        };
        let managed_root_exists = path_entry_exists(&managed_root)?;
        let adopt_interrupted_managed_publication = managed_context.is_some()
            && managed_root_exists
            && (existing_profile.is_none() || replace_existing_replica)
            && managed_context_root_matches_materialization(&managed_root, &staging_root)?;
        if managed_root_exists
            && !replace_existing_replica
            && !adopt_interrupted_managed_publication
        {
            let _ = fs::remove_dir_all(&staging_root);
            return Err(registry_error(
                "materialize account profile",
                "refusing to replace unregistered provider account data",
            ));
        }
        if managed_root_exists
            && managed_context.is_some()
            && (existing_profile.is_none() || replace_existing_replica)
            && !adopt_interrupted_managed_publication
        {
            let _ = fs::remove_dir_all(&staging_root);
            return Err(registry_error(
                "materialize managed account profile",
                "interrupted provider credential publication does not match the retry",
            ));
        }

        let backup_root = (managed_root_exists && !adopt_interrupted_managed_publication)
            .then(|| unique_sibling_path(&managed_root, "backup"));
        if let Some(backup_root) = &backup_root {
            if let Err(error) = fs::rename(&managed_root, backup_root) {
                let _ = fs::remove_dir_all(&staging_root);
                return Err(registry_io("materialize account profile")(error));
            }
        }
        let publication_result = if adopt_interrupted_managed_publication {
            fs::remove_dir_all(&staging_root)
                .map_err(registry_io("recover managed account profile publication"))
                .and_then(|_| sync_private_tree(&managed_root))
                .and_then(|_| sync_directory(managed_parent))
        } else {
            fs::rename(&staging_root, &managed_root)
                .map_err(registry_io("materialize account profile"))
                .and_then(|_| sync_directory(managed_parent))
        };
        if let Err(error) = publication_result {
            let failed_root = unique_sibling_path(&managed_root, "failed");
            let _ = fs::rename(&managed_root, &failed_root);
            if let Some(backup_root) = &backup_root {
                let _ = fs::rename(backup_root, &managed_root);
            }
            let _ = fs::remove_dir_all(&failed_root);
            return Err(error);
        }

        let replaced_profile = managed_context.and_then(|_| {
            existing_profile
                .filter(|stored| !stored.materialized_replica)
                .map(|stored| ReplacedProviderAccountProfile {
                    public: stored.public.clone(),
                    locator: stored.locator.clone(),
                })
        });
        let previous_default_profile_id = managed_context
            .filter(|_| materialization.profile.is_default)
            .and_then(|_| {
                document
                    .profiles
                    .iter()
                    .find(|stored| {
                        stored.public.owner_user_id == owner_user_id
                            && stored.public.provider == provider
                            && stored.public.profile_id != profile_id
                            && stored.public.is_default
                    })
                    .map(|stored| stored.public.profile_id.clone())
            });
        if let Some(previous_default_profile_id) = &previous_default_profile_id {
            if let Some(previous_default) = document.profiles.iter_mut().find(|stored| {
                stored.public.owner_user_id == owner_user_id
                    && stored.public.provider == provider
                    && stored.public.profile_id == *previous_default_profile_id
            }) {
                previous_default.public.is_default = false;
            }
        }
        let managed_context_binding = managed_context.map(|intent| ManagedContextReplicaBinding {
            context_id: intent.context_id.to_string(),
            package_sha256: intent.package_sha256.to_string(),
            materialization_sha256: intent.materialization_sha256.to_string(),
            replaced_profile,
            previous_default_profile_id,
        });
        let result = if let Some(existing) = document.profiles.iter_mut().find(|stored| {
            stored.public.owner_user_id == owner_user_id
                && stored.public.provider == provider
                && stored.public.profile_id == profile_id
        }) {
            existing.public.label = materialization.profile.label.clone();
            existing.public.origin = materialization.profile.origin;
            existing.public.is_default = materialization.profile.is_default;
            existing.public.auth_state = ProviderAccountAuthState::Unknown;
            existing.public.identity_summary = None;
            existing.public.plan = None;
            existing.public.detected_provider_version = None;
            existing.public.last_validated_at_ms = None;
            existing.public.usage = ProviderAccountUsageSnapshot::unavailable(profile_id, provider);
            existing.locator = locator;
            existing.materialized_replica = true;
            existing.managed_context_replica = managed_context_binding;
            existing.public.clone()
        } else {
            let public = new_public_profile(
                owner_user_id,
                provider,
                profile_id,
                &materialization.profile.label,
                materialization.profile.origin,
                materialization.profile.is_default,
            );
            document.profiles.push(StoredProviderAccountProfile {
                public: public.clone(),
                locator,
                materialized_replica: true,
                managed_context_replica: managed_context_binding,
            });
            public
        };

        if let Err(error) = self.persist_locked(&document) {
            *document = original_document;
            if let Err(rollback_error) = self.persist_locked(&document) {
                return Err(registry_error(
                    "materialize account profile",
                    format!(
                        "{error}; additionally failed to restore the account profile registry: {rollback_error}"
                    ),
                ));
            }
            if managed_context.is_some() {
                return Err(error);
            }
            let failed_root = unique_sibling_path(&managed_root, "failed");
            let _ = fs::rename(&managed_root, &failed_root);
            if let Some(backup_root) = &backup_root {
                let _ = fs::rename(backup_root, &managed_root);
            }
            let _ = fs::remove_dir_all(&failed_root);
            let _ = sync_directory(managed_parent);
            return Err(error);
        }
        if let Some(backup_root) = &backup_root {
            let _ = fs::remove_dir_all(backup_root);
            let _ = sync_directory(managed_parent);
        }
        Ok(result)
    }

    pub(crate) fn rollback_managed_context_replica(
        &self,
        owner_user_id: &str,
        receipt: &ManagedContextProviderAccountReceipt,
    ) -> Result<(), DaemonError> {
        let provider = normalize_provider(&receipt.provider)?;
        let profile_id = validate_profile_id(&receipt.profile_id)?;
        let managed_root = self
            .path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("provider-accounts")
            .join(safe_path_component(owner_user_id))
            .join(provider)
            .join(profile_id);
        let rollback_root = managed_context_rollback_root(&managed_root, receipt);
        let mut document = self.write_document()?;
        let Some(index) = document.profiles.iter().position(|stored| {
            stored.public.owner_user_id == owner_user_id
                && stored.public.provider == provider
                && stored.public.profile_id == profile_id
        }) else {
            cleanup_managed_context_rollback_root(&rollback_root, &self.path)?;
            return Ok(());
        };
        let Some(binding) = document.profiles[index].managed_context_replica.clone() else {
            cleanup_managed_context_rollback_root(&rollback_root, &self.path)?;
            return Ok(());
        };
        if binding.context_id != receipt.context_id
            || binding.package_sha256 != receipt.package_sha256
            || binding.materialization_sha256 != receipt.materialization_sha256
        {
            return Err(registry_error(
                "roll back managed account profile",
                "provider account belongs to another managed context",
            ));
        }
        let expected_locator = ProviderAccountLocator::managed(provider, &managed_root)?;
        if document.profiles[index].locator != expected_locator {
            return Err(registry_error(
                "roll back managed account profile",
                "provider account root no longer matches the managed replica",
            ));
        }
        let managed_root_exists = path_entry_exists(&managed_root)?;
        let rollback_root_exists = path_entry_exists(&rollback_root)?;
        if managed_root_exists && rollback_root_exists {
            return Err(registry_error(
                "roll back managed account profile",
                "managed account profile and rollback state both exist",
            ));
        }
        if !managed_root_exists && !rollback_root_exists {
            return Err(registry_error(
                "roll back managed account profile",
                "managed account profile data is missing",
            ));
        }
        if rollback_root_exists {
            validate_managed_context_rollback_root(&rollback_root)?;
        } else {
            fs::rename(&managed_root, &rollback_root)
                .map_err(registry_io("roll back managed account profile"))?;
            sync_directory(managed_root.parent().unwrap_or_else(|| Path::new(".")))?;
        }
        let original_document = document.clone();
        if let Some(replaced) = binding.replaced_profile {
            document.profiles[index] = StoredProviderAccountProfile {
                public: replaced.public,
                locator: replaced.locator,
                materialized_replica: false,
                managed_context_replica: None,
            };
        } else {
            document.profiles.remove(index);
        }
        if let Some(previous_default_profile_id) = binding.previous_default_profile_id {
            if let Some(previous_default) = document.profiles.iter_mut().find(|stored| {
                stored.public.owner_user_id == owner_user_id
                    && stored.public.provider == provider
                    && stored.public.profile_id == previous_default_profile_id
            }) {
                previous_default.public.is_default = true;
            }
        }
        if let Err(error) = self.persist_locked(&document) {
            *document = original_document;
            if let Err(restore_error) = self.persist_locked(&document) {
                return Err(registry_error(
                    "roll back managed account profile",
                    format!(
                        "{error}; additionally failed to restore the account profile registry: {restore_error}"
                    ),
                ));
            }
            fs::rename(&rollback_root, &managed_root).map_err(registry_io(
                "restore managed account profile after registry failure",
            ))?;
            sync_directory(managed_root.parent().unwrap_or_else(|| Path::new(".")))?;
            return Err(error);
        }
        cleanup_managed_context_rollback_root(&rollback_root, &self.path)?;
        if let Some(parent) = managed_root.parent() {
            sync_directory(parent)?;
        }
        Ok(())
    }

    fn read_document(
        &self,
    ) -> Result<std::sync::RwLockReadGuard<'_, RegistryDocument>, DaemonError> {
        self.document
            .read()
            .map_err(|error| registry_error("read account profile registry", error.to_string()))
    }

    fn write_document(
        &self,
    ) -> Result<std::sync::RwLockWriteGuard<'_, RegistryDocument>, DaemonError> {
        self.document
            .write()
            .map_err(|error| registry_error("write account profile registry", error.to_string()))
    }

    fn persist_locked(&self, document: &RegistryDocument) -> Result<(), DaemonError> {
        let bytes = serde_json::to_vec_pretty(document)
            .map_err(|error| registry_error("write account profile registry", error.to_string()))?;
        atomic_write_private_internal(&self.path, &bytes, true)
    }
}

#[derive(Clone, Copy)]
struct ManagedContextReplicaIntent<'a> {
    context_id: &'a str,
    package_sha256: &'a str,
    materialization_sha256: &'a str,
}

impl ManagedContextReplicaBinding {
    fn matches(&self, intent: ManagedContextReplicaIntent<'_>) -> bool {
        self.context_id == intent.context_id
            && self.package_sha256 == intent.package_sha256
            && self.materialization_sha256 == intent.materialization_sha256
    }
}

fn managed_context_default_can_be_replaced(
    stored: &StoredProviderAccountProfile,
    materialization: &ProviderAccountMaterialization,
) -> bool {
    !stored.materialized_replica
        && stored.managed_context_replica.is_none()
        && stored.public.origin == ProviderAccountProfileOrigin::Default
        && stored.public.profile_id == "default"
        && stored.public.is_default
        && materialization.profile.profile_id == "default"
        && materialization.profile.is_default
}

fn validate_managed_context_materialization_shape(
    provider: &str,
    materialization: &ProviderAccountMaterialization,
) -> Result<(), DaemonError> {
    let (required, allowed): (&str, &[&str]) = match provider {
        "codex" => ("auth.json", &["auth.json"]),
        "claude" => (".credentials.json", &[".credentials.json"]),
        "opencode" => ("data/opencode/auth.json", &["data/opencode/auth.json"]),
        _ => return Err(unsupported_provider(provider)),
    };
    if !materialization
        .files
        .iter()
        .any(|file| file.relative_path == required)
        || materialization
            .files
            .iter()
            .any(|file| !allowed.contains(&file.relative_path.as_str()))
    {
        return Err(registry_error(
            "materialize managed account profile",
            "provider account materialization does not match the managed-context credential allowlist",
        ));
    }
    Ok(())
}

pub(crate) fn provider_account_materialization_sha256(
    materialization: &ProviderAccountMaterialization,
) -> Result<String, DaemonError> {
    let bytes = serde_json::to_vec(materialization).map_err(|error| {
        registry_error("hash account profile materialization", error.to_string())
    })?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn path_entry_exists(path: &Path) -> Result<bool, DaemonError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(registry_io("inspect account profile path")(error)),
    }
}

fn managed_context_rollback_root(
    managed_root: &Path,
    receipt: &ManagedContextProviderAccountReceipt,
) -> PathBuf {
    managed_context_sibling_root(
        managed_root,
        "rollback",
        &receipt.context_id,
        &receipt.package_sha256,
        &receipt.materialization_sha256,
    )
}

fn managed_context_staging_root(
    managed_root: &Path,
    intent: ManagedContextReplicaIntent<'_>,
) -> PathBuf {
    managed_context_sibling_root(
        managed_root,
        "stage",
        intent.context_id,
        intent.package_sha256,
        intent.materialization_sha256,
    )
}

fn managed_context_sibling_root(
    managed_root: &Path,
    purpose: &str,
    context_id: &str,
    package_sha256: &str,
    materialization_sha256: &str,
) -> PathBuf {
    let mut digest = Sha256::new();
    for value in [
        context_id.as_bytes(),
        package_sha256.as_bytes(),
        materialization_sha256.as_bytes(),
    ] {
        digest.update((value.len() as u64).to_be_bytes());
        digest.update(value);
    }
    let suffix = format!("{:x}", digest.finalize());
    let parent = managed_root.parent().unwrap_or_else(|| Path::new("."));
    let name = managed_root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("profile");
    parent.join(format!(".{name}.managed-context-{purpose}-{suffix}"))
}

fn managed_context_root_matches_materialization(
    managed_root: &Path,
    staging_root: &Path,
) -> Result<bool, DaemonError> {
    validate_managed_context_rollback_root(managed_root)?;
    let expected = collect_managed_context_private_tree(staging_root)?;
    let mut seen = vec![false; expected.len()];
    let mut pending = vec![managed_root.to_path_buf()];
    let mut entry_count = 0usize;
    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory)
            .map_err(registry_io("recover managed account profile publication"))?;
        for entry in entries {
            let entry =
                entry.map_err(registry_io("recover managed account profile publication"))?;
            entry_count = entry_count.saturating_add(1);
            if entry_count > 64 {
                return Err(registry_error(
                    "recover managed account profile publication",
                    "interrupted provider credential publication has too many entries",
                ));
            }
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(registry_io("recover managed account profile publication"))?;
            if metadata.file_type().is_symlink() {
                return Err(registry_error(
                    "recover managed account profile publication",
                    "interrupted provider credential publication contains a symbolic link",
                ));
            }
            if metadata.is_dir() {
                pending.push(path);
                continue;
            }
            if !metadata.is_file() {
                return Ok(false);
            }
            let relative = path.strip_prefix(managed_root).map_err(|error| {
                registry_error(
                    "recover managed account profile publication",
                    error.to_string(),
                )
            })?;
            let Some((index, (_, expected_contents))) = expected
                .iter()
                .enumerate()
                .find(|(_, (expected_path, _))| expected_path == relative)
            else {
                return Ok(false);
            };
            let actual = read_bounded_regular_file_no_follow(
                &path,
                expected_contents.len(),
                "managed-context credential",
            )?
            .ok_or_else(|| {
                registry_error(
                    "recover managed account profile publication",
                    "interrupted provider credential publication disappeared",
                )
            })?;
            if actual.as_slice() != *expected_contents || seen[index] {
                return Ok(false);
            }
            seen[index] = true;
        }
    }
    Ok(seen.into_iter().all(|seen| seen))
}

fn collect_managed_context_private_tree(
    root: &Path,
) -> Result<Vec<(PathBuf, Vec<u8>)>, DaemonError> {
    validate_managed_context_rollback_root(root)?;
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    let mut entry_count = 0usize;
    let mut total_bytes = 0usize;
    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory)
            .map_err(registry_io("recover managed account profile publication"))?;
        for entry in entries {
            let entry =
                entry.map_err(registry_io("recover managed account profile publication"))?;
            entry_count = entry_count.saturating_add(1);
            if entry_count > 64 {
                return Err(registry_error(
                    "recover managed account profile publication",
                    "interrupted provider credential publication has too many entries",
                ));
            }
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(registry_io("recover managed account profile publication"))?;
            if metadata.file_type().is_symlink() {
                return Err(registry_error(
                    "recover managed account profile publication",
                    "interrupted provider credential publication contains a symbolic link",
                ));
            }
            if metadata.is_dir() {
                pending.push(path);
                continue;
            }
            if !metadata.is_file() {
                return Err(registry_error(
                    "recover managed account profile publication",
                    "interrupted provider credential publication contains an unsupported entry",
                ));
            }
            let remaining = MAX_MATERIALIZATION_BYTES.saturating_sub(total_bytes);
            let contents = read_bounded_regular_file_no_follow(
                &path,
                remaining,
                "managed-context credential",
            )?
            .ok_or_else(|| {
                registry_error(
                    "recover managed account profile publication",
                    "interrupted provider credential publication disappeared",
                )
            })?;
            total_bytes = total_bytes.saturating_add(contents.len());
            if total_bytes > MAX_MATERIALIZATION_BYTES {
                return Err(registry_error(
                    "recover managed account profile publication",
                    "interrupted provider credential publication exceeds its safety limit",
                ));
            }
            let relative = path.strip_prefix(root).map_err(|error| {
                registry_error(
                    "recover managed account profile publication",
                    error.to_string(),
                )
            })?;
            files.push((relative.to_path_buf(), contents));
        }
    }
    Ok(files)
}

fn validate_managed_context_rollback_root(root: &Path) -> Result<(), DaemonError> {
    let metadata = fs::symlink_metadata(root)
        .map_err(registry_io("inspect managed account profile rollback"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(registry_error(
            "inspect managed account profile rollback",
            "managed account profile rollback state must be a regular directory",
        ));
    }
    Ok(())
}

fn cleanup_managed_context_rollback_root(
    rollback_root: &Path,
    registry_path: &Path,
) -> Result<(), DaemonError> {
    if !path_entry_exists(rollback_root)? {
        return Ok(());
    }
    validate_managed_context_rollback_root(rollback_root)?;
    #[cfg(test)]
    if FAIL_MANAGED_CONTEXT_ROLLBACK_CLEANUP_ONCE.with(|fail| fail.replace(false)) {
        return Err(registry_error(
            "delete managed account profile rollback",
            "injected cleanup failure",
        ));
    }
    remove_managed_root(rollback_root, registry_path)?;
    if let Some(parent) = rollback_root.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}

fn cleanup_managed_context_work_root(root: &Path, registry_path: &Path) -> Result<(), DaemonError> {
    if !path_entry_exists(root)? {
        return Ok(());
    }
    validate_managed_context_rollback_root(root)?;
    remove_managed_root(root, registry_path)?;
    if let Some(parent) = root.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}

fn new_public_profile(
    owner_user_id: &str,
    provider: &str,
    profile_id: &str,
    label: &str,
    origin: ProviderAccountProfileOrigin,
    is_default: bool,
) -> ProviderAccountProfile {
    ProviderAccountProfile {
        owner_user_id: owner_user_id.to_string(),
        provider: provider.to_string(),
        profile_id: profile_id.to_string(),
        label: label.to_string(),
        origin,
        is_default,
        auth_state: ProviderAccountAuthState::Unknown,
        identity_summary: None,
        plan: None,
        detected_provider_version: None,
        last_validated_at_ms: None,
        usage: ProviderAccountUsageSnapshot::unavailable(profile_id, provider),
        materializations: Vec::new(),
    }
}

fn mark_profile_materializations_stale(profile: &mut ProviderAccountProfile) {
    let now_ms = crate::session::unix_epoch_ms();
    for materialization in &mut profile.materializations {
        materialization.state = ProviderAccountMaterializationState::Stale;
        materialization.observed_at_ms = now_ms;
        materialization.last_error = None;
    }
}

fn normalize_provider(provider: &str) -> Result<&'static str, DaemonError> {
    crate::provider::canonical_provider_family(provider)
        .filter(|provider| SUPPORTED_PROVIDERS.contains(provider))
        .ok_or_else(|| unsupported_provider(provider))
}

fn validate_label(label: &str) -> Result<&str, DaemonError> {
    let label = label.trim();
    if label.is_empty() || label.chars().count() > 80 {
        return Err(registry_error(
            "validate account profile",
            "label must contain between 1 and 80 characters",
        ));
    }
    Ok(label)
}

fn ensure_unique_label(
    document: &RegistryDocument,
    owner_user_id: &str,
    provider: &str,
    label: &str,
) -> Result<(), DaemonError> {
    ensure_unique_label_except(document, owner_user_id, provider, label, "")
}

fn ensure_unique_label_except(
    document: &RegistryDocument,
    owner_user_id: &str,
    provider: &str,
    label: &str,
    excluded_profile_id: &str,
) -> Result<(), DaemonError> {
    if document.profiles.iter().any(|profile| {
        profile.public.owner_user_id == owner_user_id
            && profile.public.provider == provider
            && profile.public.profile_id != excluded_profile_id
            && profile.public.label.eq_ignore_ascii_case(label)
    }) {
        return Err(registry_error(
            "validate account profile",
            format!("an account profile labeled `{label}` already exists for {provider}"),
        ));
    }
    Ok(())
}

fn unique_profile_id(
    document: &RegistryDocument,
    owner_user_id: &str,
    provider: &str,
    label: &str,
) -> String {
    let slug = safe_path_component(label).to_ascii_lowercase();
    loop {
        let suffix: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(8)
            .map(char::from)
            .map(|value| value.to_ascii_lowercase())
            .collect();
        let candidate = format!("{slug}-{suffix}");
        if !document.profiles.iter().any(|profile| {
            profile.public.owner_user_id == owner_user_id
                && profile.public.provider == provider
                && profile.public.profile_id == candidate
        }) {
            return candidate;
        }
    }
}

fn safe_path_component(value: &str) -> String {
    let mut result = String::new();
    let mut separator = false;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            result.push(character);
            separator = false;
        } else if !separator && !result.is_empty() {
            result.push('-');
            separator = true;
        }
    }
    while result.ends_with('-') {
        result.pop();
    }
    if result.is_empty() {
        "profile".to_string()
    } else {
        result
    }
}

pub(crate) fn account_owner_path_component(owner_user_id: &str) -> String {
    safe_path_component(owner_user_id)
}

fn resolve_stored_profile<'a>(
    document: &'a RegistryDocument,
    owner_user_id: &str,
    provider: &str,
    profile_id: &str,
) -> Result<&'a StoredProviderAccountProfile, DaemonError> {
    let profile_id = profile_id.trim();
    let profile = if profile_id == "default" {
        document.profiles.iter().find(|profile| {
            profile.public.owner_user_id == owner_user_id
                && profile.public.provider == provider
                && profile.public.is_default
        })
    } else {
        document.profiles.iter().find(|profile| {
            profile.public.owner_user_id == owner_user_id
                && profile.public.provider == provider
                && profile.public.profile_id == profile_id
        })
    };
    profile.ok_or_else(|| {
        registry_error(
            "resolve account profile",
            format!("account profile `{profile_id}` is not registered for {provider}"),
        )
    })
}

fn resolve_stored_profile_mut<'a>(
    document: &'a mut RegistryDocument,
    owner_user_id: &str,
    provider: &str,
    profile_id: &str,
) -> Result<&'a mut StoredProviderAccountProfile, DaemonError> {
    let index = resolved_profile_index(document, owner_user_id, provider, profile_id)?;
    Ok(&mut document.profiles[index])
}

fn resolved_profile_index(
    document: &RegistryDocument,
    owner_user_id: &str,
    provider: &str,
    profile_id: &str,
) -> Result<usize, DaemonError> {
    let resolved = resolve_stored_profile(document, owner_user_id, provider, profile_id)?;
    document
        .profiles
        .iter()
        .position(|profile| std::ptr::eq(profile, resolved))
        .ok_or_else(|| registry_error("resolve account profile", "profile index disappeared"))
}

fn effective_xdg(name: &str, fallback: PathBuf) -> PathBuf {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or(fallback)
}

fn create_private_roots(locator: &ProviderAccountLocator) -> Result<(), DaemonError> {
    for root in locator.roots() {
        fs::create_dir_all(root).map_err(registry_io("create managed account profile"))?;
        set_private_dir_permissions(root)?;
    }
    Ok(())
}

fn validate_profile_id(profile_id: &str) -> Result<&str, DaemonError> {
    let profile_id = profile_id.trim();
    if profile_id.is_empty()
        || profile_id != safe_path_component(profile_id)
        || profile_id.chars().count() > 120
    {
        return Err(registry_error(
            "validate account profile",
            "profile id is not a safe stable identifier",
        ));
    }
    Ok(profile_id)
}

fn collect_optional_file(
    root: &Path,
    source_relative_path: &str,
    transfer_relative_path: &str,
    files: &mut Vec<ProviderAccountMaterializationFile>,
) -> Result<(), DaemonError> {
    collect_optional_file_bounded(
        root,
        source_relative_path,
        transfer_relative_path,
        files,
        MAX_MATERIALIZATION_BYTES,
    )
}

fn collect_optional_file_bounded(
    root: &Path,
    source_relative_path: &str,
    transfer_relative_path: &str,
    files: &mut Vec<ProviderAccountMaterializationFile>,
    maximum_bytes: usize,
) -> Result<(), DaemonError> {
    let source = root.join(source_relative_path);
    let existing_bytes = materialization_decoded_bytes(files);
    let remaining_bytes = maximum_bytes.saturating_sub(existing_bytes);
    let Some(contents) =
        read_bounded_regular_file_no_follow(&source, remaining_bytes, source_relative_path)?
    else {
        return Ok(());
    };
    if existing_bytes.saturating_add(contents.len()) > maximum_bytes {
        return Err(registry_error(
            "export account profile",
            format!(
                "provider account materialization exceeds the {} MiB safety limit",
                maximum_bytes / (1024 * 1024)
            ),
        ));
    }
    files.push(ProviderAccountMaterializationFile {
        relative_path: transfer_relative_path.to_string(),
        contents_base64: base64::engine::general_purpose::STANDARD.encode(contents),
    });
    Ok(())
}

fn materialization_decoded_bytes(files: &[ProviderAccountMaterializationFile]) -> usize {
    files
        .iter()
        .map(|file| file.contents_base64.len().saturating_mul(3) / 4)
        .sum()
}

fn read_bounded_regular_file_no_follow(
    path: &Path,
    maximum_bytes: usize,
    source_relative_path: &str,
) -> Result<Option<Vec<u8>>, DaemonError> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(registry_error("export account profile", error.to_string())),
    };
    let metadata = file
        .metadata()
        .map_err(registry_io("export account profile"))?;
    if !metadata.is_file() {
        return Err(registry_error(
            "export account profile",
            format!("credential file `{source_relative_path}` must be a regular file"),
        ));
    }
    if metadata.len() > maximum_bytes as u64 {
        return Err(registry_error(
            "export account profile",
            "provider account materialization exceeds its safety limit",
        ));
    }
    let mut contents = Vec::with_capacity(metadata.len() as usize);
    file.take(maximum_bytes.saturating_add(1) as u64)
        .read_to_end(&mut contents)
        .map_err(registry_io("export account profile"))?;
    if contents.len() > maximum_bytes {
        return Err(registry_error(
            "export account profile",
            "provider account materialization exceeds its safety limit",
        ));
    }
    Ok(Some(contents))
}

fn validate_materialization_root(root: &Path) -> Result<(), DaemonError> {
    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(registry_error("export account profile", error.to_string())),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(registry_error(
            "export account profile",
            "provider account credential root must be a regular directory",
        ));
    }
    Ok(())
}

fn require_materialization_file(
    files: &[ProviderAccountMaterializationFile],
    relative_path: &str,
    provider: &str,
    profile_id: &str,
) -> Result<(), DaemonError> {
    if materialization_decoded_bytes(files) > MAX_MANAGED_CONTEXT_MATERIALIZATION_BYTES {
        return Err(registry_error(
            "export managed account profile",
            "provider account materialization exceeds the 16 MiB managed-context limit",
        ));
    }
    if files.iter().any(|file| file.relative_path == relative_path) {
        return Ok(());
    }
    Err(registry_error(
        "export managed account profile",
        format!("{provider} account profile `{profile_id}` has no transferable credentials"),
    ))
}

fn collect_optional_tree(
    root: &Path,
    transfer_prefix: &str,
    files: &mut Vec<ProviderAccountMaterializationFile>,
) -> Result<(), DaemonError> {
    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(registry_error("export account profile", error.to_string())),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(registry_error(
            "export account profile",
            "provider account materialization root must be a regular directory",
        ));
    }
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let mut entries = fs::read_dir(&directory)
            .map_err(registry_io("export account profile"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(registry_io("export account profile"))?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let metadata =
                fs::symlink_metadata(&path).map_err(registry_io("export account profile"))?;
            if metadata.file_type().is_symlink() {
                return Err(registry_error(
                    "export account profile",
                    "symlinks are not allowed in transferred provider profile data",
                ));
            }
            if metadata.is_dir() {
                pending.push(path);
                continue;
            }
            if !metadata.is_file() {
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .map_err(|error| registry_error("export account profile", error.to_string()))?;
            let transfer_relative = Path::new(transfer_prefix).join(relative);
            collect_optional_file(
                root,
                &relative.to_string_lossy(),
                &transfer_relative.to_string_lossy(),
                files,
            )?;
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn collect_default_claude_keychain_credentials(
    files: &mut Vec<ProviderAccountMaterializationFile>,
) -> Result<(), DaemonError> {
    let output = std::process::Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            "Claude Code-credentials",
            "-w",
        ])
        .output()
        .map_err(registry_io("export Claude Keychain credentials"))?;
    if !output.status.success() || output.stdout.is_empty() {
        return Ok(());
    }
    if output.stdout.len() > MAX_MATERIALIZATION_BYTES {
        return Err(registry_error(
            "export Claude Keychain credentials",
            "Claude Keychain credential exceeds the materialization safety limit",
        ));
    }
    files.push(ProviderAccountMaterializationFile {
        relative_path: ".credentials.json".to_string(),
        contents_base64: base64::engine::general_purpose::STANDARD.encode(output.stdout),
    });
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn collect_default_claude_keychain_credentials(
    _files: &mut Vec<ProviderAccountMaterializationFile>,
) -> Result<(), DaemonError> {
    Ok(())
}

fn materialization_destination(
    locator: &ProviderAccountLocator,
    relative_path: &str,
) -> Result<PathBuf, DaemonError> {
    let relative = Path::new(relative_path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(registry_error(
            "materialize account profile",
            "provider account materialization contains an unsafe relative path",
        ));
    }
    match locator {
        ProviderAccountLocator::Codex { codex_home } => Ok(codex_home.join(relative)),
        ProviderAccountLocator::Claude { claude_config_dir } => {
            Ok(claude_config_dir.join(relative))
        }
        ProviderAccountLocator::Opencode {
            xdg_data_home,
            xdg_config_home,
            xdg_state_home,
            opencode_config_dir,
            ..
        } => {
            let mut components = relative.components();
            let root = match components
                .next()
                .and_then(|component| component.as_os_str().to_str())
            {
                Some("data") => xdg_data_home,
                Some("config") => xdg_config_home,
                Some("state") => xdg_state_home,
                Some("opencode-config") => opencode_config_dir,
                _ => {
                    return Err(registry_error(
                        "materialize account profile",
                        "OpenCode materialization path has an unknown root",
                    ))
                }
            };
            Ok(root.join(components.as_path()))
        }
    }
}

fn enforce_codex_file_credentials(codex_home: &Path) -> Result<(), DaemonError> {
    let config_path = codex_home.join("config.toml");
    let existing = match fs::read_to_string(&config_path) {
        Ok(existing) => existing,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(registry_error(
                "configure Codex account profile",
                error.to_string(),
            ))
        }
    };
    let mut replaced = false;
    let mut lines = existing
        .lines()
        .map(|line| {
            if line.trim_start().starts_with("cli_auth_credentials_store") {
                replaced = true;
                "cli_auth_credentials_store = \"file\"".to_string()
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>();
    if !replaced {
        lines.insert(0, "cli_auth_credentials_store = \"file\"".to_string());
    }
    let mut config = lines.join("\n");
    config.push('\n');
    atomic_write_private(&config_path, config.as_bytes())
}

fn validate_linked_root(path: &Path) -> Result<PathBuf, DaemonError> {
    let canonical = fs::canonicalize(path).map_err(registry_io("link account profile"))?;
    let metadata = fs::metadata(&canonical).map_err(registry_io("link account profile"))?;
    if !metadata.is_dir() {
        return Err(registry_error(
            "link account profile",
            "linked provider root must be a directory",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != unsafe { libc::geteuid() } {
            return Err(registry_error(
                "link account profile",
                "linked provider root must be owned by the current user",
            ));
        }
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(registry_error(
                "link account profile",
                "linked provider root must not be accessible by group or other users",
            ));
        }
    }
    if canonical
        .ancestors()
        .any(|ancestor| ancestor.join(".git").exists())
    {
        return Err(registry_error(
            "link account profile",
            "repositories and workspaces cannot be provider credential roots",
        ));
    }
    Ok(canonical)
}

fn unique_sibling_path(path: &Path, purpose: &str) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("profile");
    loop {
        let suffix: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(12)
            .map(char::from)
            .collect();
        let candidate = parent.join(format!(".{name}.{purpose}-{suffix}"));
        if !candidate.exists() {
            return candidate;
        }
    }
}

fn sync_private_tree(root: &Path) -> Result<(), DaemonError> {
    let mut pending = vec![root.to_path_buf()];
    let mut directories = Vec::new();
    while let Some(directory) = pending.pop() {
        directories.push(directory.clone());
        for entry in
            fs::read_dir(&directory).map_err(registry_io("sync account profile replica"))?
        {
            let entry = entry.map_err(registry_io("sync account profile replica"))?;
            let metadata = entry
                .file_type()
                .map_err(registry_io("sync account profile replica"))?;
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.is_file() {
                fs::File::open(entry.path())
                    .and_then(|file| file.sync_all())
                    .map_err(registry_io("sync account profile replica"))?;
            } else {
                return Err(registry_error(
                    "sync account profile replica",
                    "provider account replica contains an unsupported filesystem entry",
                ));
            }
        }
    }
    directories.sort_by_key(|directory| std::cmp::Reverse(directory.components().count()));
    for directory in directories {
        sync_directory(&directory)?;
    }
    Ok(())
}

fn sync_directory(path: &Path) -> Result<(), DaemonError> {
    fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(registry_io("sync account profile directory"))
}

fn remove_managed_root(root: &Path, registry_path: &Path) -> Result<(), DaemonError> {
    let managed_base = registry_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("provider-accounts");
    let canonical_base =
        fs::canonicalize(&managed_base).map_err(registry_io("delete managed account profile"))?;
    let canonical_root =
        fs::canonicalize(root).map_err(registry_io("delete managed account profile"))?;
    if !canonical_root.starts_with(&canonical_base) || canonical_root == canonical_base {
        return Err(registry_error(
            "delete managed account profile",
            "refusing to delete a path outside the managed account root",
        ));
    }
    fs::remove_dir_all(canonical_root).map_err(registry_io("delete managed account profile"))
}

fn atomic_write_private(path: &Path, bytes: &[u8]) -> Result<(), DaemonError> {
    atomic_write_private_internal(path, bytes, false)
}

fn atomic_write_private_internal(
    path: &Path,
    bytes: &[u8],
    _account_registry_write: bool,
) -> Result<(), DaemonError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(registry_io("write account profile registry"))?;
    set_private_dir_permissions(parent)?;
    let suffix: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(8)
        .map(char::from)
        .collect();
    let temporary = parent.join(format!(".account-profiles-{suffix}.tmp"));
    let mut published = false;
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(registry_io("write account profile registry"))?;
        set_private_file_permissions(&temporary)?;
        file.write_all(bytes)
            .map_err(registry_io("write account profile registry"))?;
        file.sync_all()
            .map_err(registry_io("sync account profile registry"))?;
        drop(file);
        fs::rename(&temporary, path).map_err(registry_io("write account profile registry"))?;
        published = true;
        #[cfg(test)]
        if _account_registry_write
            && FAIL_ACCOUNT_PROFILE_REGISTRY_PARENT_SYNC_ONCE.with(|fail| fail.replace(false))
        {
            return Err(registry_error(
                "sync account profile registry directory",
                "injected post-rename sync failure",
            ));
        }
        sync_directory(parent)
    })();
    if result.is_err() && !published {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(unix)]
fn set_private_dir_permissions(path: &Path) -> Result<(), DaemonError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(registry_io("secure account profile directory"))
}

#[cfg(not(unix))]
fn set_private_dir_permissions(_path: &Path) -> Result<(), DaemonError> {
    Ok(())
}

#[cfg(unix)]
fn set_private_file_permissions(path: &Path) -> Result<(), DaemonError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(registry_io("secure account profile file"))
}

#[cfg(not(unix))]
fn set_private_file_permissions(_path: &Path) -> Result<(), DaemonError> {
    Ok(())
}

fn registry_io(operation: &'static str) -> impl FnOnce(std::io::Error) -> DaemonError {
    move |error| registry_error(operation, error.to_string())
}

fn registry_error(operation: &'static str, message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation,
        message: message.into(),
    }
}

fn unsupported_provider(provider: &str) -> DaemonError {
    registry_error(
        "validate account profile",
        format!("provider `{provider}` does not support managed account profiles"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_owner_aliases_local_accounts_without_aliasing_collaborators() {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            user_id: "cloud-owner".to_string(),
            ..Default::default()
        });

        assert_eq!(
            provider_account_authority_owner_user_id(&config, "cloud-owner"),
            crate::session::DEFAULT_LOCAL_USER_ID
        );
        assert_eq!(
            provider_account_authority_owner_user_id(&config, "collaborator"),
            "collaborator"
        );
        assert_eq!(
            provider_account_authority_owner_user_id(
                &config,
                crate::session::DEFAULT_LOCAL_USER_ID,
            ),
            crate::session::DEFAULT_LOCAL_USER_ID
        );
    }

    fn fixture() -> (PathBuf, ProviderAccountProfileRegistry) {
        let root = std::env::temp_dir().join(format!(
            "chariox-account-profile-test-{}-{}",
            std::process::id(),
            rand::thread_rng().gen::<u64>()
        ));
        fs::create_dir_all(&root).unwrap();
        let registry = ProviderAccountProfileRegistry::open(root.join("accounts.json")).unwrap();
        (root, registry)
    }

    #[test]
    fn migrates_one_effective_default_per_provider_without_scanning() {
        let (root, registry) = fixture();
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();

        let first = registry
            .migrate_effective_defaults("owner-a", &home)
            .unwrap();
        let second = registry
            .migrate_effective_defaults("owner-a", &home)
            .unwrap();

        assert_eq!(first.len(), 3);
        assert_eq!(second.len(), 3);
        assert!(first.iter().all(|profile| profile.profile_id == "default"));
        assert!(first.iter().all(|profile| profile.is_default));
        for provider in ["codex", "claude", "opencode"] {
            let environment = registry
                .resolve_environment("owner-a", provider, "default")
                .unwrap();
            for path in environment.values() {
                assert!(
                    Path::new(path).is_dir(),
                    "{provider} root {path} should exist"
                );
            }
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn resolving_an_existing_default_repairs_missing_profile_roots() {
        let (root, registry) = fixture();
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        registry
            .migrate_effective_defaults("owner-a", &home)
            .unwrap();
        let environment = registry
            .resolve_environment("owner-a", "opencode", "default")
            .unwrap();
        fs::remove_dir_all(&environment["XDG_STATE_HOME"]).unwrap();
        fs::remove_dir_all(&environment["OPENCODE_CONFIG_DIR"]).unwrap();

        let repaired = registry
            .resolve_environment("owner-a", "opencode", "default")
            .unwrap();
        assert!(Path::new(&repaired["XDG_STATE_HOME"]).is_dir());
        assert!(Path::new(&repaired["OPENCODE_CONFIG_DIR"]).is_dir());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn managed_profiles_are_isolated_and_codex_is_file_backed() {
        let (root, registry) = fixture();
        registry
            .migrate_effective_defaults("owner-a", &root.join("home"))
            .unwrap();

        let work = registry.create_managed("owner-a", "codex", "Work").unwrap();
        let personal = registry
            .create_managed("owner-a", "codex", "Personal")
            .unwrap();
        let work_env = registry
            .resolve_environment("owner-a", "codex", &work.profile_id)
            .unwrap();
        let personal_env = registry
            .resolve_environment("owner-a", "codex", &personal.profile_id)
            .unwrap();

        assert_ne!(work_env["CODEX_HOME"], personal_env["CODEX_HOME"]);
        let config =
            fs::read_to_string(Path::new(&work_env["CODEX_HOME"]).join("config.toml")).unwrap();
        assert_eq!(config, "cli_auth_credentials_store = \"file\"\n");
        let projected = serde_json::to_value(work).unwrap();
        assert!(projected.get("locator").is_none());
        assert!(!projected.to_string().contains("CODEX_HOME"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn codex_file_mode_preserves_existing_configuration() {
        let (root, registry) = fixture();
        let profile = registry.create_managed("owner-a", "codex", "Work").unwrap();
        let environment = registry
            .resolve_environment("owner-a", "codex", &profile.profile_id)
            .unwrap();
        let codex_home = Path::new(&environment["CODEX_HOME"]);
        fs::write(
            codex_home.join("config.toml"),
            "model = \"gpt-5.5\"\ncli_auth_credentials_store = \"keyring\"\n",
        )
        .unwrap();

        enforce_codex_file_credentials(codex_home).unwrap();

        let config = fs::read_to_string(codex_home.join("config.toml")).unwrap();
        assert!(config.contains("model = \"gpt-5.5\""));
        assert!(config.contains("cli_auth_credentials_store = \"file\""));
        assert!(!config.contains("keyring"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn account_materialization_is_profile_specific_and_secret_debug_is_redacted() {
        let (source_root, source) = fixture();
        let profile = source.create_managed("owner-a", "codex", "Work").unwrap();
        let source_environment = source
            .resolve_environment("owner-a", "codex", &profile.profile_id)
            .unwrap();
        fs::write(
            Path::new(&source_environment["CODEX_HOME"]).join("auth.json"),
            br#"{"token":"never-log-this"}"#,
        )
        .unwrap();
        let materialization = source
            .export_materialization("owner-a", "codex", &profile.profile_id)
            .unwrap();
        assert!(!format!("{materialization:?}").contains("never-log-this"));

        let (target_root, target) = fixture();
        let materialized = target
            .materialize_replica("owner-a", &materialization)
            .unwrap();
        let target_environment = target
            .resolve_environment("owner-a", "codex", &materialized.profile_id)
            .unwrap();
        assert_ne!(
            source_environment["CODEX_HOME"],
            target_environment["CODEX_HOME"]
        );
        assert_eq!(
            fs::read_to_string(Path::new(&target_environment["CODEX_HOME"]).join("auth.json"))
                .unwrap(),
            r#"{"token":"never-log-this"}"#
        );
        assert!(target
            .materialize_replica("owner-b", &materialization)
            .is_err());
        let _ = fs::remove_dir_all(source_root);
        let _ = fs::remove_dir_all(target_root);
    }

    #[test]
    fn managed_context_replica_is_idempotent_and_restores_the_target_default() {
        let (source_root, source) = fixture();
        let source_profile = source
            .create_managed("owner-a", "codex", "Source default")
            .unwrap();
        source
            .set_default("owner-a", "codex", &source_profile.profile_id)
            .unwrap();
        let source_environment = source
            .resolve_environment("owner-a", "codex", "default")
            .unwrap();
        fs::write(
            Path::new(&source_environment["CODEX_HOME"]).join("auth.json"),
            br#"{"token":"source"}"#,
        )
        .unwrap();
        let materialization = source
            .export_managed_context_materialization("owner-a", "codex", "default")
            .unwrap();

        let (target_root, target) = fixture();
        let target_home = target_root.join("home");
        target
            .migrate_effective_defaults("owner-a", &target_home)
            .unwrap();
        let receipt = target
            .materialize_managed_context_replica(
                "owner-a",
                "context-a",
                &"a".repeat(64),
                &materialization,
            )
            .unwrap();
        let imported_environment = target
            .resolve_environment("owner-a", "codex", "default")
            .unwrap();
        let imported_auth = Path::new(&imported_environment["CODEX_HOME"]).join("auth.json");
        assert_eq!(
            fs::read_to_string(&imported_auth).unwrap(),
            r#"{"token":"source"}"#
        );

        fs::write(&imported_auth, br#"{"token":"rotated-on-target"}"#).unwrap();
        target
            .materialize_managed_context_replica(
                "owner-a",
                "context-a",
                &"a".repeat(64),
                &materialization,
            )
            .unwrap();
        assert_eq!(
            fs::read_to_string(&imported_auth).unwrap(),
            r#"{"token":"rotated-on-target"}"#
        );

        target
            .rollback_managed_context_replica("owner-a", &receipt)
            .unwrap();
        let restored = target.get("owner-a", "codex", "default").unwrap();
        assert_eq!(restored.profile_id, "default");
        let restored_environment = target
            .resolve_environment("owner-a", "codex", "default")
            .unwrap();
        assert!(restored_environment["CODEX_HOME"].contains("home/.codex"));
        assert!(!imported_auth.exists());
        target
            .rollback_managed_context_replica("owner-a", &receipt)
            .unwrap();

        let _ = fs::remove_dir_all(source_root);
        let _ = fs::remove_dir_all(target_root);
    }

    #[test]
    fn managed_context_rollback_recovers_crash_and_deletion_failure() {
        let (source_root, source) = fixture();
        let source_profile = source
            .create_managed("owner-a", "codex", "Source")
            .expect("create source account");
        let source_environment = source
            .resolve_environment("owner-a", "codex", &source_profile.profile_id)
            .expect("resolve source account");
        fs::write(
            Path::new(&source_environment["CODEX_HOME"]).join("auth.json"),
            br#"{"token":"source"}"#,
        )
        .expect("write source credential");
        let materialization = source
            .export_managed_context_materialization("owner-a", "codex", &source_profile.profile_id)
            .expect("export source credential");

        let (target_root, target) = fixture();
        let registry_path = target_root.join("accounts.json");
        let receipt = target
            .materialize_managed_context_replica(
                "owner-a",
                "context-crash",
                &"a".repeat(64),
                &materialization,
            )
            .expect("materialize target credential");
        let managed_root = PathBuf::from(
            target
                .resolve_environment("owner-a", "codex", &source_profile.profile_id)
                .expect("resolve target account")["CODEX_HOME"]
                .clone(),
        );
        let rollback_root = managed_context_rollback_root(&managed_root, &receipt);
        fs::rename(&managed_root, &rollback_root).expect("simulate crash after rollback rename");
        drop(target);

        let target = ProviderAccountProfileRegistry::open(&registry_path)
            .expect("reopen registry after rollback crash");
        target
            .rollback_managed_context_replica("owner-a", &receipt)
            .expect("resume rollback after crash");
        assert!(!managed_root.exists());
        assert!(!rollback_root.exists());

        let receipt = target
            .materialize_managed_context_replica(
                "owner-a",
                "context-delete-failure",
                &"b".repeat(64),
                &materialization,
            )
            .expect("rematerialize target credential");
        let rollback_root = managed_context_rollback_root(&managed_root, &receipt);
        fs::rename(&managed_root, &rollback_root).expect("prepare failed cleanup state");
        FAIL_MANAGED_CONTEXT_ROLLBACK_CLEANUP_ONCE.with(|fail| fail.set(true));
        assert!(target
            .rollback_managed_context_replica("owner-a", &receipt)
            .is_err());
        assert!(target
            .get("owner-a", "codex", &source_profile.profile_id)
            .is_err());
        drop(target);

        let target = ProviderAccountProfileRegistry::open(&registry_path)
            .expect("reopen registry after cleanup failure");
        assert!(target
            .get("owner-a", "codex", &source_profile.profile_id)
            .is_err());
        target
            .rollback_managed_context_replica("owner-a", &receipt)
            .expect("finish receipt-bound cleanup after reopen");
        assert!(!rollback_root.exists());

        let receipt = target
            .materialize_managed_context_replica(
                "owner-a",
                "context-registry-sync-failure",
                &"c".repeat(64),
                &materialization,
            )
            .expect("rematerialize target credential for registry sync failure");
        let rollback_root = managed_context_rollback_root(&managed_root, &receipt);
        FAIL_ACCOUNT_PROFILE_REGISTRY_PARENT_SYNC_ONCE.with(|fail| fail.set(true));
        assert!(target
            .rollback_managed_context_replica("owner-a", &receipt)
            .is_err());
        assert!(managed_root.exists());
        assert!(!rollback_root.exists());
        drop(target);

        let target = ProviderAccountProfileRegistry::open(&registry_path)
            .expect("reopen registry after ambiguous registry commit");
        target
            .get("owner-a", "codex", &source_profile.profile_id)
            .expect("registry binding survives ambiguous rollback commit");
        assert!(managed_root.join("auth.json").is_file());
        target
            .rollback_managed_context_replica("owner-a", &receipt)
            .expect("retry rollback after registry recovery");
        assert!(!managed_root.exists());
        assert!(!rollback_root.exists());

        let _ = fs::remove_dir_all(source_root);
        let _ = fs::remove_dir_all(target_root);
    }

    #[test]
    fn managed_context_publication_recovers_pre_and_post_rename_crashes() {
        fn write_materialization_root(
            provider: &str,
            root: &Path,
            materialization: &ProviderAccountMaterialization,
        ) {
            let locator = ProviderAccountLocator::managed(provider, root).expect("root locator");
            create_private_roots(&locator).expect("create publication roots");
            for file in &materialization.files {
                let destination = materialization_destination(&locator, &file.relative_path)
                    .expect("materialization destination");
                let contents = base64::engine::general_purpose::STANDARD
                    .decode(&file.contents_base64)
                    .expect("decode credential");
                atomic_write_private(&destination, &contents).expect("write credential");
            }
            sync_private_tree(root).expect("sync publication root");
        }

        let (source_root, source) = fixture();
        let mut materializations = Vec::new();
        for (provider, environment_key, relative_path) in [
            ("codex", "CODEX_HOME", "auth.json"),
            ("opencode", "XDG_DATA_HOME", "opencode/auth.json"),
        ] {
            let profile = source
                .create_managed("owner-a", provider, provider)
                .expect("create source account");
            let environment = source
                .resolve_environment("owner-a", provider, &profile.profile_id)
                .expect("resolve source account");
            let credential = Path::new(&environment[environment_key]).join(relative_path);
            fs::create_dir_all(credential.parent().expect("credential parent"))
                .expect("create credential parent");
            fs::write(&credential, format!(r#"{{"provider":"{provider}"}}"#))
                .expect("write source credential");
            materializations.push(
                source
                    .export_managed_context_materialization(
                        "owner-a",
                        provider,
                        &profile.profile_id,
                    )
                    .expect("export source account"),
            );
        }

        let (target_root, target) = fixture();
        let registry_path = target_root.join("accounts.json");
        let managed_base = target_root
            .join("provider-accounts")
            .join(safe_path_component("owner-a"));

        let codex_materialization = &materializations[0];
        let codex_sha = provider_account_materialization_sha256(codex_materialization)
            .expect("hash Codex materialization");
        let codex_package_sha = "a".repeat(64);
        let codex_intent = ManagedContextReplicaIntent {
            context_id: "context-publication-crash",
            package_sha256: &codex_package_sha,
            materialization_sha256: &codex_sha,
        };
        let codex_root = managed_base
            .join("codex")
            .join(&codex_materialization.profile.profile_id);
        let codex_stage = managed_context_staging_root(&codex_root, codex_intent);
        write_materialization_root("codex", &codex_stage, codex_materialization);
        drop(target);

        let target = ProviderAccountProfileRegistry::open(&registry_path)
            .expect("reopen after pre-rename crash");
        target
            .materialize_managed_context_replica(
                "owner-a",
                codex_intent.context_id,
                codex_intent.package_sha256,
                codex_materialization,
            )
            .expect("recover deterministic pre-rename stage");
        assert!(!codex_stage.exists());
        assert!(target
            .get(
                "owner-a",
                "codex",
                &codex_materialization.profile.profile_id,
            )
            .is_ok());

        let opencode_materialization = &materializations[1];
        let opencode_sha = provider_account_materialization_sha256(opencode_materialization)
            .expect("hash OpenCode materialization");
        let opencode_package_sha = "b".repeat(64);
        let opencode_intent = ManagedContextReplicaIntent {
            context_id: "context-publication-crash",
            package_sha256: &opencode_package_sha,
            materialization_sha256: &opencode_sha,
        };
        let opencode_root = managed_base
            .join("opencode")
            .join(&opencode_materialization.profile.profile_id);
        let opencode_stage = managed_context_staging_root(&opencode_root, opencode_intent);
        write_materialization_root("opencode", &opencode_stage, opencode_materialization);
        fs::rename(&opencode_stage, &opencode_root)
            .expect("simulate crash after live-root publication");
        sync_directory(opencode_root.parent().expect("OpenCode root parent"))
            .expect("sync simulated live-root publication");
        drop(target);

        let target = ProviderAccountProfileRegistry::open(&registry_path)
            .expect("reopen after post-rename crash");
        target
            .materialize_managed_context_replica(
                "owner-a",
                opencode_intent.context_id,
                opencode_intent.package_sha256,
                opencode_materialization,
            )
            .expect("adopt exact post-rename publication");
        assert!(!opencode_stage.exists());
        assert!(target
            .get(
                "owner-a",
                "codex",
                &codex_materialization.profile.profile_id,
            )
            .is_ok());
        assert!(target
            .get(
                "owner-a",
                "opencode",
                &opencode_materialization.profile.profile_id,
            )
            .is_ok());

        let _ = fs::remove_dir_all(source_root);
        let _ = fs::remove_dir_all(target_root);
    }

    #[test]
    fn managed_context_import_compensation_recovers_ambiguous_registry_commit() {
        let (source_root, source) = fixture();
        let source_profile = source
            .create_managed("owner-a", "codex", "Source")
            .expect("create source account");
        let source_environment = source
            .resolve_environment("owner-a", "codex", &source_profile.profile_id)
            .expect("resolve source account");
        fs::write(
            Path::new(&source_environment["CODEX_HOME"]).join("auth.json"),
            br#"{"token":"source"}"#,
        )
        .expect("write source credential");
        let materialization = source
            .export_managed_context_materialization("owner-a", "codex", &source_profile.profile_id)
            .expect("export source credential");

        let (target_root, target) = fixture();
        let registry_path = target_root.join("accounts.json");
        let managed_root = target_root
            .join("provider-accounts")
            .join(safe_path_component("owner-a"))
            .join("codex")
            .join(&source_profile.profile_id);
        let managed_credential = managed_root.join("codex/auth.json");
        FAIL_ACCOUNT_PROFILE_REGISTRY_PARENT_SYNC_ONCE.with(|fail| fail.set(true));
        let error = target
            .materialize_managed_context_replica(
                "owner-a",
                "context-ambiguous-import",
                &"a".repeat(64),
                &materialization,
            )
            .expect_err("inject ambiguous import registry commit");
        assert!(managed_credential.is_file(), "{error:?}");
        assert!(target
            .get("owner-a", "codex", &source_profile.profile_id)
            .is_err());
        drop(target);

        let target = ProviderAccountProfileRegistry::open(&registry_path)
            .expect("reopen after ambiguous import registry commit");
        assert!(target
            .get("owner-a", "codex", &source_profile.profile_id)
            .is_err());
        let receipt = target
            .materialize_managed_context_replica(
                "owner-a",
                "context-ambiguous-import",
                &"a".repeat(64),
                &materialization,
            )
            .expect("adopt intact credential root after registry recovery");
        assert_eq!(
            fs::read_to_string(&managed_credential).expect("read recovered credential"),
            r#"{"token":"source"}"#
        );
        target
            .rollback_managed_context_replica("owner-a", &receipt)
            .expect("clean recovered provider account");

        let _ = fs::remove_dir_all(source_root);
        let _ = fs::remove_dir_all(target_root);
    }

    #[test]
    fn managed_context_uses_each_official_provider_credential_location() {
        for (provider, environment_key, relative_path, transfer_path) in [
            ("codex", "CODEX_HOME", "auth.json", "auth.json"),
            (
                "claude",
                "CLAUDE_CONFIG_DIR",
                ".credentials.json",
                ".credentials.json",
            ),
            (
                "opencode",
                "XDG_DATA_HOME",
                "opencode/auth.json",
                "data/opencode/auth.json",
            ),
        ] {
            let (source_root, source) = fixture();
            let source_profile = source
                .create_managed("owner-a", provider, "Work")
                .expect("create source provider account");
            let source_environment = source
                .resolve_environment("owner-a", provider, &source_profile.profile_id)
                .expect("resolve source provider account");
            let source_credential =
                Path::new(&source_environment[environment_key]).join(relative_path);
            fs::create_dir_all(source_credential.parent().expect("credential parent"))
                .expect("create credential parent");
            fs::write(
                &source_credential,
                format!(r#"{{"provider":"{provider}","token":"secret"}}"#),
            )
            .expect("write provider credential");
            let materialization = source
                .export_managed_context_materialization(
                    "owner-a",
                    provider,
                    &source_profile.profile_id,
                )
                .expect("export provider credential");
            assert_eq!(materialization.files.len(), 1);
            assert_eq!(materialization.files[0].relative_path, transfer_path);

            let (target_root, target) = fixture();
            let receipt = target
                .materialize_managed_context_replica(
                    "owner-a",
                    &format!("context-{provider}"),
                    &"a".repeat(64),
                    &materialization,
                )
                .expect("materialize provider credential");
            let target_environment = target
                .resolve_environment("owner-a", provider, &source_profile.profile_id)
                .expect("resolve target provider account");
            let target_credential =
                Path::new(&target_environment[environment_key]).join(relative_path);
            assert_eq!(
                fs::read_to_string(&target_credential).expect("read target provider credential"),
                format!(r#"{{"provider":"{provider}","token":"secret"}}"#)
            );
            target
                .rollback_managed_context_replica("owner-a", &receipt)
                .expect("roll back provider credential");
            assert!(!target_credential.exists());

            let _ = fs::remove_dir_all(source_root);
            let _ = fs::remove_dir_all(target_root);
        }
    }

    #[test]
    fn failed_replica_replacement_preserves_existing_credentials() {
        let (source_root, source) = fixture();
        let profile = source.create_managed("owner-a", "codex", "Work").unwrap();
        let source_environment = source
            .resolve_environment("owner-a", "codex", &profile.profile_id)
            .unwrap();
        fs::write(
            Path::new(&source_environment["CODEX_HOME"]).join("auth.json"),
            br#"{"token":"old"}"#,
        )
        .unwrap();
        let materialization = source
            .export_materialization("owner-a", "codex", &profile.profile_id)
            .unwrap();

        let (target_root, target) = fixture();
        let materialized = target
            .materialize_replica("owner-a", &materialization)
            .unwrap();
        let target_environment = target
            .resolve_environment("owner-a", "codex", &materialized.profile_id)
            .unwrap();
        let target_auth = Path::new(&target_environment["CODEX_HOME"]).join("auth.json");

        let mut invalid_replacement = materialization.clone();
        invalid_replacement
            .files
            .push(ProviderAccountMaterializationFile {
                relative_path: "replacement.json".to_string(),
                contents_base64: base64::engine::general_purpose::STANDARD.encode(b"new"),
            });
        invalid_replacement
            .files
            .push(ProviderAccountMaterializationFile {
                relative_path: "../escape".to_string(),
                contents_base64: base64::engine::general_purpose::STANDARD.encode(b"invalid"),
            });

        assert!(target
            .materialize_replica("owner-a", &invalid_replacement)
            .is_err());
        assert_eq!(
            fs::read_to_string(target_auth).unwrap(),
            r#"{"token":"old"}"#
        );
        assert!(target
            .get("owner-a", "codex", &materialized.profile_id)
            .is_ok());

        let _ = fs::remove_dir_all(source_root);
        let _ = fs::remove_dir_all(target_root);
    }

    #[test]
    fn default_alias_tracks_selected_provider_default() {
        let (root, registry) = fixture();
        registry
            .migrate_effective_defaults("owner-a", &root.join("home"))
            .unwrap();
        let work = registry
            .create_managed("owner-a", "claude", "Work")
            .unwrap();
        registry
            .set_default("owner-a", "claude", &work.profile_id)
            .unwrap();

        let default = registry.get("owner-a", "claude", "default").unwrap();
        assert_eq!(default.profile_id, work.profile_id);
        let environment = registry
            .resolve_environment("owner-a", "claude", "default")
            .unwrap();
        assert!(environment["CLAUDE_CONFIG_DIR"].contains(&work.profile_id));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn linked_profiles_are_never_deleted_by_registration_removal() {
        let (root, registry) = fixture();
        let linked = root.join("linked");
        fs::create_dir_all(&linked).unwrap();
        set_private_dir_permissions(&linked).unwrap();
        let profile = registry
            .link_existing("owner-a", "claude", "Existing", &linked)
            .unwrap();

        registry
            .remove_registration("owner-a", "claude", &profile.profile_id)
            .unwrap();

        assert!(linked.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn managed_kernels_reject_host_linked_provider_account_roots() {
        let _guard = crate::env_lock::lock();
        let previous = std::env::var_os(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV);
        std::env::remove_var(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV);

        let (root, registry) = fixture();
        let control_state = root.join("var/lib/chariox/home");
        fs::create_dir_all(&control_state).unwrap();
        set_private_dir_permissions(&control_state).unwrap();
        let linked = registry
            .link_existing("owner-a", "claude", "Control state", &control_state)
            .unwrap();

        std::env::set_var(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV, "1");
        let resolve_error = registry
            .resolve_environment("owner-a", "claude", &linked.profile_id)
            .expect_err("legacy linked control state must not enter a managed sandbox");
        assert!(resolve_error
            .to_string()
            .contains("cannot mount a host-linked provider account"));
        let link_error = registry
            .link_existing("owner-a", "claude", "Second link", &control_state)
            .expect_err("managed kernels must reject new host path links");
        assert!(link_error
            .to_string()
            .contains("cannot link arbitrary host provider-account paths"));

        match previous {
            Some(value) => {
                std::env::set_var(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV, value)
            }
            None => std::env::remove_var(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV),
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn destructive_delete_requires_created_origin_and_exact_confirmation() {
        let (root, registry) = fixture();
        let profile = registry
            .create_managed("owner-a", "opencode", "Work")
            .unwrap();
        assert!(registry
            .delete_managed_profile_data(
                "owner-a",
                "opencode",
                &profile.profile_id,
                "wrong-profile"
            )
            .is_err());
        registry
            .delete_managed_profile_data(
                "owner-a",
                "opencode",
                &profile.profile_id,
                &profile.profile_id,
            )
            .unwrap();
        assert!(registry
            .list("owner-a", Some("opencode"))
            .unwrap()
            .is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_insecure_linked_directories() {
        use std::os::unix::fs::PermissionsExt;

        let (root, registry) = fixture();
        let linked = root.join("insecure");
        fs::create_dir_all(&linked).unwrap();
        fs::set_permissions(&linked, fs::Permissions::from_mode(0o755)).unwrap();

        assert!(registry
            .link_existing("owner-a", "codex", "Unsafe", &linked)
            .is_err());
        let _ = fs::remove_dir_all(root);
    }
}
