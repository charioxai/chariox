//! Kernel-owned provider account profiles.
//!
//! Provider CLIs continue to own credential formats. This registry stores only
//! stable Chariox selection metadata and host-local provider root locators.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use rand::{distributions::Alphanumeric, Rng};
use serde::{Deserialize, Serialize};

use crate::error::DaemonError;

const REGISTRY_VERSION: u32 = 1;
const SUPPORTED_PROVIDERS: [&str; 3] = ["codex", "claude", "opencode"];

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
            if let ProviderAccountLocator::Codex { codex_home } = &locator {
                fs::create_dir_all(codex_home)
                    .map_err(registry_io("create default Codex account root"))?;
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

    pub(crate) fn resolve_environment(
        &self,
        owner_user_id: &str,
        provider: &str,
        profile_id: &str,
    ) -> Result<BTreeMap<String, String>, DaemonError> {
        let provider = normalize_provider(provider)?;
        let document = self.read_document()?;
        Ok(
            resolve_stored_profile(&document, owner_user_id, provider, profile_id)?
                .locator
                .environment(),
        )
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

fn enforce_codex_file_credentials(codex_home: &Path) -> Result<(), DaemonError> {
    let config_path = codex_home.join("config.toml");
    let config = "cli_auth_credentials_store = \"file\"\n";
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
