use std::collections::BTreeSet;

use semver::Version;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{archive::validate_path, ErrorCode, Limits, PackageError, Result};

pub const APP_SCHEMA: &str = "chariox.app.v1";
pub const APP_CONTRACT_VERSION: u32 = 1;
pub const RESOURCE_POLICY: &str = "chariox.app.resources.v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub schema: String,
    pub app_id: String,
    pub version: String,
    pub publisher: Publisher,
    pub sdk_version: String,
    pub app_contract_version: u32,
    pub min_kernel_protocol: u32,
    pub resource_policy: String,
    pub runtime: Runtime,
    pub ui: Ui,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub information_sets: Option<String>,
    #[serde(default)]
    pub capabilities: Capabilities,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migrations: Option<Migrations>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Publisher {
    pub id: String,
    pub key_id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Runtime {
    pub engine: RuntimeEngine,
    pub entry: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuntimeEngine {
    Node,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ui {
    pub entry: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Capabilities {
    #[serde(default)]
    pub network: Vec<NetworkDestination>,
    #[serde(default)]
    pub clipboard: Vec<ClipboardAccess>,
    #[serde(default)]
    pub external_files: Vec<ExternalFileAccess>,
    #[serde(default)]
    pub workflows: Vec<AssetAccess>,
    #[serde(default)]
    pub agents: Vec<AssetAccess>,
}

/// Destinations are exact HTTP(S) origins. Paths, redirects, connected IPs and
/// credential authority are enforced by the kernel broker at request time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkDestination {
    pub origin: String,
    pub methods: Vec<HttpMethod>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
    Head,
    Post,
    Put,
    Patch,
    Delete,
    Options,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardAccess {
    Read,
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalFileAccess {
    UserSelected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetAccess {
    Select,
    Read,
    Create,
    Update,
    Activate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Migrations {
    pub directory: String,
    pub target_version: u32,
    /// A complete linear migration chain from a clean schema (version zero).
    pub steps: Vec<Migration>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Migration {
    pub from: u32,
    pub to: u32,
    pub entry: String,
}

impl Manifest {
    pub fn validate(&self, limits: &Limits) -> Result<()> {
        let invalid = |message| PackageError::new(ErrorCode::InvalidManifest, message);
        if self.schema != APP_SCHEMA {
            return Err(invalid("unsupported manifest schema"));
        }
        if !valid_app_id(&self.app_id) {
            return Err(invalid("appId must be a lowercase reverse-DNS identifier"));
        }
        Version::parse(&self.version).map_err(|_| invalid("version must be SemVer"))?;
        Version::parse(&self.sdk_version).map_err(|_| invalid("sdkVersion must be SemVer"))?;
        if self.app_contract_version == 0
            || self.min_kernel_protocol == 0
            || self.resource_policy.is_empty()
        {
            return Err(invalid(
                "contract, protocol and resource policy are required",
            ));
        }
        if !valid_identifier(&self.publisher.id)
            || !valid_identifier(&self.publisher.key_id)
            || self.publisher.name.trim().is_empty()
            || self.publisher.name.len() > 256
        {
            return Err(invalid("publisher identity, keyId and name are required"));
        }
        validate_path(&self.runtime.entry, limits)?;
        validate_path(&self.ui.entry, limits)?;
        if !self.runtime.entry.starts_with("runtime/") || !javascript_path(&self.runtime.entry) {
            return Err(invalid("runtime entry must be JavaScript inside runtime/"));
        }
        if !self.ui.entry.starts_with("ui/") || !self.ui.entry.ends_with(".html") {
            return Err(invalid("UI entry must be HTML inside ui/"));
        }
        for path in [
            &self.tools,
            &self.events,
            &self.actions,
            &self.information_sets,
        ]
        .into_iter()
        .flatten()
        {
            validate_path(path, limits)?;
            if !path.starts_with("schemas/") || !path.ends_with(".json") {
                return Err(invalid("declarations must be JSON inside schemas/"));
            }
        }
        unique(&self.capabilities.clipboard)?;
        unique(&self.capabilities.external_files)?;
        unique(&self.capabilities.workflows)?;
        unique(&self.capabilities.agents)?;
        if self.capabilities.network.len() > 64 {
            return Err(invalid("too many network destinations"));
        }
        let mut origins = BTreeSet::new();
        for destination in &self.capabilities.network {
            validate_origin(&destination.origin)?;
            if !origins.insert(&destination.origin) || destination.methods.is_empty() {
                return Err(invalid(
                    "destinations must be unique and declare allowed methods",
                ));
            }
            unique(&destination.methods)?;
        }
        if let Some(migrations) = &self.migrations {
            validate_path(&migrations.directory, limits)?;
            if migrations.directory != "migrations"
                || migrations.steps.is_empty()
                || migrations.steps.len() > limits.max_declarations
                || migrations.steps.len() != migrations.target_version as usize
            {
                return Err(invalid(
                    "migrations must declare the complete bounded version chain",
                ));
            }
            let mut entries = BTreeSet::new();
            for (index, step) in migrations.steps.iter().enumerate() {
                validate_path(&step.entry, limits)?;
                if step.from as usize != index
                    || step.to as usize != index + 1
                    || !step.entry.starts_with("migrations/")
                    || !javascript_path(&step.entry)
                    || !entries.insert(&step.entry)
                {
                    return Err(invalid("migration steps must be unique JavaScript entries ordered from zero to target"));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn required_paths(&self) -> Vec<&str> {
        let mut paths = vec![self.runtime.entry.as_str(), self.ui.entry.as_str()];
        paths.extend(
            [
                &self.tools,
                &self.events,
                &self.actions,
                &self.information_sets,
            ]
            .into_iter()
            .flatten()
            .map(String::as_str),
        );
        if let Some(migrations) = &self.migrations {
            paths.extend(migrations.steps.iter().map(|step| step.entry.as_str()));
        }
        paths
    }
}

pub(crate) fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
        && value
            .bytes()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
}

pub(crate) fn valid_function_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
        && value.as_bytes()[0].is_ascii_lowercase()
}

fn valid_app_id(value: &str) -> bool {
    value.len() <= 128
        && value.split('.').count() >= 3
        && value.split('.').all(|part| {
            !part.is_empty()
                && part.as_bytes()[0].is_ascii_lowercase()
                && part
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
                && !part.ends_with('-')
        })
}

pub(crate) fn javascript_path(path: &str) -> bool {
    [".js", ".mjs", ".cjs"]
        .iter()
        .any(|suffix| path.ends_with(suffix))
}

pub(crate) fn validate_origin(origin: &str) -> Result<()> {
    let parsed = Url::parse(origin)
        .map_err(|_| PackageError::new(ErrorCode::InvalidManifest, "invalid network origin"))?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || parsed.origin().ascii_serialization() != origin
    {
        return Err(PackageError::new(ErrorCode::InvalidManifest, "network destinations must be canonical exact HTTP(S) origins without paths or credentials"));
    }
    Ok(())
}

fn unique<T: Ord>(values: &[T]) -> Result<()> {
    if values.iter().collect::<BTreeSet<_>>().len() != values.len() {
        return Err(PackageError::new(
            ErrorCode::InvalidManifest,
            "duplicate capability",
        ));
    }
    Ok(())
}
