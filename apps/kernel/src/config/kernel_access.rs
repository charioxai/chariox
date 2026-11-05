use serde::{Deserialize, Serialize};

use crate::error::DaemonError;
use crate::local::KernelAccessProviderHarness;
use std::path::Path;

/// External-agent access grants last at most 24 hours.
const GRANT_MAX_MINUTES: u32 = 1440;

/// Lifetime policy shared by access requests, grants and extension prompts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UserKernelAccessConfig {
    pub grant_default_minutes: u32,
    pub grant_max_minutes: u32,
    pub grant_extend_notice_minutes: u32,
    pub request_timeout_minutes: u32,
    /// Accepts the hand-edited `standing_holders` list so the file loads. The list never lives
    /// in memory or in a client response: `read_standing_holders` reads it from the file and
    /// `persist_user_config` carries it through every other config write.
    #[serde(skip_serializing)]
    standing_holders: FileOwnedList,
}

/// Placeholder for a config key that the file owns.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct FileOwnedList;

impl<'de> Deserialize<'de> for FileOwnedList {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        serde::de::IgnoredAny::deserialize(deserializer).map(|_| Self)
    }
}

/// A holder that an owner pre-approved for `until-exit` access: the agent harness and the
/// exact executable path of its process.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StandingHolder {
    pub harness: KernelAccessProviderHarness,
    pub executable: String,
}

/// Reads `[[kernel_access.standing_holders]]` from the user config file on every call, so adding
/// or removing an entry needs no restart. An unreadable file or a malformed entry grants nothing.
pub(crate) fn read_standing_holders(path: &Path) -> Vec<StandingHolder> {
    let entries = standing_holder_entries(path);
    let total = entries.len();
    let holders: Vec<StandingHolder> = entries
        .into_iter()
        .filter_map(|entry| entry.try_into().ok())
        .collect();
    if holders.len() != total {
        crate::logging::warn_with_fields(
            "config.kernel_access",
            "ignored malformed kernel_access.standing_holders entries; each needs harness (codex, claude or opencode) and executable",
            serde_json::json!({ "ignored": total - holders.len(), "user_config_path": path.display().to_string() }),
        );
    }
    holders
}

/// The raw `[[kernel_access.standing_holders]]` entries of the file, empty when it is missing or
/// does not parse.
pub(super) fn standing_holder_entries(path: &Path) -> Vec<toml::Value> {
    let Ok(payload) = super::read_user_config(&path.to_path_buf()) else {
        return Vec::new();
    };
    toml::from_str::<toml::Table>(&payload)
        .ok()
        .and_then(
            |table| match table.get("kernel_access")?.get("standing_holders")? {
                toml::Value::Array(entries) => Some(entries.clone()),
                _ => None,
            },
        )
        .unwrap_or_default()
}

impl Default for UserKernelAccessConfig {
    fn default() -> Self {
        Self {
            grant_default_minutes: 480,
            grant_max_minutes: GRANT_MAX_MINUTES,
            grant_extend_notice_minutes: 5,
            request_timeout_minutes: 10,
            standing_holders: FileOwnedList,
        }
    }
}

impl UserKernelAccessConfig {
    /// Configs written before the 8 h default and 24 h maximum stay bootable:
    /// out-of-range terms are clamped and reported instead of refused.
    pub(super) fn clamp_legacy(&mut self) -> Option<String> {
        let before = self.clone();
        self.grant_max_minutes = self.grant_max_minutes.min(GRANT_MAX_MINUTES);
        self.grant_default_minutes = self.grant_default_minutes.min(self.grant_max_minutes);
        if self.grant_default_minutes > 1 {
            self.grant_extend_notice_minutes = self
                .grant_extend_notice_minutes
                .min(self.grant_default_minutes - 1);
        }
        (*self != before).then(|| {
            format!(
                "kernel_access grant terms clamped to the current policy: default {} -> {}, maximum {} -> {}, extend notice {} -> {} minutes",
                before.grant_default_minutes,
                self.grant_default_minutes,
                before.grant_max_minutes,
                self.grant_max_minutes,
                before.grant_extend_notice_minutes,
                self.grant_extend_notice_minutes
            )
        })
    }

    pub(super) fn validate(&self) -> Result<(), DaemonError> {
        for (field, value) in [
            (
                "kernel_access.grant_default_minutes",
                self.grant_default_minutes,
            ),
            ("kernel_access.grant_max_minutes", self.grant_max_minutes),
            (
                "kernel_access.grant_extend_notice_minutes",
                self.grant_extend_notice_minutes,
            ),
            (
                "kernel_access.request_timeout_minutes",
                self.request_timeout_minutes,
            ),
        ] {
            if value == 0 {
                return Err(DaemonError::InvalidConfig {
                    field,
                    message: "value must not be zero",
                });
            }
        }
        if self.grant_max_minutes > GRANT_MAX_MINUTES {
            return Err(DaemonError::InvalidConfig {
                field: "kernel_access.grant_max_minutes",
                message: "value must not exceed 24 hours (1440 minutes)",
            });
        }
        if self.grant_default_minutes > self.grant_max_minutes {
            return Err(DaemonError::InvalidConfig {
                field: "kernel_access.grant_default_minutes",
                message: "value must be less than or equal to kernel_access.grant_max_minutes",
            });
        }
        if self.grant_extend_notice_minutes >= self.grant_default_minutes {
            return Err(DaemonError::InvalidConfig {
                field: "kernel_access.grant_extend_notice_minutes",
                message: "value must be less than kernel_access.grant_default_minutes",
            });
        }
        Ok(())
    }
}
