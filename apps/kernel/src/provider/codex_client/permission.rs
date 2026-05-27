//! Codex request permission, sandbox, and collaboration policy mapping.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::provider::{AgentExecutionMode, AgentPermissionLevel, ProviderWriteAccessMode};

pub(super) struct CodexPermissionPolicy {
    pub(super) approval_policy: Value,
    pub(super) sandbox: &'static str,
    pub(super) sandbox_policy: Value,
    pub(super) config_overrides: BTreeMap<String, Value>,
}

pub(super) fn codex_permission_policy(
    write_access_mode: ProviderWriteAccessMode,
    execution_mode: AgentExecutionMode,
    permission_level: AgentPermissionLevel,
) -> CodexPermissionPolicy {
    match write_access_mode {
        ProviderWriteAccessMode::Unrestricted | ProviderWriteAccessMode::WorkspaceLiveSyncTracked => {
            let yolo_build = execution_mode == AgentExecutionMode::Build
                && permission_level == AgentPermissionLevel::Yolo;
            CodexPermissionPolicy {
                approval_policy: match permission_level {
                    AgentPermissionLevel::Required => json!("untrusted"),
                    AgentPermissionLevel::Yolo => json!("never"),
                },
                sandbox: match (execution_mode, yolo_build) {
                    (AgentExecutionMode::Build, true) => "danger-full-access",
                    (AgentExecutionMode::Build, false) => "workspace-write",
                    (AgentExecutionMode::Plan, _) => "read-only",
                },
                sandbox_policy: match (execution_mode, yolo_build) {
                    (AgentExecutionMode::Build, true) => json!({ "type": "dangerFullAccess" }),
                    (AgentExecutionMode::Build, false) => json!({ "type": "workspaceWrite" }),
                    (AgentExecutionMode::Plan, _) => json!({ "type": "readOnly" }),
                },
                config_overrides: BTreeMap::new(),
            }
        }
        ProviderWriteAccessMode::WorkspaceLiveSyncManaged => {
            let mut config_overrides = BTreeMap::new();
            config_overrides.insert("include_apply_patch_tool".to_string(), json!(false));
            config_overrides.insert("features.apply_patch_freeform".to_string(), json!(false));
            CodexPermissionPolicy {
                approval_policy: json!("never"),
                sandbox: "read-only",
                sandbox_policy: json!({ "type": "readOnly" }),
                config_overrides,
            }
        }
    }
}

pub(super) fn codex_collaboration_mode(
    execution_mode: AgentExecutionMode,
    model: Option<&str>,
    effort: Option<&str>,
) -> Option<Value> {
    let model = model?.trim();
    if model.is_empty() {
        return None;
    }
    let effort = effort
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(Value::from)
        .unwrap_or(Value::Null);
    let mode = match execution_mode {
        AgentExecutionMode::Build => "default",
        AgentExecutionMode::Plan => "plan",
    };
    Some(json!({
        "mode": mode,
        "settings": {
            "model": model,
            "reasoning_effort": effort,
            "developer_instructions": Value::Null,
        }
    }))
}

pub(super) fn workspace_live_sync_codex_permission_grant(requested_permissions: &Value) -> Value {
    let Some(requested) = requested_permissions.as_object() else {
        return json!({});
    };
    let mut granted = serde_json::Map::new();
    if let Some(network) = requested.get("network") {
        granted.insert("network".to_string(), network.clone());
    }
    if let Some(file_system) = requested.get("fileSystem").and_then(Value::as_object) {
        let mut granted_file_system = serde_json::Map::new();
        if let Some(read) = file_system.get("read") {
            granted_file_system.insert("read".to_string(), read.clone());
        }
        if !granted_file_system.is_empty() {
            granted.insert("fileSystem".to_string(), Value::Object(granted_file_system));
        }
    }
    Value::Object(granted)
}
