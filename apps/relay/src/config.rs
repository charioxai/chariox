use std::env;

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayConfig {
    pub host: String,
    pub port: u16,
    pub shared_token: Option<String>,
}

#[derive(Debug, Error)]
pub enum RelayConfigError {
    #[error("invalid relay configuration for `{field}`: {message}")]
    InvalidConfig {
        field: &'static str,
        message: &'static str,
    },
}

impl RelayConfig {
    pub fn load_from_env() -> Result<Self, RelayConfigError> {
        let config = Self {
            host: env::var("CHARIOX_RELAY_HOST").unwrap_or_else(|_| "127.0.0.1".to_string()),
            port: env::var("CHARIOX_RELAY_PORT")
                .ok()
                .and_then(|value| value.parse::<u16>().ok())
                .unwrap_or(43130),
            shared_token: env::var("CHARIOX_RELAY_TOKEN")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
        };
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), RelayConfigError> {
        if self.host.trim().is_empty() {
            return Err(RelayConfigError::InvalidConfig {
                field: "host",
                message: "value must not be empty",
            });
        }
        if self.port == 0 {
            return Err(RelayConfigError::InvalidConfig {
                field: "port",
                message: "value must not be zero",
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_rejects_empty_host() {
        let error = RelayConfig {
            host: String::new(),
            port: 43130,
            shared_token: None,
        }
        .validate()
        .expect_err("empty host should be rejected");
        match error {
            RelayConfigError::InvalidConfig { field, .. } => assert_eq!(field, "host"),
        }
    }
}
