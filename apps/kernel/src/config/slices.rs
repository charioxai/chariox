use serde::{Deserialize, Serialize};

use super::{validate_non_empty, validate_optional_nonzero};
use crate::error::DaemonError;

pub const DEFAULT_LINUX_SLICE_DOCKER_IMAGE: &str = "chariox-slice-linux:0.1.0";
pub const DEFAULT_LOCAL_DOCKER_SLICE_MEMORY_MB: u32 = 2_048;

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
    pub disk_layer_mb: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk_home_mb: Option<u32>,
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
            memory_mb: Some(DEFAULT_LOCAL_DOCKER_SLICE_MEMORY_MB),
            cpus: None,
            disk_layer_mb: None,
            disk_home_mb: None,
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
            validate_docker_cpus(cpus)?;
        }
        validate_optional_nonzero("slices.linux.disk_layer_mb", self.disk_layer_mb)?;
        validate_optional_nonzero("slices.linux.disk_home_mb", self.disk_home_mb)?;
        Ok(())
    }
}

pub(crate) fn validate_docker_cpus(value: &str) -> Result<(), DaemonError> {
    // Shared native Docker nanocpu range and decimal precision, also consumed
    // by the broker. Reject ambiguous/zero/nonfinite values before either adapter.
    let policy: serde_json::Value = serde_json::from_str(include_str!(
        "../../slice-linux-docker/docker-cpu-policy.json"
    ))
    .expect("packaged Docker CPU policy must be valid");
    let maximum_digits = policy["maximumFractionDigits"].as_u64().unwrap() as usize;
    let maximum_nanos = policy["maximumNanoCpus"]
        .as_str()
        .unwrap()
        .parse::<u64>()
        .unwrap();
    let invalid = || DaemonError::InvalidConfig {
        field: "slices.linux.cpus",
        message: "value must be a positive Docker CPU decimal within the nanocpu range",
    };
    let (whole, fraction) = match value.split_once('.') {
        Some((whole, fraction)) if !fraction.is_empty() && fraction.len() <= maximum_digits => {
            (whole, fraction)
        }
        Some(_) => return Err(invalid()),
        None => (value, ""),
    };
    if whole.is_empty()
        || !whole
            .bytes()
            .chain(fraction.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return Err(invalid());
    }
    let whole = whole.parse::<u64>().map_err(|_| invalid())?;
    let fraction = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<u64>().map_err(|_| invalid())? * 10u64.pow((9 - fraction.len()) as u32)
    };
    let nanos = whole
        .checked_mul(1_000_000_000)
        .and_then(|n| n.checked_add(fraction))
        .ok_or_else(invalid)?;
    if nanos == 0 || nanos > maximum_nanos {
        return Err(invalid());
    }
    Ok(())
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

#[cfg(test)]
mod cpu_policy_tests {
    use super::*;
    #[test]
    fn mp08_mp11_config_cpu_admission_uses_shared_positive_docker_cases() {
        let policy: serde_json::Value = serde_json::from_str(include_str!(
            "../../slice-linux-docker/docker-cpu-policy.json"
        ))
        .unwrap();
        for case in policy["cases"].as_array().unwrap() {
            let mut config = UserLinuxSliceConfig::default();
            config.cpus = Some(case[0].as_str().unwrap().to_string());
            assert_eq!(
                config.validate().is_ok(),
                case[1].as_bool().unwrap(),
                "{}",
                case[0]
            );
        }
    }
}
