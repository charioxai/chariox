use serde::{Deserialize, Serialize};

use crate::error::DaemonError;

use super::{validate_non_empty, validate_optional_nonzero};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserHistoryConfig {
    #[serde(default)]
    pub operational: UserOperationalHistoryConfig,
    #[serde(default)]
    pub archive: UserArchiveHistoryConfig,
}

impl Default for UserHistoryConfig {
    fn default() -> Self {
        Self {
            operational: UserOperationalHistoryConfig::default(),
            archive: UserArchiveHistoryConfig::default(),
        }
    }
}

impl UserHistoryConfig {
    pub(super) fn validate(&self) -> Result<(), DaemonError> {
        self.operational.validate()?;
        self.archive.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserOperationalHistoryConfig {
    #[serde(default = "default_history_capture_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub backend: HistoryOperationalBackend,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention_days: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_size_mb: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_pinned_sessions: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_inactive_after_days: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_deleted_agents: Option<bool>,
}

impl Default for UserOperationalHistoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: HistoryOperationalBackend::Sqlite,
            path: Some("~/.chariox/history/operational.db".to_string()),
            retention_days: Some(30),
            max_size_mb: Some(crate::history::OPERATIONAL_HISTORY_HARD_MAX_MB),
            keep_pinned_sessions: Some(true),
            archive_inactive_after_days: Some(30),
            archive_deleted_agents: Some(true),
        }
    }
}

const fn default_history_capture_enabled() -> bool {
    true
}

