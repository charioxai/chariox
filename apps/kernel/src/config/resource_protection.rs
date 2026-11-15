//! Machine-local permission for last-resort workload intervention.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ResourceProtection {
    pub memory: bool,
    pub disk: bool,
    pub cpu: bool,
}

impl Default for ResourceProtection {
    fn default() -> Self {
        Self {
            memory: true,
            disk: true,
            cpu: true,
        }
    }
}
