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
pub const READ_ARTIFACT_TOOL_ALIAS: &str = "read_artifact";
#[allow(dead_code)]
pub const EDIT_ARTIFACT_TOOL: &str = "arroba.edit_artifact";
pub const EDIT_ARTIFACT_TOOL_ALIAS: &str = "edit_artifact";
#[allow(dead_code)]
pub const APPLY_PATCH_TOOL: &str = "arroba.apply_patch";
pub const APPLY_PATCH_TOOL_ALIAS: &str = "apply_patch";
#[allow(dead_code)]
pub const WRITE_ARTIFACT_TOOL: &str = "arroba.write_artifact";
pub const WRITE_ARTIFACT_TOOL_ALIAS: &str = "write_artifact";
#[allow(dead_code)]
pub const DELETE_ARTIFACT_TOOL: &str = "arroba.delete_artifact";
pub const DELETE_ARTIFACT_TOOL_ALIAS: &str = "delete_artifact";
#[allow(dead_code)]
pub const MOVE_ARTIFACT_TOOL: &str = "arroba.move_artifact";
pub const MOVE_ARTIFACT_TOOL_ALIAS: &str = "move_artifact";
pub const LIST_CAPABILITIES_TOOL: &str = "arroba.list_capabilities";
pub const LIST_CAPABILITIES_TOOL_ALIAS: &str = "list_capabilities";
pub const REQUEST_CAPABILITY_TOOL: &str = "arroba.request_capability";
pub const REQUEST_CAPABILITY_TOOL_ALIAS: &str = "request_capability";
pub const LIST_CREDENTIAL_HANDLES_TOOL: &str = "arroba.list_credential_handles";
pub const LIST_CREDENTIAL_HANDLES_TOOL_ALIAS: &str = "list_credential_handles";
pub const HTTP_REQUEST_WITH_CREDENTIAL_TOOL: &str = "arroba.http_request_with_credential";
pub const HTTP_REQUEST_WITH_CREDENTIAL_TOOL_ALIAS: &str = "http_request_with_credential";
pub const SEND_SECRET_TO_TERMINAL_TOOL: &str = "arroba.send_secret_to_terminal";
pub const SEND_SECRET_TO_TERMINAL_TOOL_ALIAS: &str = "send_secret_to_terminal";

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
pub struct ListCapabilitiesArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestCapabilityArgs {
    pub kind: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_body: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpRequestWithCredentialArgs {
    pub credential_id: String,
    pub url: String,
    #[serde(default = "default_http_method")]
    pub method: String,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub headers: std::collections::BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_json: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendSecretToTerminalArgs {
    pub credential_id: String,
    #[serde(default = "default_append_newline")]
    pub append_newline: bool,
}

fn default_append_newline() -> bool {
    true
}

fn default_http_method() -> String {
    "GET".to_string()
}

impl ManagedMoveArtifactArgs {
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

pub fn capability_runtime_tool_specs() -> Vec<RuntimeToolSpec> {
    let canonical = vec![
        RuntimeToolSpec {
            name: LIST_CAPABILITIES_TOOL.to_string(),
            description: "List Arroba-managed MCPs and skills available in this workspace, including whether they are already granted to the current agent. Use this before requesting a capability.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "kind": {
                        "type": "string",
                        "enum": ["mcp", "skill", "all"]
                    }
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: REQUEST_CAPABILITY_TOOL.to_string(),
            description: "Request access to an Arroba-managed MCP or skill for the current agent. V1 grants valid requests automatically. MCP grants activate through an Arroba-managed provider conversation reload; agent-requested MCPs resume via an automatic continuation prompt after the current turn. Skill requests can return the full SKILL.md body so the current turn can use the skill immediately.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["kind", "name"],
                "properties": {
                    "kind": {
                        "type": "string",
                        "enum": ["mcp", "skill"]
                    },
                    "name": {"type": "string"},
                    "reason": {"type": "string"},
                    "return_body": {"type": "boolean"}
                },
                "additionalProperties": false
            }),
        },
    ];
    let aliases = canonical
        .iter()
        .filter_map(capability_alias_spec)
        .collect::<Vec<_>>();
    let mut specs = canonical;
    specs.extend(aliases);
    specs
}

pub fn credential_runtime_tool_specs() -> Vec<RuntimeToolSpec> {
    let canonical = vec![
        RuntimeToolSpec {
            name: LIST_CREDENTIAL_HANDLES_TOOL.to_string(),
            description: "List Arroba credential handles available to this runtime. Values are never returned; use a handle id with http_request_with_credential when a request needs a secret.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: HTTP_REQUEST_WITH_CREDENTIAL_TOOL.to_string(),
            description: "Perform an HTTP request using an Arroba credential handle. Arroba resolves and injects/signs the secret outside the model context, enforces the handle policy, and returns only the HTTP status/body.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["credential_id", "url"],
                "properties": {
                    "credential_id": {"type": "string"},
                    "method": {
                        "type": "string",
                        "enum": ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"]
                    },
                    "url": {"type": "string"},
                    "headers": {
                        "type": "object",
                        "additionalProperties": {"type": "string"}
                    },
                    "body_text": {"type": "string"},
                    "body_json": {}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: SEND_SECRET_TO_TERMINAL_TOOL.to_string(),
            description: "Write a terminal credential directly to the current provider PTY stdin. The secret value is not returned, recorded as terminal input, or placed in model context.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["credential_id"],
                "properties": {
                    "credential_id": {"type": "string"},
                    "append_newline": {"type": "boolean"}
                },
                "additionalProperties": false
            }),
        },
    ];
    let aliases = canonical
        .iter()
        .filter_map(credential_alias_spec)
        .collect::<Vec<_>>();
    let mut specs = canonical;
    specs.extend(aliases);
    specs
}

