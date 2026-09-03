use serde::{Deserialize, Serialize};

use super::{validate_non_empty, validate_optional_nonzero};
use crate::error::DaemonError;

pub const DEFAULT_LINUX_SLICE_DOCKER_IMAGE: &str = "chariox-slice-linux:0.1.0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserSlicesConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    #[serde(default)]
    pub linux: UserLinuxSliceConfig,
}

impl Default for UserSlicesConfig {
    fn default() -> Self {
        Self {
            root: Some("~/.chariox/slices".to_string()),
            linux: UserLinuxSliceConfig::default(),
        }
    }
}

impl UserSlicesConfig {
    pub(super) fn validate(&self) -> Result<(), DaemonError> {
        if let Some(root) = &self.root {
            validate_non_empty("slices.root", root)?;
        }
        self.linux.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserLinuxSliceConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docker_image: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_image: Option<SliceImageBuildPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension_dockerfile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_unconfined_seccomp: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_provider_sandbox_compatibility: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_mb: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpus: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_timeout_minutes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen_width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen_height: Option<u32>,
}

impl Default for UserLinuxSliceConfig {
    fn default() -> Self {
        Self {
            docker_image: Some(DEFAULT_LINUX_SLICE_DOCKER_IMAGE.to_string()),
            build_image: Some(SliceImageBuildPolicy::Auto),
            extension_dockerfile: None,
            allow_unconfined_seccomp: Some(false),
            allow_provider_sandbox_compatibility: Some(false),
            memory_mb: None,
            cpus: None,
            idle_timeout_minutes: Some(30),
            screen_width: Some(1280),
            screen_height: Some(800),
        }
    }
}

impl UserLinuxSliceConfig {
    fn validate(&self) -> Result<(), DaemonError> {
        if let Some(image) = &self.docker_image {
            validate_non_empty("slices.linux.docker_image", image)?;
        }
        if let Some(path) = &self.extension_dockerfile {
            validate_non_empty("slices.linux.extension_dockerfile", path)?;
        }
        validate_optional_nonzero("slices.linux.memory_mb", self.memory_mb)?;
        validate_optional_nonzero(
            "slices.linux.idle_timeout_minutes",
            self.idle_timeout_minutes,
        )?;
        validate_optional_nonzero("slices.linux.screen_width", self.screen_width)?;
        validate_optional_nonzero("slices.linux.screen_height", self.screen_height)?;
        if let Some(cpus) = &self.cpus {
            validate_non_empty("slices.linux.cpus", cpus)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SliceImageBuildPolicy {
    Auto,
    Always,
    Never,
}

impl SliceImageBuildPolicy {
    pub(super) fn parse(value: &str) -> Result<Self, DaemonError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "always" => Ok(Self::Always),
            "never" | "off" | "false" | "0" => Ok(Self::Never),
            _ => Err(DaemonError::InvalidConfig {
                field: "slices.linux.build_image",
                message: "value must be `auto`, `always`, or `never`",
            }),
        }
    }

    pub fn as_env_value(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Always => "always",
            Self::Never => "never",
        }
    }
}
