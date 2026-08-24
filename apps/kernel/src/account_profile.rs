//! Kernel-owned provider account profiles.
//!
//! Provider CLIs continue to own credential formats. This registry stores only
//! stable Chariox selection metadata and host-local provider root locators.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use base64::Engine as _;
use rand::{Rng, distributions::Alphanumeric};
use serde::{Deserialize, Serialize};

use crate::error::DaemonError;

const REGISTRY_VERSION: u32 = 1;
const SUPPORTED_PROVIDERS: [&str; 3] = ["codex", "claude", "opencode"];
const MAX_MATERIALIZATION_BYTES: usize = 64 * 1024 * 1024;

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

/// Version of the credential-kind contract shared with clients. Bump when the
/// serialized `credential_kind`/`credential_kind_not_reported_reason` shapes
/// change meaningfully.
pub const PROVIDER_CREDENTIAL_KIND_CONTRACT_VERSION: u32 = 1;

/// Provider-observed account/billing class for a credential. This is
/// deliberately separate from enrollment method and profile origin: an
/// imported/linked profile may carry any of these classes, so the kind stays
/// explicitly unknown (no value + a not-reported reason) until the
/// provider-native adapter actually reports it. Never secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCredentialKind {
    /// Subscription-backed access confirmed by the provider-native flow.
    Subscription,
    /// Static provider API key.
    ApiKey,
    /// Prepaid credit balance.
    Prepaid,
    /// More than one of the above on one account.
    Mixed,
}

/// Enrollment methods a provider adapter can run through its own native CLI /
/// app-server flow. Empty for providers without reliable programmatic
/// enrollment; callers must reject selections clearly instead of guessing.
pub fn supported_provider_enrollment_methods(provider: &str) -> &'static [&'static str] {
    match crate::provider::canonical_provider_family(provider) {
        Some("codex") => &["device_code"],
        Some("claude") | Some("opencode") => &["terminal"],
        _ => &[],
    }
}

