//! OpenCode MCP server endpoint operations.

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
}