fn credential_alias_spec(spec: &RuntimeToolSpec) -> Option<RuntimeToolSpec> {
    let alias = match spec.name.as_str() {
        LIST_CREDENTIAL_HANDLES_TOOL => LIST_CREDENTIAL_HANDLES_TOOL_ALIAS,
        HTTP_REQUEST_WITH_CREDENTIAL_TOOL => HTTP_REQUEST_WITH_CREDENTIAL_TOOL_ALIAS,
        SEND_SECRET_TO_TERMINAL_TOOL => SEND_SECRET_TO_TERMINAL_TOOL_ALIAS,
        _ => return None,
    };
    let mut spec = spec.clone();
    spec.name = alias.to_string();
    spec.description = format!("{} Alias for `{}`.", spec.description, alias);
    Some(spec)
}

pub fn canonical_credential_tool_name(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        LIST_CREDENTIAL_HANDLES_TOOL
        | LIST_CREDENTIAL_HANDLES_TOOL_ALIAS
        | "arroba_list_credential_handles"
        | "mcp__arroba__list_credential_handles"
        | "mcp__arroba__arroba_list_credential_handles" => Some(LIST_CREDENTIAL_HANDLES_TOOL),
        HTTP_REQUEST_WITH_CREDENTIAL_TOOL
        | HTTP_REQUEST_WITH_CREDENTIAL_TOOL_ALIAS
        | "arroba_http_request_with_credential"
        | "mcp__arroba__http_request_with_credential"
        | "mcp__arroba__arroba_http_request_with_credential" => {
            Some(HTTP_REQUEST_WITH_CREDENTIAL_TOOL)
        }
        SEND_SECRET_TO_TERMINAL_TOOL
        | SEND_SECRET_TO_TERMINAL_TOOL_ALIAS
        | "arroba_send_secret_to_terminal"
        | "mcp__arroba__send_secret_to_terminal"
        | "mcp__arroba__arroba_send_secret_to_terminal" => Some(SEND_SECRET_TO_TERMINAL_TOOL),
        _ => None,
    }
}

fn capability_alias_spec(spec: &RuntimeToolSpec) -> Option<RuntimeToolSpec> {
    let alias = match spec.name.as_str() {
        LIST_CAPABILITIES_TOOL => LIST_CAPABILITIES_TOOL_ALIAS,
        REQUEST_CAPABILITY_TOOL => REQUEST_CAPABILITY_TOOL_ALIAS,
        _ => return None,
    };
    let mut spec = spec.clone();
    spec.name = alias.to_string();
    spec.description = format!(
        "{} Alias for `{}`.",
        spec.description,
        canonical_capability_tool_name(alias).unwrap_or(alias)
    );
    Some(spec)
}

