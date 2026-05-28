use super::{
    normalized_optional, validate_config_key_path, validate_non_empty, ArrobaUserConfig,
    ArtifactOperationalBackend, HistoryArchiveMode, HistoryOperationalBackend,
    SliceImageBuildPolicy, StateBackend, WorkspaceLiveSyncConfig, WorkspaceLiveSyncMode,
};
use crate::error::DaemonError;

impl ArrobaUserConfig {
    pub(super) fn set_value(&mut self, key_path: &str, value: String) -> Result<(), DaemonError> {
        let normalized = key_path.trim();
        validate_config_key_path(normalized)?;
        match normalized {
            "version" => {
                self.version = value
                    .parse::<u32>()
                    .map_err(|_| DaemonError::InvalidConfig {
                        field: "version",
                        message: "value must be an unsigned integer",
                    })?;
            }
            "providers.default" => {
                self.providers.default = Some(non_empty_config_string("providers.default", value)?)
            }
            "providers.model" => {
                self.providers.model = Some(non_empty_config_string("providers.model", value)?)
            }
            "providers.account_profile" => {
                self.providers.account_profile =
                    Some(non_empty_config_string("providers.account_profile", value)?)
            }
            "providers.effort" => {
                self.providers.effort = Some(non_empty_config_string("providers.effort", value)?)
            }
            "ui.theme" => self.ui.theme = Some(non_empty_config_string("ui.theme", value)?),
            "ui.multi_agent_response_layout" => {
                self.ui.multi_agent_response_layout = Some(non_empty_config_string(
                    "ui.multi_agent_response_layout",
                    value,
                )?)
            }
            "ui.max_agents_per_screen" => {
                self.ui.max_agents_per_screen =
                    Some(
                        value
                            .parse::<u32>()
                            .map_err(|_| DaemonError::InvalidConfig {
                                field: "ui.max_agents_per_screen",
                                message: "value must be an unsigned integer",
                            })?,
                    );
            }
            path if path.starts_with("ui.worktree_aliases.") => {
                let key = path.trim_start_matches("ui.worktree_aliases.").trim();
                validate_config_key_path(&format!("ui.worktree_aliases.{key}"))?;
                self.ui.worktree_aliases.insert(
                    key.to_string(),
                    non_empty_config_string("ui.worktree_aliases", value)?,
                );
            }
            "relay.url" => self.relay.url = normalized_optional(Some(value)),
            "relay.accept_remote_leases" => {
                self.relay.accept_remote_leases =
                    Some(parse_config_bool("relay.accept_remote_leases", &value)?)
            }
            "history.operational.backend" => {
                self.history.operational.backend =
                    HistoryOperationalBackend::parse("history.operational.backend", &value)?
            }
            "history.operational.path" => {
                self.history.operational.path =
                    Some(non_empty_config_string("history.operational.path", value)?)
            }
            "history.operational.retention_days" => {
                self.history.operational.retention_days = Some(parse_config_u32(
                    "history.operational.retention_days",
                    &value,
                    true,
                )?)
            }
            "history.operational.max_size_mb" => {
                self.history.operational.max_size_mb = Some(parse_config_u32(
                    "history.operational.max_size_mb",
                    &value,
                    true,
                )?)
            }
            "history.operational.keep_pinned_sessions" => {
                self.history.operational.keep_pinned_sessions = Some(parse_config_bool(
                    "history.operational.keep_pinned_sessions",
                    &value,
                )?)
            }
            "history.operational.archive_inactive_after_days" => {
                self.history.operational.archive_inactive_after_days = Some(parse_config_u32(
                    "history.operational.archive_inactive_after_days",
                    &value,
                    true,
                )?)
            }
            "history.operational.archive_deleted_agents" => {
                self.history.operational.archive_deleted_agents = Some(parse_config_bool(
                    "history.operational.archive_deleted_agents",
                    &value,
                )?)
            }
            "history.archive.mode" => {
                self.history.archive.mode = HistoryArchiveMode::parse(&value)?
            }
            "history.archive.url" => {
                self.history.archive.url =
                    Some(non_empty_config_string("history.archive.url", value)?)
            }
            "history.archive.token_env" => {
                self.history.archive.token_env =
                    Some(non_empty_config_string("history.archive.token_env", value)?)
            }
            "history.archive.archive_deleted_agents" => {
                self.history.archive.archive_deleted_agents = Some(parse_config_bool(
                    "history.archive.archive_deleted_agents",
                    &value,
                )?)
            }
            "history.archive.archive_before_delete" => {
                self.history.archive.archive_before_delete = Some(parse_config_bool(
                    "history.archive.archive_before_delete",
                    &value,
                )?)
            }
            "history.archive.delete_operational_after_verified_archive" => {
                self.history
                    .archive
                    .delete_operational_after_verified_archive = Some(parse_config_bool(
                    "history.archive.delete_operational_after_verified_archive",
                    &value,
                )?)
            }
            "history.archive.require_durable_acceptance" => {
                self.history.archive.require_durable_acceptance = Some(parse_config_bool(
                    "history.archive.require_durable_acceptance",
                    &value,
                )?)
            }
            "artifacts.operational.backend" => {
                self.artifacts.operational.backend =
                    ArtifactOperationalBackend::parse("artifacts.operational.backend", &value)?
            }
            "artifacts.operational.root" => {
                self.artifacts.operational.root = Some(non_empty_config_string(
                    "artifacts.operational.root",
                    value,
                )?)
            }
            "artifacts.operational.index_path" => {
                self.artifacts.operational.index_path = Some(non_empty_config_string(
                    "artifacts.operational.index_path",
                    value,
                )?)
            }
            "artifacts.operational.retention_days" => {
                self.artifacts.operational.retention_days = Some(parse_config_u32(
                    "artifacts.operational.retention_days",
                    &value,
                    true,
                )?)
            }
            "artifacts.archive.mode" => {
                self.artifacts.archive.mode = HistoryArchiveMode::parse(&value)?
            }
            "artifacts.archive.url" => {
                self.artifacts.archive.url =
                    Some(non_empty_config_string("artifacts.archive.url", value)?)
            }
            "artifacts.archive.token_env" => {
                self.artifacts.archive.token_env = Some(non_empty_config_string(
                    "artifacts.archive.token_env",
                    value,
                )?)
            }
            "artifacts.archive.require_durable_acceptance" => {
                self.artifacts.archive.require_durable_acceptance = Some(parse_config_bool(
                    "artifacts.archive.require_durable_acceptance",
                    &value,
                )?)
            }
            "state.backend" => self.state.backend = StateBackend::parse("state.backend", &value)?,
            "state.path" => self.state.path = Some(non_empty_config_string("state.path", value)?),
            "state.snapshot_interval_events" => {
                self.state.snapshot_interval_events = Some(parse_config_u32(
                    "state.snapshot_interval_events",
                    &value,
                    true,
                )?)
            }
            "slices.root" => {
                self.slices.root = Some(non_empty_config_string("slices.root", value)?)
            }
            "slices.linux.docker_image" => {
                self.slices.linux.docker_image =
                    Some(non_empty_config_string("slices.linux.docker_image", value)?)
            }
            "slices.linux.build_image" => {
                self.slices.linux.build_image = Some(SliceImageBuildPolicy::parse(&value)?)
            }
            "slices.linux.extension_dockerfile" => {
                self.slices.linux.extension_dockerfile = Some(non_empty_config_string(
                    "slices.linux.extension_dockerfile",
                    value,
                )?)
            }
            "slices.linux.memory_mb" => {
                self.slices.linux.memory_mb =
                    Some(parse_config_u32("slices.linux.memory_mb", &value, true)?)
            }
            "slices.linux.cpus" => {
                self.slices.linux.cpus = Some(non_empty_config_string("slices.linux.cpus", value)?)
            }
            "slices.linux.idle_timeout_minutes" => {
                self.slices.linux.idle_timeout_minutes = Some(parse_config_u32(
                    "slices.linux.idle_timeout_minutes",
                    &value,
                    true,
                )?)
            }
            "slices.linux.screen_width" => {
                self.slices.linux.screen_width =
                    Some(parse_config_u32("slices.linux.screen_width", &value, true)?)
            }
            "slices.linux.screen_height" => {
                self.slices.linux.screen_height = Some(parse_config_u32(
                    "slices.linux.screen_height",
                    &value,
                    true,
                )?)
            }
            "kernel.websocket_host" => {
                self.kernel.websocket_host =
                    Some(non_empty_config_string("kernel.websocket_host", value)?)
            }
            "kernel.websocket_port" => {
                self.kernel.websocket_port =
                    Some(parse_config_port("kernel.websocket_port", &value)?)
            }
            "kernel.runtime_mcp_host" => {
                self.kernel.runtime_mcp_host =
                    Some(non_empty_config_string("kernel.runtime_mcp_host", value)?)
            }
            "kernel.runtime_mcp_port" => {
                self.kernel.runtime_mcp_port =
                    Some(parse_config_port("kernel.runtime_mcp_port", &value)?)
            }
            "workflow.max_queues_per_workflow" => {
                self.workflow.max_queues_per_workflow = Some(parse_config_u32(
                    "workflow.max_queues_per_workflow",
                    &value,
                    true,
                )?)
            }
            "credential_vault.service" => {
                self.credential_vault.service =
                    non_empty_config_string("credential_vault.service", value)?
            }
            "providers.workspace_live_sync" => {
                self.providers.workspace_live_sync =
                    WorkspaceLiveSyncConfig::from_mode(
                        WorkspaceLiveSyncMode::parse_config_policy(&value)?,
                    );
            }
            _ => {
                return Err(DaemonError::InvalidConfig {
                    field: "user_config",
                    message: "unsupported user config key",
                });
            }
        }
        self.validate()
    }

