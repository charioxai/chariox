use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserConfigSchemaEntry {
    pub path: String,
    pub value_type: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_values: Vec<String>,
    pub settable: bool,
    pub unsettable: bool,
    pub effect: String,
    pub status: String,
    pub description: String,
}

fn entry(
    path: &str,
    value_type: &str,
    allowed_values: &[&str],
    settable: bool,
    unsettable: bool,
    effect: &str,
    status: &str,
    description: &str,
) -> UserConfigSchemaEntry {
    UserConfigSchemaEntry {
        path: path.to_string(),
        value_type: value_type.to_string(),
        allowed_values: allowed_values
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        settable,
        unsettable,
        effect: effect.to_string(),
        status: status.to_string(),
        description: description.to_string(),
    }
}

pub(super) fn entries() -> Vec<UserConfigSchemaEntry> {
    vec![
        entry(
            "providers.workspace_live_sync",
            "enum",
            &["off", "managed", "tracked"],
            true,
            true,
            "provider_reload",
            "live",
            "Global default workspace live sync mode for supported provider runs.",
        ),
        entry("providers.default", "string", &[], true, true, "no_runtime_effect", "unwired", "Persisted provider default; currently not used by launch defaulting."),
        entry("providers.model", "string", &[], true, true, "no_runtime_effect", "unwired", "Persisted model default; currently not used by launch defaulting."),
        entry("providers.account_profile", "string", &[], true, true, "no_runtime_effect", "unwired", "Persisted account profile default; currently not used by launch defaulting."),
        entry("providers.effort", "string", &[], true, true, "no_runtime_effect", "unwired", "Persisted effort default; currently not used by launch defaulting."),
        entry("ui.theme", "string", &[], true, true, "no_runtime_effect", "unwired", "Persisted UI theme value; terminal UI currently uses CLI preferences."),
        entry("ui.multi_agent_response_layout", "string", &[], true, true, "no_runtime_effect", "unwired", "Persisted response layout value; terminal UI currently uses CLI/session preferences."),
        entry("ui.max_agents_per_screen", "u32", &[], true, true, "no_runtime_effect", "unwired", "Persisted pane-count value; terminal UI currently uses CLI preferences."),
        entry("ui.worktree_aliases.<alias>", "string", &[], true, true, "no_runtime_effect", "unwired", "Pattern key for a worktree alias entry."),
        entry("relay.url", "string|null", &[], true, true, "no_runtime_effect", "unwired", "Persisted user-config relay URL; daemon relay connection currently uses daemon config."),
        entry("relay.accept_remote_leases", "bool", &["true", "false"], true, true, "no_runtime_effect", "unwired", "Persisted remote-lease acceptance flag; daemon runtime currently uses daemon config."),
        entry("history.operational.backend", "enum", &["sqlite"], true, false, "restart_required", "boot", "Operational history storage backend."),
        entry("history.operational.path", "string", &[], true, true, "restart_required", "boot", "Operational history SQLite database path."),
        entry("history.operational.retention_days", "u32", &[], true, true, "no_runtime_effect", "unwired", "Retention-days setting; no pruning job currently consumes it."),
        entry("history.operational.max_size_mb", "u32", &[], true, true, "no_runtime_effect", "unwired", "Max-size setting; no pruning job currently consumes it."),
        entry("history.operational.keep_pinned_sessions", "bool", &["true", "false"], true, true, "no_runtime_effect", "unwired", "Pinned-session retention setting; no pruning job currently consumes it."),
        entry("history.operational.archive_inactive_after_days", "u32", &[], true, true, "no_runtime_effect", "unwired", "Inactive-session archival threshold; no archival scheduler currently consumes it."),
        entry("history.operational.archive_deleted_agents", "bool", &["true", "false"], true, true, "no_runtime_effect", "unwired", "Deleted-agent archival flag; deletion flow does not currently consume it."),
        entry("history.archive.mode", "enum", &["disabled", "external"], true, true, "none", "live", "History archive mode."),
        entry("history.archive.url", "string", &[], true, true, "none", "live", "External history archive endpoint."),
        entry("history.archive.token_env", "string", &[], true, true, "none", "live", "Environment variable name for the history archive bearer token."),
        entry("history.archive.archive_deleted_agents", "bool", &["true", "false"], true, true, "no_runtime_effect", "unwired", "Archive-deleted-agents flag; deletion flow does not currently consume it."),
        entry("history.archive.archive_before_delete", "bool", &["true", "false"], true, true, "no_runtime_effect", "unwired", "Archive-before-delete flag; deletion flow does not currently consume it."),
        entry("history.archive.delete_operational_after_verified_archive", "bool", &["true", "false"], true, true, "no_runtime_effect", "unwired", "Delete-after-archive flag; no archive cleanup flow currently consumes it."),
        entry("history.archive.require_durable_acceptance", "bool", &["true", "false"], true, true, "none", "live", "Require durable archive acceptance for history events."),
        entry("artifacts.operational.backend", "enum", &["filesystem"], true, false, "none", "live", "Operational artifact storage backend."),
        entry("artifacts.operational.root", "string", &[], true, true, "none", "live", "Operational artifact filesystem root."),
        entry("artifacts.operational.index_path", "string", &[], true, true, "none", "live", "Operational artifact SQLite index path."),
        entry("artifacts.operational.retention_days", "u32", &[], true, true, "no_runtime_effect", "unwired", "Artifact retention setting; no cleanup job currently consumes it."),
        entry("artifacts.archive.mode", "enum", &["disabled", "external"], true, true, "none", "live", "Artifact archive mode."),
        entry("artifacts.archive.url", "string", &[], true, true, "none", "live", "External artifact archive endpoint."),
        entry("artifacts.archive.token_env", "string", &[], true, true, "none", "live", "Environment variable name for the artifact archive bearer token."),
        entry("artifacts.archive.require_durable_acceptance", "bool", &["true", "false"], true, true, "none", "live", "Require durable archive acceptance for artifact events."),
        entry("state.backend", "enum", &["sqlite"], true, false, "restart_required", "boot", "Durable kernel state backend."),
        entry("state.path", "string", &[], true, true, "restart_required", "boot", "Durable kernel state SQLite database path."),
        entry("state.snapshot_interval_events", "u32", &[], true, true, "none", "live", "Number of state events between durable snapshots."),
        entry("slices.root", "string", &[], true, true, "none", "live", "Arroba-owned slice metadata, logs, and build-helper root."),
        entry("slices.linux.docker_image", "string", &[], true, true, "none", "live", "Docker image tag used for new Linux slices."),
        entry("slices.linux.build_image", "enum", &["auto", "always", "never"], true, true, "none", "live", "Linux slice image build policy."),
        entry("slices.linux.extension_dockerfile", "string", &[], true, true, "none", "live", "Optional user Dockerfile layered on top of the Linux slice image."),
        entry("slices.linux.allow_unconfined_seccomp", "bool", &["true", "false"], true, true, "none", "live", "Allow local Docker slices to disable Docker's seccomp profile for Chromium sandbox compatibility."),
        entry("slices.linux.memory_mb", "u32", &[], true, true, "none", "live", "Optional Docker memory limit for new Linux slice containers."),
        entry("slices.linux.cpus", "string", &[], true, true, "none", "live", "Optional Docker CPU limit for new Linux slice containers."),
        entry("slices.linux.idle_timeout_minutes", "u32", &[], true, true, "no_runtime_effect", "unwired", "Future idle-stop timeout for Linux slices."),
        entry("slices.linux.screen_width", "u32", &[], true, true, "none", "live", "Linux slice virtual screen width."),
        entry("slices.linux.screen_height", "u32", &[], true, true, "none", "live", "Linux slice virtual screen height."),
        entry("kernel.websocket_host", "string", &[], true, true, "restart_required", "boot", "Kernel websocket bind host."),
        entry("kernel.websocket_port", "port", &[], true, true, "restart_required", "boot", "Kernel websocket bind port."),
        entry("kernel.runtime_mcp_host", "string", &[], true, true, "restart_required", "boot", "Runtime MCP bind host."),
        entry("kernel.runtime_mcp_port", "port", &[], true, true, "restart_required", "boot", "Runtime MCP bind port."),
        entry("workflow.max_queues_per_workflow", "u32", &[], true, true, "restart_required", "boot", "Maximum number of prompt queues allowed for one workflow."),
        entry("credential_vault.service", "string", &[], true, false, "none", "live", "OS keychain service namespace for vault-backed credentials."),
        entry("credential_vault.agent_management", "enum", &["allow", "deny"], true, true, "none", "live", "Whether runtime agents may create or update user vault-backed credential handles."),
        entry("version", "u32", &[], true, false, "none", "internal", "User config schema version; migration-owned and not recommended for manual edits."),
    ]
}
