use jsonschema::JSONSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const ACK_WORKFLOW_TURN_TOOL: &str = "ack_workflow_turn";
pub const VALIDATE_WORKFLOW_OUTPUT_TOOL: &str = "validate_workflow_output";
pub const VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL: &str =
    "validate_and_submit_workflow_run_output";
pub const VALIDATE_AND_SUBMIT_INTERMEDIATE_WORKFLOW_RUN_OUTPUT_TOOL: &str =
    "validate_and_submit_intermediate_workflow_run_output";
pub const WORKFLOW_CONSOLE_READ_TOOL: &str = "workflow_console_read";
pub const WORKFLOW_CONSOLE_WRITE_TOOL: &str = "workflow_console_write";
pub const WORKFLOW_CONSOLE_CLEAR_TOOL: &str = "workflow_console_clear";
#[allow(dead_code)]
pub const READ_ARTIFACT_TOOL: &str = "arroba.read_artifact";
#[allow(dead_code)]
pub const EDIT_ARTIFACT_TOOL: &str = "arroba.edit_artifact";
#[allow(dead_code)]
pub const APPLY_PATCH_TOOL: &str = "arroba.apply_patch";
#[allow(dead_code)]
pub const WRITE_ARTIFACT_TOOL: &str = "arroba.write_artifact";
#[allow(dead_code)]
pub const DELETE_ARTIFACT_TOOL: &str = "arroba.delete_artifact";
#[allow(dead_code)]
pub const MOVE_ARTIFACT_TOOL: &str = "arroba.move_artifact";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowRuntimeToolContext {
    pub session_id: String,
    pub workflow_run_ref: String,
    pub workflow_node_run_id: String,
    pub delivery_token: Option<String>,
    pub allowed_output_schema_refs: Vec<String>,
    pub workflow_run_output_schema_ref: Option<String>,
    pub workflow_intermediate_output_schema_ref: Option<String>,
    pub can_complete_workflow_run: bool,
    pub can_emit_intermediate_workflow_run_output: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeToolResult {
    pub ok: bool,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AckWorkflowTurnArgs {
    pub delivery_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidateWorkflowOutputArgs {
    pub output_schema_ref: String,
    pub output_json: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowConsoleWriteArgs {
    pub text: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedReadArtifactArgs {
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
pub struct ManagedEditArtifactArgs {
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
pub struct ManagedWriteArtifactArgs {
    pub path: String,
    pub content_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedApplyPatchArgs {
    pub patch_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedDeleteArtifactArgs {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedMoveArtifactArgs {
    pub from_path: String,
    pub to_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidateAndSubmitWorkflowRunOutputArgs {
    pub workflow_output_json: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_token: Option<String>,
}

#[allow(dead_code)]
pub fn managed_io_runtime_tool_specs() -> Vec<RuntimeToolSpec> {
    vec![
        RuntimeToolSpec {
            name: READ_ARTIFACT_TOOL.to_string(),
            description: "Read a workspace-relative artifact through Arroba managed I/O and return content with snapshot/version metadata.".to_string(),
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
            description: "Apply a workspace-relative text artifact edit through Arroba managed I/O. Non-overlapping stale edits may be rebased with a warning; overlapping edits are rejected with conflict metadata.".to_string(),
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
            name: APPLY_PATCH_TOOL.to_string(),
            description: "Apply an apply_patch-style text patch through Arroba managed I/O. Supports add, update, delete, and move operations atomically. Conflicting hunks are rejected with structured conflict metadata.".to_string(),
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
            description: "Delete a workspace-relative text artifact through Arroba managed I/O.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": {"type": "string"},
                    "domain": {
                        "type": "string",
                        "enum": ["text"]
                    }
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: MOVE_ARTIFACT_TOOL.to_string(),
            description: "Move a workspace-relative text artifact through Arroba managed I/O. Optional old_text/new_text can edit the moved content atomically.".to_string(),
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
                        "enum": ["text"]
                    }
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: WRITE_ARTIFACT_TOOL.to_string(),
            description: "Create or overwrite a workspace-relative text artifact through Arroba managed I/O. Use this when creating a new file or replacing a whole file; use arroba.edit_artifact for smaller edits.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["path", "content_text"],
                "properties": {
                    "path": {"type": "string"},
                    "content_text": {"type": "string"},
                    "snapshot_id": {"type": "string"},
                    "domain": {
                        "type": "string",
                        "enum": ["text"]
                    }
                },
                "additionalProperties": false
            }),
        },
    ]
}

pub fn workflow_runtime_tool_specs() -> Vec<RuntimeToolSpec> {
    vec![
        RuntimeToolSpec {
            name: ACK_WORKFLOW_TURN_TOOL.to_string(),
            description: "Acknowledge that the current workflow turn was received. This does not complete the turn; after this tool returns, continue the same response and emit the required final fenced JSON workflow output.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["delivery_token"],
                "properties": {
                    "delivery_token": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: VALIDATE_WORKFLOW_OUTPUT_TOOL.to_string(),
            description: "Validate workflow output JSON against an allowed schema ref for the current workflow turn.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["output_schema_ref", "output_json"],
                "properties": {
                    "output_schema_ref": {"type": "string"},
                    "output_json": {"type": "string"},
                    "delivery_token": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL.to_string(),
            description: "Validate and submit the final output for the current workflow run.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["workflow_output_json"],
                "properties": {
                    "workflow_output_json": {"type": "string"},
                    "delivery_token": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: VALIDATE_AND_SUBMIT_INTERMEDIATE_WORKFLOW_RUN_OUTPUT_TOOL.to_string(),
            description: "Validate and submit intermediate output for the current workflow run.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["workflow_output_json"],
                "properties": {
                    "workflow_output_json": {"type": "string"},
                    "delivery_token": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: WORKFLOW_CONSOLE_READ_TOOL.to_string(),
            description: "Read the shared workflow console for the current workflow.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: WORKFLOW_CONSOLE_WRITE_TOOL.to_string(),
            description: "Append human-facing text to the shared workflow console for the current workflow.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["text"],
                "properties": {
                    "text": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: WORKFLOW_CONSOLE_CLEAR_TOOL.to_string(),
            description: "Clear the shared workflow console for the current workflow.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
    ]
}

pub fn validate_workflow_output_schema(schema_ref: &str, output_json: &str) -> Result<(), String> {
    let schema_source = std::fs::read_to_string(schema_ref)
        .map_err(|error| format!("schema ref `{schema_ref}` could not be read: {error}"))?;
    let schema_value = serde_json::from_str::<Value>(&schema_source)
        .map_err(|error| format!("schema ref `{schema_ref}` is not valid JSON: {error}"))?;
    let output_value = serde_json::from_str::<Value>(output_json)
        .map_err(|error| format!("output is not valid JSON: {error}"))?;
    let compiled = JSONSchema::options()
        .with_draft(jsonschema::Draft::Draft7)
        .compile(&schema_value)
        .map_err(|error| format!("schema ref `{schema_ref}` failed to compile: {error}"))?;
    if let Err(errors) = compiled.validate(&output_value) {
        let message = errors
            .into_iter()
            .next()
            .map(|error| error.to_string())
            .unwrap_or_else(|| "schema validation failed".to_string());
        return Err(message);
    }
    Ok(())
}

#[cfg(test)]
mod managed_io_tests {
    use super::*;

    #[test]
    fn managed_io_specs_expose_read_and_edit_tools() {
        let specs = managed_io_runtime_tool_specs();
        assert!(specs.iter().any(|spec| spec.name == READ_ARTIFACT_TOOL));
        assert!(specs.iter().any(|spec| spec.name == EDIT_ARTIFACT_TOOL));
        assert!(specs.iter().any(|spec| spec.name == APPLY_PATCH_TOOL));
        assert!(specs.iter().any(|spec| spec.name == WRITE_ARTIFACT_TOOL));
        assert!(specs.iter().any(|spec| spec.name == DELETE_ARTIFACT_TOOL));
        assert!(specs.iter().any(|spec| spec.name == MOVE_ARTIFACT_TOOL));
    }

    #[test]
    fn managed_edit_args_accept_text_replace_shape() {
        let args = serde_json::from_value::<ManagedEditArtifactArgs>(serde_json::json!({
            "path": "src/lib.rs",
            "snapshot_id": "snap:1",
            "old_text": "before",
            "new_text": "after"
        }))
        .expect("managed edit args should parse");

        assert_eq!(args.path, "src/lib.rs");
        assert_eq!(args.old_text.as_deref(), Some("before"));
        assert_eq!(args.new_text, "after");
    }

    #[test]
    fn managed_write_args_accept_text_content_shape() {
        let args = serde_json::from_value::<ManagedWriteArtifactArgs>(serde_json::json!({
            "path": "src/lib.rs",
            "content_text": "hello"
        }))
        .expect("managed write args should parse");

        assert_eq!(args.path, "src/lib.rs");
        assert_eq!(args.content_text, "hello");
    }

    #[test]
    fn managed_apply_patch_args_accept_patch_text_shape() {
        let args = serde_json::from_value::<ManagedApplyPatchArgs>(serde_json::json!({
            "patch_text": "*** Begin Patch\n*** Update File: src/lib.rs\n@@\n-old\n+new\n*** End Patch",
            "domain": "text"
        }))
        .expect("managed apply patch args should parse");

        assert!(args.patch_text.contains("*** Begin Patch"));
        assert_eq!(args.domain.as_deref(), Some("text"));
    }

    #[test]
    fn managed_delete_args_accept_path_shape() {
        let args = serde_json::from_value::<ManagedDeleteArtifactArgs>(serde_json::json!({
            "path": "src/lib.rs"
        }))
        .expect("managed delete args should parse");

        assert_eq!(args.path, "src/lib.rs");
    }

    #[test]
    fn managed_move_args_accept_path_shape() {
        let args = serde_json::from_value::<ManagedMoveArtifactArgs>(serde_json::json!({
            "from_path": "src/old.rs",
            "to_path": "src/new.rs",
            "old_text": "old",
            "new_text": "new"
        }))
        .expect("managed move args should parse");

        assert_eq!(args.from_path, "src/old.rs");
        assert_eq!(args.to_path, "src/new.rs");
        assert_eq!(args.old_text.as_deref(), Some("old"));
        assert_eq!(args.new_text.as_deref(), Some("new"));
    }
}
