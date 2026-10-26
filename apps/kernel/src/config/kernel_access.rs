use serde::{Deserialize, Serialize};

use crate::error::DaemonError;

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
}

impl Default for UserKernelAccessConfig {
    fn default() -> Self {
        Self {
            grant_default_minutes: 480,
            grant_max_minutes: GRANT_MAX_MINUTES,
            grant_extend_notice_minutes: 5,
            request_timeout_minutes: 10,
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
