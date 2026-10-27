//! OpenCode health probing and retry policy.

use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::error::DaemonError;

use super::OpenCodeClient;

#[derive(Debug, Deserialize)]
struct OpenCodeHealth {
    healthy: bool,
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
