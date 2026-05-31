use serde::{Deserialize, Serialize};

use super::RuntimeToolSpec;

#[allow(dead_code)]
pub const READ_ARTIFACT_TOOL: &str = "arroba.read_artifact";
pub const READ_ARTIFACT_TOOL_ALIAS: &str = "read_artifact";
#[allow(dead_code)]
pub const EDIT_ARTIFACT_TOOL: &str = "arroba.edit_artifact";
pub const EDIT_ARTIFACT_TOOL_ALIAS: &str = "edit_artifact";
#[allow(dead_code)]
pub const APPLY_PATCH_TOOL: &str = "arroba.apply_patch";
pub const APPLY_PATCH_TOOL_ALIAS: &str = "apply_patch";
pub const PATCH_ARTIFACT_TOOL_ALIAS: &str = "patch_artifact";
#[allow(dead_code)]
pub const WRITE_ARTIFACT_TOOL: &str = "arroba.write_artifact";
pub const WRITE_ARTIFACT_TOOL_ALIAS: &str = "write_artifact";
#[allow(dead_code)]
pub const DELETE_ARTIFACT_TOOL: &str = "arroba.delete_artifact";
pub const DELETE_ARTIFACT_TOOL_ALIAS: &str = "delete_artifact";
#[allow(dead_code)]
pub const MOVE_ARTIFACT_TOOL: &str = "arroba.move_artifact";
pub const MOVE_ARTIFACT_TOOL_ALIAS: &str = "move_artifact";

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceLiveSyncReadArtifactArgs {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedTextRangeArgs {
    pub start: usize,
    pub end: usize,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceLiveSyncEditArtifactArgs {
    pub path: String,
    pub new_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<ManagedTextRangeArgs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceLiveSyncWriteArtifactArgs {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_base64: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceLiveSyncApplyPatchArgs {
    pub patch_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceLiveSyncDeleteArtifactArgs {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceLiveSyncMoveArtifactArgs {
    pub from_path: String,
    pub to_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
}

impl WorkspaceLiveSyncMoveArtifactArgs {
    pub fn normalized_text_transform_fields(&self) -> (Option<String>, Option<String>) {
        if self.old_text.as_deref() == Some("") && self.new_text.as_deref() == Some("") {
            return (None, None);
        }
        (self.old_text.clone(), self.new_text.clone())
    }

    pub fn has_non_text_transform_fields(&self) -> bool {
        self.old_text
            .as_deref()
            .is_some_and(|value| !value.is_empty())
            || self
                .new_text
                .as_deref()
                .is_some_and(|value| !value.is_empty())
    }
}

#[allow(dead_code)]
pub fn workspace_live_sync_runtime_tool_specs() -> Vec<RuntimeToolSpec> {
    let canonical = vec![
        RuntimeToolSpec {
            name: READ_ARTIFACT_TOOL.to_string(),
            description: "Read a workspace-relative artifact through Arroba workspace live sync and return content with snapshot/version metadata. Text/structured artifacts return content_text; opaque artifacts return content_base64.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": {"type": "string"},
                    "domain": {
                        "type": "string",
                        "enum": ["text", "structured", "opaque"]
                    }
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: EDIT_ARTIFACT_TOOL.to_string(),
            description: "Apply a workspace-relative text artifact edit through Arroba workspace live sync. Non-overlapping stale edits may be rebased with a warning; overlapping edits are rejected with conflict metadata.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["path", "new_text"],
                "properties": {
                    "path": {"type": "string"},
                    "snapshot_id": {"type": "string"},
                    "old_text": {"type": "string"},
                    "new_text": {"type": "string"},
                    "range": {
                        "type": "object",
                        "required": ["start", "end"],
                        "properties": {
                            "start": {"type": "integer", "minimum": 0},
                            "end": {"type": "integer", "minimum": 0}
                        },
                        "additionalProperties": false
                    },
                    "domain": {
                        "type": "string",
                        "enum": ["text"]
                    }
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: PATCH_ARTIFACT_TOOL_ALIAS.to_string(),
            description: "Apply a text artifact patch through Arroba workspace live sync. Supports add, update, delete, and move operations atomically. Conflicting hunks are rejected with structured conflict metadata.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["patch_text"],
                "properties": {
                    "patch_text": {"type": "string"},
                    "domain": {
                        "type": "string",
                        "enum": ["text"]
                    }
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: DELETE_ARTIFACT_TOOL.to_string(),
            description: "Delete a workspace-relative artifact through Arroba workspace live sync. Non-text artifacts are coordinated as whole-file operations.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": {"type": "string"},
                    "domain": {
                        "type": "string",
                        "enum": ["text", "structured", "opaque"]
                    }
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: MOVE_ARTIFACT_TOOL.to_string(),
            description: "Move a workspace-relative artifact through Arroba workspace live sync. Optional old_text/new_text can edit moved text content atomically. Non-text artifacts are moved as whole files without content transforms.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["from_path", "to_path"],
                "properties": {
                    "from_path": {"type": "string"},
                    "to_path": {"type": "string"},
                    "old_text": {"type": "string"},
                    "new_text": {"type": "string"},
                    "domain": {
                        "type": "string",
                        "enum": ["text", "structured", "opaque"]
                    }
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: WRITE_ARTIFACT_TOOL.to_string(),
            description: "Create or overwrite a workspace-relative artifact through Arroba workspace live sync. Use content_text for text/structured artifacts and content_base64 for opaque artifacts. Non-text artifacts are coordinated as whole-file operations.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": {"type": "string"},
                    "content_text": {"type": "string"},
                    "content_base64": {"type": "string"},
                    "snapshot_id": {"type": "string"},
                    "domain": {
                        "type": "string",
                        "enum": ["text", "structured", "opaque"]
                    }
                },
                "additionalProperties": false
            }),
        },
    ];
    let aliases = canonical
        .iter()
        .filter_map(workspace_live_sync_alias_spec)
        .collect::<Vec<_>>();
    let mut specs = canonical;
    specs.extend(aliases);
    specs
}

fn workspace_live_sync_alias_spec(spec: &RuntimeToolSpec) -> Option<RuntimeToolSpec> {
    let alias = match spec.name.as_str() {
        READ_ARTIFACT_TOOL => READ_ARTIFACT_TOOL_ALIAS,
        EDIT_ARTIFACT_TOOL => EDIT_ARTIFACT_TOOL_ALIAS,
        DELETE_ARTIFACT_TOOL => DELETE_ARTIFACT_TOOL_ALIAS,
        MOVE_ARTIFACT_TOOL => MOVE_ARTIFACT_TOOL_ALIAS,
        WRITE_ARTIFACT_TOOL => WRITE_ARTIFACT_TOOL_ALIAS,
        _ => return None,
    };
    let mut spec = spec.clone();
    spec.name = alias.to_string();
    spec.description = format!(
        "{} Alias for `{}`.",
        spec.description,
        canonical_workspace_live_sync_tool_name(alias).unwrap_or(alias)
    );
    Some(spec)
}

pub fn canonical_workspace_live_sync_tool_name(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        READ_ARTIFACT_TOOL
        | READ_ARTIFACT_TOOL_ALIAS
        | "arroba_read_artifact"
        | "mcp__arroba__read_artifact"
        | "mcp__arroba__arroba_read_artifact" => Some(READ_ARTIFACT_TOOL),
        EDIT_ARTIFACT_TOOL
        | EDIT_ARTIFACT_TOOL_ALIAS
        | "arroba_edit_artifact"
        | "mcp__arroba__edit_artifact"
        | "mcp__arroba__arroba_edit_artifact" => Some(EDIT_ARTIFACT_TOOL),
        APPLY_PATCH_TOOL
        | APPLY_PATCH_TOOL_ALIAS
        | PATCH_ARTIFACT_TOOL_ALIAS
        | "arroba_apply_patch"
        | "arroba_patch_artifact"
        | "mcp__arroba__apply_patch"
        | "mcp__arroba__patch_artifact"
        | "mcp__arroba__arroba_apply_patch"
        | "mcp__arroba__arroba_patch_artifact" => Some(APPLY_PATCH_TOOL),
        DELETE_ARTIFACT_TOOL
        | DELETE_ARTIFACT_TOOL_ALIAS
        | "arroba_delete_artifact"
        | "mcp__arroba__delete_artifact"
        | "mcp__arroba__arroba_delete_artifact" => Some(DELETE_ARTIFACT_TOOL),
        MOVE_ARTIFACT_TOOL
        | MOVE_ARTIFACT_TOOL_ALIAS
        | "arroba_move_artifact"
        | "mcp__arroba__move_artifact"
        | "mcp__arroba__arroba_move_artifact" => Some(MOVE_ARTIFACT_TOOL),
        WRITE_ARTIFACT_TOOL
        | WRITE_ARTIFACT_TOOL_ALIAS
        | "arroba_write_artifact"
        | "mcp__arroba__write_artifact"
        | "mcp__arroba__arroba_write_artifact" => Some(WRITE_ARTIFACT_TOOL),
        _ => None,
    }
}
