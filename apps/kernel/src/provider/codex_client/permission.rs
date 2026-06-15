//! Codex request permission, sandbox, and collaboration policy mapping.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

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
    let plan_config_overrides = || {
        let mut config_overrides = BTreeMap::new();
        config_overrides.insert("include_apply_patch_tool".to_string(), json!(false));
        config_overrides.insert("features.apply_patch_freeform".to_string(), json!(false));
        config_overrides.insert("tui.mode".to_string(), json!("plan"));
        config_overrides
    };
    match write_access_mode {
        ProviderWriteAccessMode::Unrestricted => {
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
                config_overrides: match execution_mode {
                    AgentExecutionMode::Build => BTreeMap::new(),
                    AgentExecutionMode::Plan => plan_config_overrides(),
                },
            }
        }
        ProviderWriteAccessMode::WorkspaceLiveSyncTracked => CodexPermissionPolicy {
            approval_policy: match permission_level {
                AgentPermissionLevel::Required => json!("untrusted"),
                AgentPermissionLevel::Yolo => json!("never"),
            },
            sandbox: match execution_mode {
                AgentExecutionMode::Build => "danger-full-access",
                AgentExecutionMode::Plan => "read-only",
            },
            sandbox_policy: match execution_mode {
                AgentExecutionMode::Build => json!({ "type": "dangerFullAccess" }),
                AgentExecutionMode::Plan => json!({ "type": "readOnly" }),
            },
            config_overrides: match execution_mode {
                AgentExecutionMode::Build => BTreeMap::new(),
                AgentExecutionMode::Plan => plan_config_overrides(),
            },
        },
        ProviderWriteAccessMode::WorkspaceLiveSyncManaged => {
            #[cfg(target_os = "macos")]
            {
                CodexPermissionPolicy {
                    approval_policy: json!("never"),
                    sandbox: "danger-full-access",
                    sandbox_policy: json!({ "type": "dangerFullAccess" }),
                    config_overrides: BTreeMap::new(),
                }
            }
            #[cfg(not(target_os = "macos"))]
            {
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
}

pub(super) fn workspace_live_sync_codex_permission_grant(
    requested_permissions: &Value,
    protected_roots: &[PathBuf],
    cwd: Option<&Path>,
) -> Value {
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
        if let Some(write) =
            workspace_live_sync_allowed_writes(file_system.get("write"), protected_roots, cwd)
        {
            granted_file_system.insert("write".to_string(), write);
        }
        if !granted_file_system.is_empty() {
            granted.insert("fileSystem".to_string(), Value::Object(granted_file_system));
        }
    }
    Value::Object(granted)
}

fn workspace_live_sync_allowed_writes(
    requested_write: Option<&Value>,
    protected_roots: &[PathBuf],
    cwd: Option<&Path>,
) -> Option<Value> {
    let writes = requested_write?.as_array()?;
    let allowed = writes
        .iter()
        .filter(|entry| {
            entry.as_str().is_some_and(|path| {
                workspace_live_sync_write_is_disjoint_from_protected_roots(
                    path,
                    protected_roots,
                    cwd,
                )
            })
        })
        .cloned()
        .collect::<Vec<_>>();
    (!allowed.is_empty()).then(|| Value::Array(allowed))
}

fn workspace_live_sync_write_is_disjoint_from_protected_roots(
    path: &str,
    protected_roots: &[PathBuf],
    cwd: Option<&Path>,
) -> bool {
    let path = Path::new(path);
    if protected_roots.is_empty() {
        return false;
    }
    let resolved_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        let Some(cwd) = cwd else {
            return false;
        };
        if !cwd.is_absolute() {
            return false;
        }
        cwd.join(path)
    };
    let normalized_path = normalize_path(&resolved_path);
    protected_roots.iter().all(|root| {
        let normalized_root = normalize_path(root);
        !normalized_path.starts_with(&normalized_root)
            && !normalized_root.starts_with(&normalized_path)
    })
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_live_sync_codex_permission_grant_allows_only_disjoint_write_scopes() {
        let requested = json!({
            "network": true,
            "fileSystem": {
                "read": ["/repo/selected", "/repo/sibling"],
                "write": [
                    "/repo/selected",
                    "/repo/selected/src",
                    "/repo",
                    "/repo/sibling",
                    "../peer",
                    "local-temp"
                ]
            }
        });

        let grant = workspace_live_sync_codex_permission_grant(
            &requested,
            &[PathBuf::from("/repo/selected")],
            Some(Path::new("/repo/selected")),
        );

        assert_eq!(
            grant,
            json!({
                "network": true,
                "fileSystem": {
                    "read": ["/repo/selected", "/repo/sibling"],
                    "write": ["/repo/sibling", "../peer"]
                }
            })
        );
    }

    #[test]
    fn workspace_live_sync_codex_permission_grant_keeps_sibling_repo_writes_unrestricted() {
        assert!(workspace_live_sync_write_is_disjoint_from_protected_roots(
            "../../sibling",
            &[PathBuf::from("/repo/selected")],
            Some(Path::new("/repo/selected/src")),
        ));
        assert!(workspace_live_sync_write_is_disjoint_from_protected_roots(
            "/repo/sibling",
            &[PathBuf::from("/repo/selected")],
            Some(Path::new("/repo/selected")),
        ));
        assert!(!workspace_live_sync_write_is_disjoint_from_protected_roots(
            "/repo",
            &[PathBuf::from("/repo/selected")],
            Some(Path::new("/repo/selected")),
        ));
    }
}