impl UserOperationalHistoryConfig {
    fn validate(&self) -> Result<(), DaemonError> {
        if let Some(path) = &self.path {
            validate_non_empty("history.operational.path", path)?;
        }
        validate_optional_nonzero("history.operational.retention_days", self.retention_days)?;
        validate_optional_nonzero("history.operational.max_size_mb", self.max_size_mb)?;
        validate_optional_nonzero(
            "history.operational.archive_inactive_after_days",
            self.archive_inactive_after_days,
        )?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryOperationalBackend {
    Sqlite,
}

impl Default for HistoryOperationalBackend {
    fn default() -> Self {
        Self::Sqlite
    }
}

impl HistoryOperationalBackend {
    pub(super) fn parse(field: &'static str, value: &str) -> Result<Self, DaemonError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "sqlite" => Ok(Self::Sqlite),
            _ => Err(DaemonError::InvalidConfig {
                field,
                message: "value must be `sqlite`",
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserArchiveHistoryConfig {
    #[serde(default)]
    pub mode: HistoryArchiveMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_deleted_agents: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_before_delete: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delete_operational_after_verified_archive: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_durable_acceptance: Option<bool>,
}

impl Default for UserArchiveHistoryConfig {
    fn default() -> Self {
        Self {
            mode: HistoryArchiveMode::Disabled,
            url: None,
            token_env: None,
            archive_deleted_agents: Some(true),
            archive_before_delete: Some(true),
            delete_operational_after_verified_archive: Some(true),
            require_durable_acceptance: Some(true),
        }
    }
}

impl UserArchiveHistoryConfig {
    fn validate(&self) -> Result<(), DaemonError> {
        match self.mode {
            HistoryArchiveMode::Disabled => Ok(()),
            HistoryArchiveMode::External => {
                let Some(url) = self.url.as_deref() else {
                    return Err(DaemonError::InvalidConfig {
                        field: "history.archive.url",
                        message: "value must be set when archive mode is external",
                    });
                };
                validate_non_empty("history.archive.url", url)?;
                if let Some(token_env) = &self.token_env {
                    validate_non_empty("history.archive.token_env", token_env)?;
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryArchiveMode {
    Disabled,
    External,
}

impl Default for HistoryArchiveMode {
    fn default() -> Self {
        Self::Disabled
    }
}

impl HistoryArchiveMode {
    pub(super) fn parse(value: &str) -> Result<Self, DaemonError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "disabled" | "off" | "false" | "0" | "none" => Ok(Self::Disabled),
            "external" => Ok(Self::External),
            _ => Err(DaemonError::InvalidConfig {
                field: "history.archive.mode",
                message: "value must be `disabled` or `external`",
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserArtifactsConfig {
    #[serde(default)]
    pub operational: UserOperationalArtifactsConfig,
    #[serde(default)]
    pub archive: UserArchiveArtifactsConfig,
}

impl Default for UserArtifactsConfig {
    fn default() -> Self {
        Self {
            operational: UserOperationalArtifactsConfig::default(),
            archive: UserArchiveArtifactsConfig::default(),
        }
    }
}

impl UserArtifactsConfig {
    pub(super) fn validate(&self) -> Result<(), DaemonError> {
        self.operational.validate()?;
        self.archive.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserOperationalArtifactsConfig {
    #[serde(default)]
    pub backend: ArtifactOperationalBackend,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention_days: Option<u32>,
}

impl Default for UserOperationalArtifactsConfig {
    fn default() -> Self {
        Self {
            backend: ArtifactOperationalBackend::Filesystem,
            root: Some("~/.chariox/artifacts".to_string()),
            index_path: Some("~/.chariox/artifacts/index.db".to_string()),
            retention_days: Some(30),
        }
    }
}

impl UserOperationalArtifactsConfig {
    fn validate(&self) -> Result<(), DaemonError> {
        if let Some(root) = &self.root {
            validate_non_empty("artifacts.operational.root", root)?;
        }
        if let Some(index_path) = &self.index_path {
            validate_non_empty("artifacts.operational.index_path", index_path)?;
        }
        validate_optional_nonzero("artifacts.operational.retention_days", self.retention_days)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactOperationalBackend {
    Filesystem,
}

impl Default for ArtifactOperationalBackend {
    fn default() -> Self {
        Self::Filesystem
    }
}

impl ArtifactOperationalBackend {
    pub(super) fn parse(field: &'static str, value: &str) -> Result<Self, DaemonError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "filesystem" => Ok(Self::Filesystem),
            _ => Err(DaemonError::InvalidConfig {
                field,
                message: "value must be `filesystem`",
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserArchiveArtifactsConfig {
    #[serde(default)]
    pub mode: HistoryArchiveMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_durable_acceptance: Option<bool>,
}

impl Default for UserArchiveArtifactsConfig {
    fn default() -> Self {
        Self {
            mode: HistoryArchiveMode::Disabled,
            url: None,
            token_env: None,
            require_durable_acceptance: Some(true),
        }
    }
}

impl UserArchiveArtifactsConfig {
    fn validate(&self) -> Result<(), DaemonError> {
        match self.mode {
            HistoryArchiveMode::Disabled => Ok(()),
            HistoryArchiveMode::External => {
                let Some(url) = self.url.as_deref() else {
                    return Err(DaemonError::InvalidConfig {
                        field: "artifacts.archive.url",
                        message: "value must be set when artifact archive mode is external",
                    });
                };
                validate_non_empty("artifacts.archive.url", url)?;
                if let Some(token_env) = &self.token_env {
                    validate_non_empty("artifacts.archive.token_env", token_env)?;
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserStateConfig {
    #[serde(default)]
    pub backend: StateBackend,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_interval_events: Option<u32>,
}

impl Default for UserStateConfig {
    fn default() -> Self {
        Self {
            backend: StateBackend::Sqlite,
            path: Some("~/.chariox/state/kernel.db".to_string()),
            snapshot_interval_events: Some(1_000),
        }
    }
}

impl UserStateConfig {
    pub(super) fn validate(&self) -> Result<(), DaemonError> {
        if let Some(path) = &self.path {
            validate_non_empty("state.path", path)?;
        }
        validate_optional_nonzero(
            "state.snapshot_interval_events",
            self.snapshot_interval_events,
        )?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateBackend {
    Sqlite,
}

impl Default for StateBackend {
    fn default() -> Self {
        Self::Sqlite
    }
}

impl StateBackend {
    pub(super) fn parse(field: &'static str, value: &str) -> Result<Self, DaemonError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "sqlite" => Ok(Self::Sqlite),
            _ => Err(DaemonError::InvalidConfig {
                field,
                message: "value must be `sqlite`",
            }),
        }
    }
}
