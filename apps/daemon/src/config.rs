use std::env;

use crate::error::DaemonError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonConfig {
    pub daemon_id: String,
    pub host_machine_id: String,
    pub os_user: String,
}

impl DaemonConfig {
    pub fn load_from_env() -> Self {
        Self {
            daemon_id: env::var("ARROBA_DAEMON_ID").unwrap_or_else(|_| "daemon-local".to_string()),
            host_machine_id: env::var("ARROBA_MACHINE_ID")
                .unwrap_or_else(|_| "machine-local".to_string()),
            os_user: env::var("USER")
                .or_else(|_| env::var("USERNAME"))
                .unwrap_or_else(|_| "unknown".to_string()),
        }
    }

    pub fn new(
        daemon_id: impl Into<String>,
        host_machine_id: impl Into<String>,
        os_user: impl Into<String>,
    ) -> Self {
        Self {
            daemon_id: daemon_id.into(),
            host_machine_id: host_machine_id.into(),
            os_user: os_user.into(),
        }
    }

    pub fn for_tests() -> Self {
        Self::new("daemon-test", "machine-test", "tester")
    }

    pub fn validate(&self) -> Result<(), DaemonError> {
        validate_non_empty("daemon_id", &self.daemon_id)?;
        validate_non_empty("host_machine_id", &self.host_machine_id)?;
        validate_non_empty("os_user", &self.os_user)?;
        Ok(())
    }
}

fn validate_non_empty(field: &'static str, value: &str) -> Result<(), DaemonError> {
    if value.trim().is_empty() {
        return Err(DaemonError::InvalidConfig {
            field,
            message: "value must not be empty",
        });
    }

    Ok(())
}
