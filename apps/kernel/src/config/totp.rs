use serde::{Deserialize, Serialize};

use crate::error::DaemonError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TotpAlgorithm {
    #[default]
    Sha1,
    Sha256,
    Sha512,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TotpConfig {
    #[serde(default)]
    pub algorithm: TotpAlgorithm,
    #[serde(default = "default_digits")]
    pub digits: u8,
    #[serde(default = "default_period")]
    pub period_s: u32,
}

impl Default for TotpConfig {
    fn default() -> Self {
        Self {
            algorithm: TotpAlgorithm::default(),
            digits: default_digits(),
            period_s: default_period(),
        }
    }
}

impl TotpConfig {
    pub(crate) fn validate(&self) -> Result<(), DaemonError> {
        if !matches!(self.digits, 6 | 8) || !(1..=3600).contains(&self.period_s) {
            return Err(DaemonError::InvalidConfig {
                field: "credentials.injection.totp",
                message: "TOTP requires 6 or 8 digits and a period_s between 1 and 3600",
            });
        }
        Ok(())
    }
}

fn default_digits() -> u8 {
    6
}

fn default_period() -> u32 {
    30
}
