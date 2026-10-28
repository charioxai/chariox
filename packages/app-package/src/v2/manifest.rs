use serde::{Deserialize, Serialize};

use super::{world::WorldId, AppId, APP_SCHEMA_V2};
use crate::{
    archive::validate_path, Capabilities, ErrorCode, Limits, Migrations, PackageError, Publisher,
    Result, Ui,
};

/// Kernel features a package needs. The kernel states the set it supports; a
/// package whose set is not covered is refused with the missing names, never by
/// a protocol number copied into the package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RequiredFeature {
    WasmComponent,
    UiActions,
    AgentPanel,
    Connections,
    DirectedEvents,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuntimeEngineV2 {
    Wasm,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeV2 {
    pub engine: RuntimeEngineV2,
    /// The portable backend component inside `runtime/`. Never a `.cwasm`.
    pub component: String,
}

/// How the component was built. Informational: backend language is build
/// metadata and never a second compatibility promise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Build {
    pub language: String,
    pub toolchain: String,
    pub sdk_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManifestV2 {
    pub schema: String,
    pub app_id: String,
    pub version: String,
    pub publisher: Publisher,
    pub app_contract_version: u32,
    pub resource_policy: String,
    /// Exact world, for example `chariox:app/app@1.0.0`.
    pub world: String,
    /// Exactly the features the content below needs (see `derive_features`).
    pub required_features: Vec<RequiredFeature>,
    pub runtime: RuntimeV2,
    pub build: Build,
    pub ui: Ui,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events: Option<String>,
    /// Backend actions with critical validation and protected effect routes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<String>,
    /// The signed UI action manifest agents run through the view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_actions: Option<String>,
    #[serde(default)]
    pub capabilities: Capabilities,
    /// Compiled migration components in the migration world.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migrations: Option<Migrations>,
}

impl ManifestV2 {
    /// The features this content needs; `requiredFeatures` must equal them so a
    /// package cannot understate what it uses.
    pub fn derive_features(&self) -> Vec<RequiredFeature> {
        let mut features = vec![RequiredFeature::WasmComponent];
        features.extend(self.ui_actions.as_ref().map(|_| RequiredFeature::UiActions));
        features.extend(
            self.ui
                .agent_panel
                .as_ref()
                .map(|_| RequiredFeature::AgentPanel),
        );
        if !self.capabilities.connections.is_empty() {
            features.push(RequiredFeature::Connections);
        }
        features.extend(
            self.events
                .as_ref()
                .map(|_| RequiredFeature::DirectedEvents),
        );
        features.sort_unstable();
        features
    }

    pub fn validate(&self, limits: &Limits) -> Result<()> {
        let invalid = |message| PackageError::new(ErrorCode::InvalidManifest, message);
        if self.schema != APP_SCHEMA_V2 {
            return Err(invalid("unsupported manifest schema"));
        }
        AppId::parse(&self.app_id)?;
        semver::Version::parse(&self.version).map_err(|_| invalid("version must be SemVer"))?;
        if self.app_contract_version == 0 || self.resource_policy.is_empty() {
            return Err(invalid("contract and resource policy are required"));
        }
        self.publisher.validate()?;
        WorldId::parse(&self.world)?;
        if self.required_features != self.derive_features() {
            return Err(invalid(
                "requiredFeatures must list exactly the features the package uses, sorted",
            ));
        }
        validate_path(&self.runtime.component, limits)?;
        validate_path(&self.ui.entry, limits)?;
        if !self.runtime.component.starts_with("runtime/")
            || !self.runtime.component.ends_with(".wasm")
        {
            return Err(invalid(
                "runtime component must be a .wasm file inside runtime/",
            ));
        }
        if !self.ui.entry.starts_with("ui/") || !self.ui.entry.ends_with(".html") {
            return Err(invalid("UI entry must be HTML inside ui/"));
        }
        if self
            .ui
            .agent_panel
            .as_ref()
            .is_some_and(|panel| !panel.size_valid())
        {
            return Err(invalid(
                "ui.agentPanel needs a size of 120 to 1200 CSS pixels (none takes no size)",
            ));
        }
        let language_ok = !self.build.language.is_empty()
            && self.build.language.len() <= 32
            && self
                .build
                .language
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-');
        let toolchain_ok = !self.build.toolchain.is_empty()
            && self.build.toolchain.len() <= 128
            && self.build.toolchain.bytes().all(|c| (32..127).contains(&c));
        if !language_ok || !toolchain_ok || semver::Version::parse(&self.build.sdk_version).is_err()
        {
            return Err(invalid(
                "build needs a language, a toolchain and a SemVer sdkVersion",
            ));
        }
        for path in self.declaration_paths() {
            validate_path(path, limits)?;
            if !path.starts_with("schemas/") || !path.ends_with(".json") {
                return Err(invalid("declarations must be JSON inside schemas/"));
            }
        }
        self.capabilities.validate()?;
        if let Some(migrations) = &self.migrations {
            migrations.validate(limits, |entry| entry.ends_with(".wasm"))?;
        }
        Ok(())
    }

    fn declaration_paths(&self) -> impl Iterator<Item = &str> {
        [&self.tools, &self.events, &self.actions, &self.ui_actions]
            .into_iter()
            .flatten()
            .map(String::as_str)
    }

    pub(crate) fn required_paths(&self) -> Vec<&str> {
        let mut paths = vec![self.runtime.component.as_str(), self.ui.entry.as_str()];
        paths.extend(self.declaration_paths());
        if let Some(migrations) = &self.migrations {
            paths.extend(migrations.steps.iter().map(|step| step.entry.as_str()));
        }
        paths
    }
}