/// Validates a client-selected enrollment method against what the provider
/// adapter actually supports. `None` keeps the provider's historical default.
pub fn validate_provider_enrollment_method(
    provider: &str,
    method: Option<&str>,
) -> Result<(), DaemonError> {
    let Some(method) = method.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    let supported = supported_provider_enrollment_methods(provider);
    if supported.contains(&method) {
        return Ok(());
    }
    Err(DaemonError::LocalTransport {
        operation: "start provider login",
        message: if supported.is_empty() {
            format!(
                "provider `{provider}` does not expose a reliable enrollment method; \
                 enroll through the provider's own CLI/app"
            )
        } else {
            format!(
                "enrollment method `{method}` is not supported for `{provider}`; \
                 supported methods: {}",
                supported.join(", ")
            )
        },
    })
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
    /// Versioned credential-kind contract (v1). `None` on records written
    /// before the contract existed; readers must treat it as not-reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_kind: Option<ProviderCredentialKind>,
    /// Set only when the adapter cannot reliably report the kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_kind_not_reported_reason: Option<String>,
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

    fn home_relative(provider: &str, home: &Path) -> Result<Self, DaemonError> {
        match provider {
            "codex" => Ok(Self::Codex {
                codex_home: home.join(".codex"),
            }),
            "claude" => Ok(Self::Claude {
                claude_config_dir: home.join(".claude"),
            }),
            "opencode" => {
                let config = home.join(".config");
                Ok(Self::Opencode {
                    xdg_data_home: home.join(".local/share"),
                    xdg_config_home: config.clone(),
                    xdg_state_home: home.join(".local/state"),
                    xdg_cache_home: home.join(".cache"),
                    opencode_config_dir: config.join("opencode"),
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
}

impl StoredProviderAccountProfile {
    fn environment(&self) -> BTreeMap<String, String> {
        if self.public.provider == "claude"
            && self.public.origin == ProviderAccountProfileOrigin::Default
            && !self.materialized_replica
        {
            return BTreeMap::new();
        }
        self.locator.environment()
    }
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
        let mut document = if path.exists() {
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
        let changed = migrate_legacy_default_profile_ids(&mut document)
            | migrate_legacy_default_profile_labels(&mut document);
        let registry = Self {
            path,
            document: Arc::new(RwLock::new(document)),
        };
        if changed {
            let document = registry.read_document()?;
            registry.persist_locked(&document)?;
        }
        Ok(registry)
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
            if let ProviderAccountLocator::Codex { codex_home } = &locator {
                fs::create_dir_all(codex_home)
                    .map_err(registry_io("create default Codex account root"))?;
                enforce_codex_file_credentials(codex_home)?;
            }
            let label = next_automatic_label(&document, owner_user_id, provider);
            let profile_id = unique_profile_id(&document, owner_user_id, provider, &label);
            let profile = new_public_profile(
                owner_user_id,
                provider,
                &profile_id,
                &label,
                ProviderAccountProfileOrigin::Default,
                true,
            );
            document.profiles.push(StoredProviderAccountProfile {
                public: profile,
                locator,
                materialized_replica: false,
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
        let profile_index = resolved_profile_index(&document, owner_user_id, provider, profile_id)?;
        if auth_state == ProviderAccountAuthState::Authenticated {
            if let Some(identity) = normalized_account_identity(identity_summary.as_deref()) {
                let duplicate_index =
                    document
                        .profiles
                        .iter()
                        .enumerate()
                        .find_map(|(index, candidate)| {
                            (index != profile_index
                                && candidate.public.owner_user_id == owner_user_id
                                && candidate.public.provider == provider
                                && candidate.public.auth_state
                                    == ProviderAccountAuthState::Authenticated
                                && normalized_account_identity(
                                    candidate.public.identity_summary.as_deref(),
                                )
                                .is_some_and(
                                    |candidate_identity| {
                                        candidate_identity.eq_ignore_ascii_case(identity)
                                    },
                                ))
                            .then_some(index)
                        });
                if let Some(duplicate_index) = duplicate_index {
                    let incoming_wins = document.profiles[profile_index].public.is_default
                        && !document.profiles[duplicate_index].public.is_default;
                    let losing_index = if incoming_wins {
                        duplicate_index
                    } else {
                        profile_index
                    };
                    document.profiles[losing_index].public.auth_state =
                        ProviderAccountAuthState::Error;
                    document.profiles[losing_index].public.last_validated_at_ms =
                        Some(crate::session::unix_epoch_ms());
                    mark_profile_materializations_stale(
                        &mut document.profiles[losing_index].public,
                    );
                    if !incoming_wins {
                        let existing_label =
                            document.profiles[duplicate_index].public.label.clone();
                        self.persist_locked(&document)?;
                        return Err(registry_error(
                            "validate account profile",
                            format!(
                                "this {provider} account is already authenticated as `{existing_label}`"
                            ),
                        ));
                    }
                }
            }
        }
        let profile = &mut document.profiles[profile_index];
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
        let document = self.read_document()?;
        Ok(resolve_stored_profile(&document, owner_user_id, provider, profile_id)?.environment())
    }

    pub fn create_managed(
        &self,
        owner_user_id: &str,
        provider: &str,
        label: &str,
    ) -> Result<ProviderAccountProfile, DaemonError> {
        let provider = normalize_provider(provider)?;
        let mut document = self.write_document()?;
        let label = resolved_new_profile_label(&document, owner_user_id, provider, label)?;
        ensure_unique_label(&document, owner_user_id, provider, &label)?;
        let profile_id = unique_profile_id(&document, owner_user_id, provider, &label);
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
            &label,
            ProviderAccountProfileOrigin::CharioxCreated,
            false,
        );
        document.profiles.push(StoredProviderAccountProfile {
            public: profile.clone(),
            locator,
            materialized_replica: false,
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
        let canonical = validate_linked_root(path)?;
        let mut document = self.write_document()?;
        let label = resolved_new_profile_label(&document, owner_user_id, provider, label)?;
        ensure_unique_label(&document, owner_user_id, provider, &label)?;
        let profile_id = unique_profile_id(&document, owner_user_id, provider, &label);
        let profile = new_public_profile(
            owner_user_id,
            provider,
            &profile_id,
            &label,
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
        let files = materialization_files(
            &stored.locator,
            stored.public.origin == ProviderAccountProfileOrigin::Default,
        )?;
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

    pub(crate) fn materialize_deployment_profile(
        &self,
        owner_user_id: &str,
        provider: &str,
        profile_id: &str,
        label: &str,
        source_home: &Path,
    ) -> Result<ProviderAccountProfile, DaemonError> {
        let provider = normalize_provider(provider)?;
        let profile_id = validate_profile_id(profile_id)?;
        let locator = ProviderAccountLocator::home_relative(provider, source_home)?;
        let files = materialization_files(&locator, false)?;
        if files.is_empty() {
            return Err(registry_error(
                "materialize deployment account profile",
                "provider credential profile is empty",
            ));
        }
        self.materialize_replica(
            owner_user_id,
            &ProviderAccountMaterialization {
                profile: ProviderAccountReplicaMetadata {
                    owner_user_id: owner_user_id.to_string(),
                    provider: provider.to_string(),
                    profile_id: profile_id.to_string(),
                    label: label.trim().to_string(),
                    origin: ProviderAccountProfileOrigin::Linked,
                    is_default: false,
                },
                files,
                generated_at_ms: crate::session::unix_epoch_ms(),
            },
        )
    }

    pub(crate) fn materialize_replica(
        &self,
        owner_user_id: &str,
        materialization: &ProviderAccountMaterialization,
    ) -> Result<ProviderAccountProfile, DaemonError> {
        if materialization.profile.owner_user_id != owner_user_id {
            return Err(registry_error(
                "materialize account profile",
                "materialization owner does not match the execution lease owner",
            ));
        }
        let provider = normalize_provider(&materialization.profile.provider)?;
        let profile_id = validate_profile_id(&materialization.profile.profile_id)?;
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
        let staging_root = unique_sibling_path(&managed_root, "stage");
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
            if decoded_bytes > MAX_MATERIALIZATION_BYTES {
                return Err(registry_error(
                    "materialize account profile",
                    "provider account materialization exceeds the 64 MiB safety limit",
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
        let replace_existing_replica = match document.profiles.iter().find(|stored| {
            stored.public.owner_user_id == owner_user_id
                && stored.public.provider == provider
                && stored.public.profile_id == profile_id
        }) {
            Some(stored) if !stored.materialized_replica => {
                let _ = fs::remove_dir_all(&staging_root);
                return Err(registry_error(
                    "materialize account profile",
                    "refusing to replace an authoritative local account profile",
                ));
            }
            Some(_) => true,
            None => false,
        };
        if managed_root.exists() && !replace_existing_replica {
            let _ = fs::remove_dir_all(&staging_root);
            return Err(registry_error(
                "materialize account profile",
                "refusing to replace unregistered provider account data",
            ));
        }

        let backup_root = managed_root
            .exists()
            .then(|| unique_sibling_path(&managed_root, "backup"));
        if let Some(backup_root) = &backup_root {
            if let Err(error) = fs::rename(&managed_root, backup_root) {
                let _ = fs::remove_dir_all(&staging_root);
                return Err(registry_io("materialize account profile")(error));
            }
        }
        if let Err(error) = fs::rename(&staging_root, &managed_root) {
            if let Some(backup_root) = &backup_root {
                let _ = fs::rename(backup_root, &managed_root);
            }
            let _ = fs::remove_dir_all(&staging_root);
            return Err(registry_io("materialize account profile")(error));
        }
        if let Err(error) = sync_directory(managed_parent) {
            let failed_root = unique_sibling_path(&managed_root, "failed");
            let _ = fs::rename(&managed_root, &failed_root);
            if let Some(backup_root) = &backup_root {
                let _ = fs::rename(backup_root, &managed_root);
            }
            let _ = fs::remove_dir_all(&failed_root);
            return Err(error);
        }

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
            });
            public
        };

        if let Err(error) = self.persist_locked(&document) {
            *document = original_document;
            let failed_root = unique_sibling_path(&managed_root, "failed");
            let _ = fs::rename(&managed_root, &failed_root);
            if let Some(backup_root) = &backup_root {
                let _ = fs::rename(backup_root, &managed_root);
            }
            let _ = fs::remove_dir_all(&failed_root);
            let rollback_result = self.persist_locked(&document);
            let _ = sync_directory(managed_parent);
            return match rollback_result {
                Ok(()) => Err(error),
                Err(rollback_error) => Err(registry_error(
                    "materialize account profile",
                    format!(
                        "{error}; additionally failed to restore the account profile registry: {rollback_error}"
                    ),
                )),
            };
        }
        if let Some(backup_root) = &backup_root {
            let _ = fs::remove_dir_all(backup_root);
            let _ = sync_directory(managed_parent);
        }
        Ok(result)
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
        atomic_write_private(&self.path, &bytes)
    }
}

fn credential_kind_for_new_profile(
    origin: ProviderAccountProfileOrigin,
    provider: &str,
) -> (Option<ProviderCredentialKind>, Option<String>) {
    // Only managed Codex profiles have a known account class up front: their
    // sole enrollment surface is the official app-server ChatGPT
    // subscription device-code flow. Everything else — including
    // linked/imported roots, which may hold subscription, API-key, prepaid,
    // or mixed credentials — stays explicitly unknown until the adapter
    // reports the class.
    if origin == ProviderAccountProfileOrigin::CharioxCreated
        && crate::provider::canonical_provider_family(provider) == Some("codex")
    {
        return (Some(ProviderCredentialKind::Subscription), None);
    }
    let reason = match origin {
        ProviderAccountProfileOrigin::Linked => {
            "imported credentials are not classified until the provider reports the account type"
        }
        _ => "the provider-native login does not report the resulting credential type",
    };
    (None, Some(reason.to_string()))
}

fn new_public_profile(
    owner_user_id: &str,
    provider: &str,
    profile_id: &str,
    label: &str,
    origin: ProviderAccountProfileOrigin,
    is_default: bool,
) -> ProviderAccountProfile {
    let (credential_kind, credential_kind_not_reported_reason) =
        credential_kind_for_new_profile(origin, provider);
    ProviderAccountProfile {
        owner_user_id: owner_user_id.to_string(),
        provider: provider.to_string(),
        profile_id: profile_id.to_string(),
        label: label.to_string(),
        origin,
        is_default,
        auth_state: ProviderAccountAuthState::Unknown,
        credential_kind,
        credential_kind_not_reported_reason,
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
    if label.eq_ignore_ascii_case("default") {
        return Err(registry_error(
            "validate account profile",
            "`default` is reserved for the provider-level account pointer",
        ));
    }
    Ok(label)
}

fn resolved_new_profile_label(
    document: &RegistryDocument,
    owner_user_id: &str,
    provider: &str,
    requested: &str,
) -> Result<String, DaemonError> {
    let requested = requested.trim();
    if requested.is_empty() {
        Ok(next_automatic_label(document, owner_user_id, provider))
    } else {
        Ok(validate_label(requested)?.to_string())
    }
}

fn next_automatic_label(
    document: &RegistryDocument,
    owner_user_id: &str,
    provider: &str,
) -> String {
    let mut index = 1_u64;
    loop {
        let candidate = format!("{provider}-{index}");
        if !document.profiles.iter().any(|profile| {
            profile.public.owner_user_id == owner_user_id
                && profile.public.provider == provider
                && profile.public.label.eq_ignore_ascii_case(&candidate)
        }) {
            return candidate;
        }
        index += 1;
    }
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

fn migrate_legacy_default_profile_ids(document: &mut RegistryDocument) -> bool {
    let legacy_profiles = document
        .profiles
        .iter()
        .enumerate()
        .filter(|(_, profile)| profile.public.profile_id == "default")
        .map(|(index, profile)| {
            (
                index,
                profile.public.owner_user_id.clone(),
                profile.public.provider.clone(),
            )
        })
        .collect::<Vec<_>>();
    for (index, owner_user_id, provider) in &legacy_profiles {
        let profile_id = unique_profile_id(document, owner_user_id, provider, "Native default");
        let profile = &mut document.profiles[*index].public;
        profile.profile_id = profile_id.clone();
        profile.usage.profile_id = profile_id;
    }
    !legacy_profiles.is_empty()
}

fn migrate_legacy_default_profile_labels(document: &mut RegistryDocument) -> bool {
    let legacy_profiles = document
        .profiles
        .iter()
        .enumerate()
        .filter(|(_, profile)| {
            profile.public.label.trim().is_empty()
                || profile.public.label.eq_ignore_ascii_case("default")
        })
        .map(|(index, profile)| {
            (
                index,
                profile.public.owner_user_id.clone(),
                profile.public.provider.clone(),
            )
        })
        .collect::<Vec<_>>();
    for (index, owner_user_id, provider) in &legacy_profiles {
        let label = next_automatic_label(document, owner_user_id, provider);
        document.profiles[*index].public.label = label;
    }
    !legacy_profiles.is_empty()
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

fn normalized_account_identity(identity: Option<&str>) -> Option<&str> {
    identity
        .map(str::trim)
        .filter(|identity| !identity.is_empty())
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

fn materialization_files(
    locator: &ProviderAccountLocator,
    include_default_claude_keychain: bool,
) -> Result<Vec<ProviderAccountMaterializationFile>, DaemonError> {
    let mut files = Vec::new();
    match locator {
        ProviderAccountLocator::Codex { codex_home } => {
            collect_optional_file(codex_home, "auth.json", "auth.json", &mut files)?;
            collect_optional_file(codex_home, "config.toml", "config.toml", &mut files)?;
        }
        ProviderAccountLocator::Claude { claude_config_dir } => {
            for name in [".credentials.json", "settings.json", "stats-cache.json"] {
                collect_optional_file(claude_config_dir, name, name, &mut files)?;
            }
            if include_default_claude_keychain
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
            collect_optional_tree(&xdg_data_home.join("opencode"), "data/opencode", &mut files)?;
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
    Ok(files)
}

fn collect_optional_file(
    root: &Path,
    source_relative_path: &str,
    transfer_relative_path: &str,
    files: &mut Vec<ProviderAccountMaterializationFile>,
) -> Result<(), DaemonError> {
    let source = root.join(source_relative_path);
    let metadata = match fs::symlink_metadata(&source) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(registry_error("export account profile", error.to_string())),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(registry_error(
            "export account profile",
            format!("credential file `{source_relative_path}` must be a regular file"),
        ));
    }
    let contents = fs::read(&source).map_err(registry_io("export account profile"))?;
    let existing_bytes = files
        .iter()
        .map(|file| file.contents_base64.len().saturating_mul(3) / 4)
        .sum::<usize>();
    if existing_bytes.saturating_add(contents.len()) > MAX_MATERIALIZATION_BYTES {
        return Err(registry_error(
            "export account profile",
            "provider account materialization exceeds the 64 MiB safety limit",
        ));
    }
    files.push(ProviderAccountMaterializationFile {
        relative_path: transfer_relative_path.to_string(),
        contents_base64: base64::engine::general_purpose::STANDARD.encode(contents),
    });
    Ok(())
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
                    ));
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
            ));
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
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(registry_io("write account profile registry"))?;
    set_private_dir_permissions(parent)?;
    let suffix: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(8)
        .map(char::from)
        .collect();
    let temporary = parent.join(format!(".account-profiles-{suffix}.tmp"));
    fs::write(&temporary, bytes).map_err(registry_io("write account profile registry"))?;
    set_private_file_permissions(&temporary)?;
    fs::rename(&temporary, path).map_err(registry_io("write account profile registry"))?;
    set_private_file_permissions(path)
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
    fn credential_kind_contract_is_versioned_and_grounded_in_observable_facts() {
        assert_eq!(PROVIDER_CREDENTIAL_KIND_CONTRACT_VERSION, 1);
        // The contract pins the wire names for every observable class so
        // clients can rely on them without ever seeing secret material.
        assert_eq!(
            serde_json::to_value(ProviderCredentialKind::Subscription).unwrap(),
            serde_json::json!("subscription")
        );
        assert_eq!(
            serde_json::to_value(ProviderCredentialKind::ApiKey).unwrap(),
            serde_json::json!("api_key")
        );
        assert_eq!(
            serde_json::to_value(ProviderCredentialKind::Prepaid).unwrap(),
            serde_json::json!("prepaid")
        );
        assert_eq!(
            serde_json::to_value(ProviderCredentialKind::Mixed).unwrap(),
            serde_json::json!("mixed")
        );

        let (root, registry) = fixture();
        let managed_codex = registry.create_managed("owner-a", "codex", "Work").unwrap();
        assert_eq!(
            managed_codex.credential_kind,
            Some(ProviderCredentialKind::Subscription)
        );
        assert_eq!(managed_codex.credential_kind_not_reported_reason, None);

        // Claude/OpenCode native logins do not report the resulting credential
        // type, so the contract requires an explicit not-reported reason.
        let managed_claude = registry
            .create_managed("owner-a", "claude", "Terminal Work")
            .unwrap();
        assert_eq!(managed_claude.credential_kind, None);
        assert_eq!(
            managed_claude
                .credential_kind_not_reported_reason
                .as_deref(),
            Some("the provider-native login does not report the resulting credential type")
        );

        // Linked/imported roots are origin facts, not class facts: they may
        // hold subscription, API-key, prepaid, or mixed credentials, so the
        // class stays explicitly unknown until the adapter reports it.
        let linked_root = std::env::temp_dir()
            .join(format!("chariox-linked-kind-{}", rand::thread_rng().gen::<u64>()));
        fs::create_dir_all(&linked_root).unwrap();
        let linked = registry
            .link_existing(
                "owner-a",
                "opencode",
                "Imported Work",
                &linked_root,
            )
            .unwrap();
        assert_eq!(linked.credential_kind, None);
        assert_eq!(
            linked.credential_kind_not_reported_reason.as_deref(),
            Some(
                "imported credentials are not classified until the provider reports the account type"
            )
        );
        let _ = fs::remove_dir_all(&linked_root);

        // Legacy records written before the contract deserialize with no kind;
        // readers must treat that as not-reported.
        let legacy: ProviderAccountProfile = serde_json::from_str(
            r#"{"owner_user_id":"owner-a","provider":"codex","profile_id":"legacy",
                "label":"Legacy","origin":"default","is_default":true,
                "auth_state":"unknown","usage":{"profile_id":"legacy","provider":"codex",
                "availability":"unavailable","meters":[],"source":"provider_not_observed"}}"#,
        )
        .expect("legacy profile should deserialize");
        assert_eq!(legacy.credential_kind, None);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn enrollment_method_support_is_grounded_in_adapter_facts() {
        assert_eq!(
            supported_provider_enrollment_methods("codex"),
            &["device_code"]
        );
        assert_eq!(
            supported_provider_enrollment_methods("claude-p"),
            &["terminal"]
        );
        assert_eq!(
            supported_provider_enrollment_methods("opencode"),
            &["terminal"]
        );
        let expected_empty: &[&str] = &[];
        assert_eq!(
            supported_provider_enrollment_methods("dev-stub"),
            expected_empty
        );

        validate_provider_enrollment_method("codex", Some("device_code")).unwrap();
        validate_provider_enrollment_method("claude", Some("terminal")).unwrap();
        validate_provider_enrollment_method("opencode", None).unwrap();

        let unsupported = validate_provider_enrollment_method("codex", Some("api_key"))
            .expect_err("unsupported method must be rejected");
        match unsupported {
            DaemonError::LocalTransport { message, .. } => {
                assert!(message.contains("device_code"), "{message}");
                assert!(!message.contains("secret"), "{message}");
            }
            other => panic!("expected clear rejection, got {other:?}"),
        }
        assert!(validate_provider_enrollment_method("dev-stub", Some("terminal")).is_err());
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
        assert!(first.iter().all(|profile| profile.profile_id != "default"));
        assert!(
            first
                .iter()
                .all(|profile| profile.label == format!("{}-1", profile.provider))
        );
        assert_eq!(
            first
                .iter()
                .map(|profile| &profile.profile_id)
                .collect::<Vec<_>>(),
            second
                .iter()
                .map(|profile| &profile.profile_id)
                .collect::<Vec<_>>(),
        );
        assert!(first.iter().all(|profile| profile.is_default));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn assigns_sequential_provider_aliases_when_labels_are_omitted() {
        let (root, registry) = fixture();
        registry
            .migrate_effective_defaults("owner-a", &root.join("home"))
            .unwrap();

        let second = registry.create_managed("owner-a", "codex", "").unwrap();
        let linked_root = root.join("linked-codex");
        fs::create_dir_all(&linked_root).unwrap();
        set_private_dir_permissions(&linked_root).unwrap();
        let third = registry
            .link_existing("owner-a", "codex", "   ", &linked_root)
            .unwrap();
        let named = registry
            .create_managed("owner-a", "codex", "client-work")
            .unwrap();

        assert_eq!(second.label, "codex-2");
        assert_eq!(third.label, "codex-3");
        assert_eq!(named.label, "client-work");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn reserves_default_for_the_provider_pointer() {
        let (root, registry) = fixture();
        let native = registry
            .migrate_effective_defaults("owner-a", &root.join("home"))
            .unwrap()
            .into_iter()
            .find(|profile| profile.provider == "codex")
            .unwrap();

        let rename_error = registry
            .rename("owner-a", "codex", &native.profile_id, "Default")
            .unwrap_err();
        let create_error = registry
            .create_managed("owner-a", "codex", "default")
            .unwrap_err();

        assert!(rename_error.to_string().contains("reserved"));
        assert!(create_error.to_string().contains("reserved"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn migrates_legacy_default_aliases_regardless_of_profile_origin() {
        let (root, registry) = fixture();
        let profile = registry
            .create_managed("owner-a", "codex", "legacy-name")
            .unwrap();
        {
            let mut document = registry.write_document().unwrap();
            document
                .profiles
                .iter_mut()
                .find(|candidate| candidate.public.profile_id == profile.profile_id)
                .unwrap()
                .public
                .label = "Default".to_string();
            registry.persist_locked(&document).unwrap();
        }
        drop(registry);
        let registry = ProviderAccountProfileRegistry::open(root.join("accounts.json")).unwrap();

        let migrated = registry
            .list("owner-a", Some("codex"))
            .unwrap()
            .into_iter()
            .find(|candidate| candidate.profile_id == profile.profile_id)
            .unwrap();

        assert_eq!(migrated.label, "codex-1");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn default_claude_profile_inherits_the_host_environment() {
        let (root, registry) = fixture();
        registry
            .migrate_effective_defaults("owner-a", &root.join("home"))
            .unwrap();

        let environment = registry
            .resolve_environment("owner-a", "claude", "default")
            .unwrap();

        assert!(!environment.contains_key("CLAUDE_CONFIG_DIR"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn explicit_claude_profiles_select_their_config_directories() {
        let (root, registry) = fixture();
        let managed = registry
            .create_managed("owner-a", "claude", "Managed")
            .unwrap();
        let linked_root = root.join("linked-claude");
        fs::create_dir_all(&linked_root).unwrap();
        set_private_dir_permissions(&linked_root).unwrap();
        let linked = registry
            .link_existing("owner-a", "claude", "Linked", &linked_root)
            .unwrap();

        let managed_environment = registry
            .resolve_environment("owner-a", "claude", &managed.profile_id)
            .unwrap();
        let linked_environment = registry
            .resolve_environment("owner-a", "claude", &linked.profile_id)
            .unwrap();

        assert!(managed_environment["CLAUDE_CONFIG_DIR"].contains(&managed.profile_id));
        assert_eq!(
            Path::new(&linked_environment["CLAUDE_CONFIG_DIR"]),
            linked_root.canonicalize().unwrap()
        );
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
    fn deployment_profiles_materialize_from_isolated_provider_homes() {
        let (root, registry) = fixture();
        let source_home = root.join("mounted-profile/home");
        fs::create_dir_all(source_home.join(".codex")).unwrap();
        fs::write(
            source_home.join(".codex/auth.json"),
            "{\"token\":\"secret\"}",
        )
        .unwrap();
        fs::write(
            source_home.join(".codex/config.toml"),
            "model = \"gpt-test\"\n",
        )
        .unwrap();

        let profile = registry
            .materialize_deployment_profile(
                "local",
                "codex",
                "cloud-profile-2",
                "Codex validation",
                &source_home,
            )
            .unwrap();
        let environment = registry
            .resolve_environment("local", "codex", &profile.profile_id)
            .unwrap();
        let codex_home = Path::new(&environment["CODEX_HOME"]);

        assert_eq!(profile.profile_id, "cloud-profile-2");
        assert_eq!(profile.label, "Codex validation");
        assert_eq!(
            fs::read_to_string(codex_home.join("auth.json")).unwrap(),
            "{\"token\":\"secret\"}"
        );
        assert!(codex_home.starts_with(root.join("provider-accounts/local/codex/cloud-profile-2")));
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
        assert!(
            target
                .materialize_replica("owner-b", &materialization)
                .is_err()
        );
        let _ = fs::remove_dir_all(source_root);
        let _ = fs::remove_dir_all(target_root);
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

        assert!(
            target
                .materialize_replica("owner-a", &invalid_replacement)
                .is_err()
        );
        assert_eq!(
            fs::read_to_string(target_auth).unwrap(),
            r#"{"token":"old"}"#
        );
        assert!(
            target
                .get("owner-a", "codex", &materialized.profile_id)
                .is_ok()
        );

        let _ = fs::remove_dir_all(source_root);
        let _ = fs::remove_dir_all(target_root);
    }

    #[test]
    fn default_alias_tracks_selected_provider_default() {
        let (root, registry) = fixture();
        let native_default = registry
            .migrate_effective_defaults("owner-a", &root.join("home"))
            .unwrap()
            .into_iter()
            .find(|profile| profile.provider == "claude")
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

        registry
            .set_default("owner-a", "claude", &native_default.profile_id)
            .unwrap();
        let restored = registry.get("owner-a", "claude", "default").unwrap();
        assert_eq!(restored.profile_id, native_default.profile_id);
        assert!(
            !registry
                .resolve_environment("owner-a", "claude", "default")
                .unwrap()
                .contains_key("CLAUDE_CONFIG_DIR")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_authenticating_the_same_identity_in_two_profiles() {
        let (root, registry) = fixture();
        registry
            .migrate_effective_defaults("owner-a", &root.join("home"))
            .unwrap();
        let secondary = registry
            .create_managed("owner-a", "codex", "Secondary")
            .unwrap();
        registry
            .update_observation(
                "owner-a",
                "codex",
                "default",
                ProviderAccountAuthState::Authenticated,
                Some("dev@example.test".to_string()),
                Some("plus".to_string()),
                None,
                None,
            )
            .unwrap();

        let error = registry
            .update_observation(
                "owner-a",
                "codex",
                &secondary.profile_id,
                ProviderAccountAuthState::Authenticated,
                Some("DEV@example.test".to_string()),
                Some("plus".to_string()),
                None,
                None,
            )
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("already authenticated as `codex-1`")
        );
        assert_eq!(
            registry
                .get("owner-a", "codex", &secondary.profile_id)
                .unwrap()
                .auth_state,
            ProviderAccountAuthState::Error,
        );
        assert_eq!(
            registry
                .get("owner-a", "codex", "default")
                .unwrap()
                .auth_state,
            ProviderAccountAuthState::Authenticated,
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn the_selected_default_profile_wins_an_existing_identity_collision() {
        let (root, registry) = fixture();
        registry
            .migrate_effective_defaults("owner-a", &root.join("home"))
            .unwrap();
        let secondary = registry
            .create_managed("owner-a", "codex", "Secondary")
            .unwrap();
        registry
            .update_observation(
                "owner-a",
                "codex",
                &secondary.profile_id,
                ProviderAccountAuthState::Authenticated,
                Some("dev@example.test".to_string()),
                None,
                None,
                None,
            )
            .unwrap();

        registry
            .update_observation(
                "owner-a",
                "codex",
                "default",
                ProviderAccountAuthState::Authenticated,
                Some("dev@example.test".to_string()),
                None,
                None,
                None,
            )
            .unwrap();

        assert_eq!(
            registry
                .get("owner-a", "codex", "default")
                .unwrap()
                .auth_state,
            ProviderAccountAuthState::Authenticated,
        );
        assert_eq!(
            registry
                .get("owner-a", "codex", &secondary.profile_id)
                .unwrap()
                .auth_state,
            ProviderAccountAuthState::Error,
        );
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
    fn destructive_delete_requires_created_origin_and_exact_confirmation() {
        let (root, registry) = fixture();
        let profile = registry
            .create_managed("owner-a", "opencode", "Work")
            .unwrap();
        assert!(
            registry
                .delete_managed_profile_data(
                    "owner-a",
                    "opencode",
                    &profile.profile_id,
                    "wrong-profile"
                )
                .is_err()
        );
        registry
            .delete_managed_profile_data(
                "owner-a",
                "opencode",
                &profile.profile_id,
                &profile.profile_id,
            )
            .unwrap();
        assert!(
            registry
                .list("owner-a", Some("opencode"))
                .unwrap()
                .is_empty()
        );
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

        assert!(
            registry
                .link_existing("owner-a", "codex", "Unsafe", &linked)
                .is_err()
        );
        let _ = fs::remove_dir_all(root);
    }
}
