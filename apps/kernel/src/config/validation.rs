use super::{validate_non_empty, ArrobaUserConfig, DaemonConfig};
use crate::error::DaemonError;

impl DaemonConfig {
    pub fn validate(&self) -> Result<(), DaemonError> {
        validate_non_empty("daemon_id", &self.daemon_id)?;
        validate_non_empty("host_machine_id", &self.host_machine_id)?;
        if self
            .relay_url
            .as_deref()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
            && self.relay_token.is_none()
        {
            return Err(DaemonError::InvalidConfig {
                field: "relay_token",
                message: "value must be set when relay_url is configured",
            });
        }
        validate_non_empty("os_user", &self.os_user)?;
        self.user_config.validate()?;
        if self.local_socket_path.as_os_str().is_empty() {
            return Err(DaemonError::InvalidConfig {
                field: "local_socket_path",
                message: "value must not be empty",
            });
        }
        validate_non_empty("os_name", &self.os_name)?;
        validate_non_empty("kernel_websocket_host", &self.kernel_websocket_host)?;
        if self.kernel_websocket_port == 0 {
            return Err(DaemonError::InvalidConfig {
                field: "kernel_websocket_port",
                message: "value must not be zero",
            });
        }
        if self.kernel_websocket_queue_capacity == 0 {
            return Err(DaemonError::InvalidConfig {
                field: "kernel_websocket_queue_capacity",
                message: "value must not be zero",
            });
        }
        validate_non_empty("runtime_mcp_host", &self.runtime_mcp_host)?;
        if self.runtime_mcp_port == 0 {
            return Err(DaemonError::InvalidConfig {
                field: "runtime_mcp_port",
                message: "value must not be zero",
            });
        }
        if self.session_history_root.as_os_str().is_empty() {
            return Err(DaemonError::InvalidConfig {
                field: "session_history_root",
                message: "value must not be empty",
            });
        }
        validate_non_empty("relay_public_key", &self.relay_public_key)?;
        validate_non_empty("relay_private_key", &self.relay_private_key)?;
        if self.relay_heartbeat_ms == 0 {
            return Err(DaemonError::InvalidConfig {
                field: "relay_heartbeat_ms",
                message: "value must not be zero",
            });
        }
        if self.relay_request_timeout_ms == 0 {
            return Err(DaemonError::InvalidConfig {
                field: "relay_request_timeout_ms",
                message: "value must not be zero",
            });
        }
        Ok(())
    }
}

impl ArrobaUserConfig {
    pub fn validate(&self) -> Result<(), DaemonError> {
        self.providers.managed_io.validate()?;
        self.history.validate()?;
        self.artifacts.validate()?;
        self.state.validate()?;
        self.slices.validate()?;
        validate_non_empty("credential_vault.service", &self.credential_vault.service)?;
        Ok(())
    }
}
