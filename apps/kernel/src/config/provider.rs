use serde::{Deserialize, Deserializer, Serialize, Serializer};

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
    pub workspace_live_sync: WorkspaceLiveSyncConfig,
}

impl Default for UserProviderConfig {
    fn default() -> Self {
        Self {
            default: Some("opencode".to_string()),
            model: Some("default".to_string()),
            account_profile: Some("default".to_string()),
            effort: None,
            workspace_live_sync: WorkspaceLiveSyncConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceLiveSyncConfig {
    pub mode: WorkspaceLiveSyncMode,
}

impl Default for WorkspaceLiveSyncConfig {
    fn default() -> Self {
        Self {
            mode: WorkspaceLiveSyncMode::Managed,
        }
    }
}

impl WorkspaceLiveSyncConfig {
    pub fn from_mode(mode: WorkspaceLiveSyncMode) -> Self {
        Self { mode }
    }

    pub fn requires_workspace_live_sync(&self) -> bool {
        self.mode.requires_workspace_live_sync()
    }

    pub fn tracks_workspace_live_sync(&self) -> bool {
        self.mode.tracks_workspace_live_sync()
    }

    pub(super) fn validate(&self) -> Result<(), DaemonError> {
        Ok(())
    }
}

impl From<WorkspaceLiveSyncMode> for WorkspaceLiveSyncConfig {
    fn from(mode: WorkspaceLiveSyncMode) -> Self {
        Self::from_mode(mode)
    }
}

impl From<WorkspaceLiveSyncConfig> for WorkspaceLiveSyncMode {
    fn from(value: WorkspaceLiveSyncConfig) -> Self {
        value.mode
    }
}

impl Serialize for WorkspaceLiveSyncConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(match self.mode {
            WorkspaceLiveSyncMode::Managed => "required",
            WorkspaceLiveSyncMode::Tracked => "tracked",
            WorkspaceLiveSyncMode::Unrestricted => "unrestricted",
        })
    }
}

impl<'de> Deserialize<'de> for WorkspaceLiveSyncConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        WorkspaceLiveSyncMode::parse(&value)
            .map(Self::from_mode)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceLiveSyncMode {
    #[serde(alias = "required")]
    Managed,
    Tracked,
    Unrestricted,
}

impl WorkspaceLiveSyncMode {
    pub(super) fn parse(value: &str) -> Result<Self, DaemonError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "required" | "on" | "true" | "1" => Ok(Self::Managed),
            "tracked" => Ok(Self::Tracked),
            "unrestricted" | "off" | "false" | "0" => Ok(Self::Unrestricted),
            _ => Err(DaemonError::InvalidConfig {
                field: "providers.workspace_live_sync",
                message: "value must be `required`, `tracked`, or `unrestricted`",
            }),
        }
    }

    pub fn requires_workspace_live_sync(&self) -> bool {
        matches!(self, Self::Managed)
    }

    pub fn tracks_workspace_live_sync(&self) -> bool {
        matches!(self, Self::Tracked)
    }
}
