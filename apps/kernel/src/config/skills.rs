//! MP-08 / MP-10 / MP-11: one metadata budget for every official harness.
use crate::error::DaemonError;
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(transparent)]
pub struct SkillMetadataBudgetPercent(f64);
// All construction/deserialization rejects NaN; equality is reflexive.
impl Eq for SkillMetadataBudgetPercent {}
impl Default for SkillMetadataBudgetPercent {
    fn default() -> Self {
        Self(1.0)
    }
}
impl SkillMetadataBudgetPercent {
    pub fn new(value: f64) -> Result<Self, DaemonError> {
        if !value.is_finite() || value <= 0.0 || value > 100.0 {
            return Err(DaemonError::InvalidConfig {
                field: "skills.metadata_budget_percent",
                message: "must be finite, greater than 0 and at most 100",
            });
        }
        Ok(Self(value))
    }
    pub fn get(self) -> f64 {
        self.0
    }
}
impl<'de> Deserialize<'de> for SkillMetadataBudgetPercent {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = f64::deserialize(deserializer)?;
        Self::new(value).map_err(|_| {
            serde::de::Error::custom(
                "skills.metadata_budget_percent must be finite, greater than 0 and at most 100",
            )
        })
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserSkillsConfig {
    #[serde(default)]
    pub metadata_budget_percent: SkillMetadataBudgetPercent,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn skills_v2_budget_defaults_to_one_and_rejects_invalid_config() {
        assert_eq!(
            UserSkillsConfig::default().metadata_budget_percent.get(),
            1.0
        );
        let config: crate::config::CharioxUserConfig =
            toml::from_str("[skills]\nmetadata_budget_percent = 2.5\n").unwrap();
        assert_eq!(config.skills.metadata_budget_percent.get(), 2.5);
        let mut settings = crate::config::CharioxUserConfig::default();
        settings
            .set_value("skills.metadata_budget_percent", "2.5".into())
            .unwrap();
        assert_eq!(settings.skills.metadata_budget_percent.get(), 2.5);
        assert!(settings
            .set_value("skills.metadata_budget_percent", "NaN".into())
            .is_err());
        assert_eq!(settings.skills.metadata_budget_percent.get(), 2.5);
        settings
            .unset_value("skills.metadata_budget_percent")
            .unwrap();
        assert_eq!(settings.skills.metadata_budget_percent.get(), 1.0);
        for invalid in ["0", "-1", "101", "nan", "inf", "\"1.0\"", "true"] {
            assert!(toml::from_str::<crate::config::CharioxUserConfig>(&format!(
                "[skills]\nmetadata_budget_percent = {invalid}\n"
            ))
            .is_err());
        }
    }
}
