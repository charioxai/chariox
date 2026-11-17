//! OpenCode health probing and retry policy.

use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::error::DaemonError;

use super::OpenCodeClient;

#[derive(Debug, Deserialize)]
struct OpenCodeHealth {
    healthy: bool,
    #[serde(default)]
    version: Option<String>,
}

impl OpenCodeClient {
    pub fn wait_until_healthy(&self, timeout: Duration) -> Result<(), DaemonError> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.health() {
                Ok(()) => return Ok(()),
                Err(error) if Instant::now() < deadline => {
                    let _ = error;
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// MP-08 / MP-10 / MP-11: /global/health identifies the running official server.
    pub(crate) fn native_skill_server_version(&self) -> Result<String, DaemonError> {
        let health: OpenCodeHealth = self.send_json_request("GET", "/global/health", None)?;
        health
            .version
            .filter(|value| {
                health.healthy
                    && !value.trim().is_empty()
                    && value.len() <= 128
                    && !value.chars().any(char::is_control)
            })
            .ok_or_else(|| {
                self.protocol_error(
                    "opencode_skill_inventory_version",
                    "server did not report a valid version identity".into(),
                )
            })
    }
    pub fn check_health(&self) -> Result<(), DaemonError> {
        self.health()
    }

    fn health(&self) -> Result<(), DaemonError> {
        let health: OpenCodeHealth = self.send_json_request("GET", "/global/health", None)?;
        if health.healthy {
            Ok(())
        } else {
            Err(self.protocol_error("health", "provider reported unhealthy".to_string()))
        }
    }
}
