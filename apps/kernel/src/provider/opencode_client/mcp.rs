//! OpenCode MCP server endpoint operations.

use std::time::{Duration, Instant};

use crate::error::DaemonError;

use super::OpenCodeClient;

impl OpenCodeClient {
    pub fn add_mcp_server(&self, name: &str, config: serde_json::Value) -> Result<(), DaemonError> {
        let _: serde_json::Value = self.send_json_request(
            "POST",
            "/mcp",
            Some(&serde_json::json!({ "name": name, "config": config })),
        )?;
        Ok(())
    }

    pub fn connect_mcp_server(&self, name: &str) -> Result<(), DaemonError> {
        let _: bool = self.send_json_request("POST", &format!("/mcp/{name}/connect"), None)?;
        Ok(())
    }

    pub fn connect_mcp_server_with_retry(
        &self,
        name: &str,
        timeout: Duration,
        retry_interval: Duration,
    ) -> Result<(), DaemonError> {
        let deadline = Instant::now() + timeout;
        let mut last_error = None;

        loop {
            match self.connect_mcp_server(name) {
                Ok(()) => return Ok(()),
                Err(error) if Instant::now() < deadline => {
                    last_error = Some(error);
                    std::thread::sleep(retry_interval);
                }
                Err(error) => return Err(last_error.unwrap_or(error)),
            }
        }
    }
}