    pub(super) fn unset_value(&mut self, key_path: &str) -> Result<(), DaemonError> {
        let normalized = key_path.trim();
        validate_config_key_path(normalized)?;
        match normalized {
            "providers.default" => self.providers.default = None,
            "providers.model" => self.providers.model = None,
            "providers.account_profile" => self.providers.account_profile = None,
            "providers.effort" => self.providers.effort = None,
            "ui.theme" => self.ui.theme = None,
            "ui.multi_agent_response_layout" => self.ui.multi_agent_response_layout = None,
            "ui.max_agents_per_screen" => self.ui.max_agents_per_screen = None,
            path if path.starts_with("ui.worktree_aliases.") => {
                let key = path.trim_start_matches("ui.worktree_aliases.").trim();
                validate_config_key_path(&format!("ui.worktree_aliases.{key}"))?;
                self.ui.worktree_aliases.remove(key);
            }
            "relay.url" => self.relay.url = None,
            "relay.accept_remote_leases" => self.relay.accept_remote_leases = None,
            "history.operational.backend" => {
                return Err(DaemonError::InvalidConfig {
                    field: "history.operational.backend",
                    message: "operational history backend cannot be unset",
                });
            }
            "history.operational.path" => self.history.operational.path = None,
            "history.operational.retention_days" => self.history.operational.retention_days = None,
            "history.operational.max_size_mb" => self.history.operational.max_size_mb = None,
            "history.operational.keep_pinned_sessions" => {
                self.history.operational.keep_pinned_sessions = None
            }
            "history.operational.archive_inactive_after_days" => {
                self.history.operational.archive_inactive_after_days = None
            }
            "history.operational.archive_deleted_agents" => {
                self.history.operational.archive_deleted_agents = None
            }
            "history.archive.mode" => self.history.archive.mode = HistoryArchiveMode::Disabled,
            "history.archive.url" => self.history.archive.url = None,
            "history.archive.token_env" => self.history.archive.token_env = None,
            "history.archive.archive_deleted_agents" => {
                self.history.archive.archive_deleted_agents = None
            }
            "history.archive.archive_before_delete" => {
                self.history.archive.archive_before_delete = None
            }
            "history.archive.delete_operational_after_verified_archive" => {
                self.history
                    .archive
                    .delete_operational_after_verified_archive = None
            }
            "history.archive.require_durable_acceptance" => {
                self.history.archive.require_durable_acceptance = None
            }
            "artifacts.operational.backend" => {
                return Err(DaemonError::InvalidConfig {
                    field: "artifacts.operational.backend",
                    message: "operational artifact backend cannot be unset",
                });
            }
            "artifacts.operational.root" => self.artifacts.operational.root = None,
            "artifacts.operational.index_path" => self.artifacts.operational.index_path = None,
            "artifacts.operational.retention_days" => {
                self.artifacts.operational.retention_days = None
            }
            "artifacts.archive.mode" => self.artifacts.archive.mode = HistoryArchiveMode::Disabled,
            "artifacts.archive.url" => self.artifacts.archive.url = None,
            "artifacts.archive.token_env" => self.artifacts.archive.token_env = None,
            "artifacts.archive.require_durable_acceptance" => {
                self.artifacts.archive.require_durable_acceptance = None
            }
            "state.backend" => {
                return Err(DaemonError::InvalidConfig {
                    field: "state.backend",
                    message: "state backend cannot be unset",
                });
            }
            "state.path" => self.state.path = None,
            "state.snapshot_interval_events" => self.state.snapshot_interval_events = None,
            "slices.root" => self.slices.root = None,
            "slices.linux.docker_image" => self.slices.linux.docker_image = None,
            "slices.linux.build_image" => self.slices.linux.build_image = None,
            "slices.linux.extension_dockerfile" => self.slices.linux.extension_dockerfile = None,
            "slices.linux.memory_mb" => self.slices.linux.memory_mb = None,
            "slices.linux.cpus" => self.slices.linux.cpus = None,
            "slices.linux.idle_timeout_minutes" => self.slices.linux.idle_timeout_minutes = None,
            "slices.linux.screen_width" => self.slices.linux.screen_width = None,
            "slices.linux.screen_height" => self.slices.linux.screen_height = None,
            "kernel.websocket_host" => self.kernel.websocket_host = None,
            "kernel.websocket_port" => self.kernel.websocket_port = None,
            "kernel.runtime_mcp_host" => self.kernel.runtime_mcp_host = None,
            "kernel.runtime_mcp_port" => self.kernel.runtime_mcp_port = None,
            "workflow.max_queues_per_workflow" => self.workflow.max_queues_per_workflow = None,
            "providers.workspace_live_sync" => {
                self.providers.workspace_live_sync = WorkspaceLiveSyncConfig::default()
            }
            "version" => {
                return Err(DaemonError::InvalidConfig {
                    field: "version",
                    message: "version cannot be unset",
                });
            }
            _ => {
                return Err(DaemonError::InvalidConfig {
                    field: "user_config",
                    message: "unsupported user config key",
                });
            }
        }
        self.validate()
    }
}

