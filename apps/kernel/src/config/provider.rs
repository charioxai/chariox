use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::DaemonError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserProviderConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default)]
    pub managed_io: ManagedIoConfig,
}

impl Default for UserProviderConfig {
    fn default() -> Self {
        Self {
            default: Some("opencode".to_string()),
            model: Some("default".to_string()),
            account_profile: Some("default".to_string()),
            effort: None,
            managed_io: ManagedIoConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "ManagedIoConfigSerde", into = "ManagedIoConfigSerde")]
pub struct ManagedIoConfig {
    pub mode: ManagedIoMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum ManagedIoConfigSerde {
    Mode(ManagedIoMode),
    LegacyModes(BTreeMap<String, ManagedIoMode>),
}

impl Default for ManagedIoConfig {
    fn default() -> Self {
        Self {
            mode: ManagedIoMode::Unrestricted,
        }
    }
}

impl ManagedIoConfig {
    pub fn from_mode(mode: ManagedIoMode) -> Self {
        Self { mode }
    }

    pub fn requires_managed_io(&self) -> bool {
        self.mode.requires_managed_io()
    }

    pub(super) fn validate(&self) -> Result<(), DaemonError> {
        Ok(())
    }
}

impl From<ManagedIoConfigSerde> for ManagedIoConfig {
    fn from(value: ManagedIoConfigSerde) -> Self {
        match value {
            ManagedIoConfigSerde::Mode(mode) => Self::from_mode(mode),
            ManagedIoConfigSerde::LegacyModes(modes) => {
                Self::from_mode(legacy_managed_io_mode(modes))
            }
        }
    }
}

impl From<ManagedIoConfig> for ManagedIoConfigSerde {
    fn from(value: ManagedIoConfig) -> Self {
        Self::Mode(value.mode)
    }
}

fn legacy_managed_io_mode(modes: BTreeMap<String, ManagedIoMode>) -> ManagedIoMode {
    if let Some(mode) = modes.get("default").copied() {
        return mode;
    }
    let Some(first) = modes.values().copied().next() else {
        return ManagedIoMode::Unrestricted;
    };
    if modes.values().all(|mode| *mode == first) {
        first
    } else {
        ManagedIoMode::Required
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedIoMode {
    Required,
    Unrestricted,
}

impl ManagedIoMode {
    pub(super) fn parse(value: &str) -> Result<Self, DaemonError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "required" | "managed" | "managed_io_required" | "on" | "true" | "1" => {
                Ok(Self::Required)
            }
            "unrestricted" | "off" | "false" | "0" => Ok(Self::Unrestricted),
            _ => Err(DaemonError::InvalidConfig {
                field: "providers.managed_io",
                message: "value must be `required` or `unrestricted`",
            }),
        }
    }

    pub fn requires_managed_io(&self) -> bool {
        matches!(self, Self::Required)
    }
}
