use super::{validate_non_empty, CharioxUserConfig, DaemonConfig};
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
        if self
            .user_config
            .workflow
            .max_queues_per_workflow
            .is_some_and(|value| value == 0)
        {
            return Err(DaemonError::InvalidConfig {
                field: "workflow.max_queues_per_workflow",
                message: "value must not be zero",
            });
        }
        if self
            .user_config
            .workflow
            .session_default_max_agents
            .is_some_and(|value| value == 0)
        {
            return Err(DaemonError::InvalidConfig {
                field: "workflow.session_default_max_agents",
                message: "value must not be zero",
            });
        }
        validate_workflow_code_limits(&self.user_config.workflow)?;
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

fn validate_workflow_code_limits(
    workflow: &crate::config::UserWorkflowConfig,
) -> Result<(), DaemonError> {
    let Some(code) = workflow.code.as_ref() else {
        return Ok(());
    };
    validate_optional_nonzero("workflow.code.max_concurrent", code.max_concurrent)?;
    validate_optional_nonzero("workflow.code.max_nodes", code.max_nodes)?;
    validate_optional_nonzero("workflow.code.max_agents", code.max_agents)?;
    validate_optional_nonzero("workflow.code.max_edges", code.max_edges)?;
    validate_optional_nonzero("workflow.code.max_endpoints", code.max_endpoints)?;
    validate_optional_nonzero("workflow.code.max_queues", code.max_queues)?;
    validate_optional_nonzero("workflow.code.max_watchdogs", code.max_watchdogs)?;
    validate_optional_nonzero("workflow.code.max_schema_bytes", code.max_schema_bytes)?;
    validate_optional_nonzero(
        "workflow.code.max_generated_prompt_bytes",
        code.max_generated_prompt_bytes,
    )?;
    validate_optional_nonzero_u64("workflow.code.script_timeout_ms", code.script_timeout_ms)?;
    validate_optional_nonzero_u64(
        "workflow.code.script_memory_bytes",
        code.script_memory_bytes,
    )?;
    Ok(())
}

fn validate_optional_nonzero(field: &'static str, value: Option<u32>) -> Result<(), DaemonError> {
    if value.is_some_and(|value| value == 0) {
        return Err(DaemonError::InvalidConfig {
            field,
            message: "value must not be zero",
        });
    }
    Ok(())
}

fn validate_optional_nonzero_u64(
    field: &'static str,
    value: Option<u64>,
) -> Result<(), DaemonError> {
    if value.is_some_and(|value| value == 0) {
        return Err(DaemonError::InvalidConfig {
            field,
            message: "value must not be zero",
        });
    }
    Ok(())
}

impl CharioxUserConfig {
    pub fn validate(&self) -> Result<(), DaemonError> {
        self.providers.workspace_live_sync.validate()?;
        self.history.validate()?;
        self.artifacts.validate()?;
        self.state.validate()?;
        self.slices.validate()?;
        validate_non_empty("credential_vault.service", &self.credential_vault.service)?;
        validate_non_empty("credential_vault.path", &self.credential_vault.path)?;
        if self.credential_vault.default_ttl_minutes == 0 {
            return Err(DaemonError::InvalidConfig {
                field: "credential_vault.default_ttl_minutes",
                message: "value must not be zero",
            });
        }
        if self.credential_vault.max_ttl_minutes == 0 {
            return Err(DaemonError::InvalidConfig {
                field: "credential_vault.max_ttl_minutes",
                message: "value must not be zero",
            });
        }
        if self.credential_vault.default_ttl_minutes > self.credential_vault.max_ttl_minutes {
            return Err(DaemonError::InvalidConfig {
                field: "credential_vault.default_ttl_minutes",
                message: "value must be less than or equal to credential_vault.max_ttl_minutes",
            });
        }
        Ok(())
    }
}