fn non_empty_config_string(field: &'static str, value: String) -> Result<String, DaemonError> {
    let value = value.trim().to_string();
    validate_non_empty(field, &value)?;
    Ok(value)
}

fn parse_config_bool(field: &'static str, value: &str) -> Result<bool, DaemonError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(DaemonError::InvalidConfig {
            field,
            message: "value must be a boolean",
        }),
    }
}

fn parse_config_port(field: &'static str, value: &str) -> Result<u16, DaemonError> {
    let port = value
        .parse::<u16>()
        .map_err(|_| DaemonError::InvalidConfig {
            field,
            message: "value must be a TCP port",
        })?;
    if port == 0 {
        return Err(DaemonError::InvalidConfig {
            field,
            message: "value must not be zero",
        });
    }
    Ok(port)
}

fn parse_config_u32(
    field: &'static str,
    value: &str,
    require_nonzero: bool,
) -> Result<u32, DaemonError> {
    let parsed = value
        .parse::<u32>()
        .map_err(|_| DaemonError::InvalidConfig {
            field,
            message: "value must be an unsigned integer",
        })?;
    if require_nonzero && parsed == 0 {
        return Err(DaemonError::InvalidConfig {
            field,
            message: "value must not be zero",
        });
    }
    Ok(parsed)
}