pub fn canonical_capability_tool_name(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        LIST_CAPABILITIES_TOOL
        | LIST_CAPABILITIES_TOOL_ALIAS
        | "arroba_list_capabilities"
        | "mcp__arroba__list_capabilities"
        | "mcp__arroba__arroba_list_capabilities" => Some(LIST_CAPABILITIES_TOOL),
        REQUEST_CAPABILITY_TOOL
        | REQUEST_CAPABILITY_TOOL_ALIAS
        | "arroba_request_capability"
        | "mcp__arroba__request_capability"
        | "mcp__arroba__arroba_request_capability" => Some(REQUEST_CAPABILITY_TOOL),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidateAndSubmitWorkflowRunOutputArgs {
    pub workflow_output_json: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_token: Option<String>,
}

#[allow(dead_code)]
pub fn managed_io_runtime_tool_specs() -> Vec<RuntimeToolSpec> {
    let canonical = vec![
        RuntimeToolSpec {
            name: READ_ARTIFACT_TOOL.to_string(),
            description: "Read a workspace-relative artifact through Arroba managed I/O and return content with snapshot/version metadata. Text/structured artifacts return content_text; opaque artifacts return content_base64.".to_string(),
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
            description: "Delete a workspace-relative artifact through Arroba managed I/O. Non-text artifacts are coordinated as whole-file operations.".to_string(),
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
            description: "Move a workspace-relative artifact through Arroba managed I/O. Optional old_text/new_text can edit moved text content atomically. Non-text artifacts are moved as whole files without content transforms.".to_string(),
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
            description: "Create or overwrite a workspace-relative artifact through Arroba managed I/O. Use content_text for text/structured artifacts and content_base64 for opaque artifacts. Non-text artifacts are coordinated as whole-file operations.".to_string(),
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
        .filter_map(managed_io_alias_spec)
        .collect::<Vec<_>>();
    let mut specs = canonical;
    specs.extend(aliases);
    specs
}

fn managed_io_alias_spec(spec: &RuntimeToolSpec) -> Option<RuntimeToolSpec> {
    let alias = match spec.name.as_str() {
        READ_ARTIFACT_TOOL => READ_ARTIFACT_TOOL_ALIAS,
        EDIT_ARTIFACT_TOOL => EDIT_ARTIFACT_TOOL_ALIAS,
        APPLY_PATCH_TOOL => APPLY_PATCH_TOOL_ALIAS,
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
        canonical_managed_io_tool_name(alias).unwrap_or(alias)
    );
    Some(spec)
}

pub fn canonical_managed_io_tool_name(tool_name: &str) -> Option<&'static str> {
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
        | "arroba_apply_patch"
        | "mcp__arroba__apply_patch"
        | "mcp__arroba__arroba_apply_patch" => Some(APPLY_PATCH_TOOL),
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
        assert!(specs
            .iter()
            .any(|spec| spec.name == READ_ARTIFACT_TOOL_ALIAS));
        assert!(specs.iter().any(|spec| spec.name == EDIT_ARTIFACT_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == EDIT_ARTIFACT_TOOL_ALIAS));
        assert!(specs.iter().any(|spec| spec.name == APPLY_PATCH_TOOL));
        assert!(specs.iter().any(|spec| spec.name == APPLY_PATCH_TOOL_ALIAS));
        assert!(specs.iter().any(|spec| spec.name == WRITE_ARTIFACT_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == WRITE_ARTIFACT_TOOL_ALIAS));
        assert!(specs.iter().any(|spec| spec.name == DELETE_ARTIFACT_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == DELETE_ARTIFACT_TOOL_ALIAS));
        assert!(specs.iter().any(|spec| spec.name == MOVE_ARTIFACT_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == MOVE_ARTIFACT_TOOL_ALIAS));
    }

    #[test]
    fn capability_specs_expose_discovery_and_request_tools() {
        let specs = capability_runtime_tool_specs();
        assert!(specs.iter().any(|spec| spec.name == LIST_CAPABILITIES_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == LIST_CAPABILITIES_TOOL_ALIAS));
        assert!(specs
            .iter()
            .any(|spec| spec.name == REQUEST_CAPABILITY_TOOL));
        assert!(specs
            .iter()
            .any(|spec| spec.name == REQUEST_CAPABILITY_TOOL_ALIAS));
    }

    #[test]
    fn canonical_capability_tool_name_accepts_provider_aliases() {
        assert_eq!(
            canonical_capability_tool_name("mcp__arroba__list_capabilities"),
            Some(LIST_CAPABILITIES_TOOL)
        );
        assert_eq!(
            canonical_capability_tool_name("mcp__arroba__arroba_request_capability"),
            Some(REQUEST_CAPABILITY_TOOL)
        );
        assert_eq!(canonical_capability_tool_name("unknown"), None);
    }

    #[test]
    fn canonical_managed_io_tool_name_accepts_provider_aliases() {
        assert_eq!(
            canonical_managed_io_tool_name("mcp__arroba__read_artifact"),
            Some(READ_ARTIFACT_TOOL)
        );
        assert_eq!(
            canonical_managed_io_tool_name("mcp__arroba__arroba_read_artifact"),
            Some(READ_ARTIFACT_TOOL)
        );
        assert_eq!(
            canonical_managed_io_tool_name("arroba_apply_patch"),
            Some(APPLY_PATCH_TOOL)
        );
        assert_eq!(canonical_managed_io_tool_name("unknown"), None);
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
        assert_eq!(args.content_text.as_deref(), Some("hello"));
        assert_eq!(args.content_base64, None);
    }

    #[test]
    fn managed_write_args_accept_opaque_content_shape() {
        let args = serde_json::from_value::<ManagedWriteArtifactArgs>(serde_json::json!({
            "path": "assets/blob.bin",
            "content_base64": "AAEC",
            "domain": "opaque"
        }))
        .expect("managed opaque write args should parse");

        assert_eq!(args.path, "assets/blob.bin");
        assert_eq!(args.content_text, None);
        assert_eq!(args.content_base64.as_deref(), Some("AAEC"));
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

    #[test]
    fn managed_move_args_treat_empty_transform_fields_as_absent_for_non_text() {
        let args = serde_json::from_value::<ManagedMoveArtifactArgs>(serde_json::json!({
            "from_path": "from.bin",
            "to_path": "to.bin",
            "old_text": "",
            "new_text": "",
            "domain": "opaque"
        }))
        .expect("managed move args should parse");

        assert!(!args.has_non_text_transform_fields());
    }
}
