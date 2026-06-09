use serde::{Deserialize, Serialize};
use serde_json::Value;

mod slice_tools;
mod validation;
mod workspace_live_sync_tools;
pub use slice_tools::{canonical_slice_tool_name, slice_runtime_tool_specs};
pub use validation::{validate_json_output_schema, validate_workflow_handoff_schema};
pub use workspace_live_sync_tools::*;

#[cfg(test)]
mod tests;

pub const ACK_WORKFLOW_TURN_TOOL: &str = "ack_workflow_turn";
pub const VALIDATE_WORKFLOW_HANDOFF_TOOL: &str = "validate_workflow_handoff";
pub const VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL: &str =
    "validate_and_submit_workflow_run_output";
pub const VALIDATE_AND_SUBMIT_INTERMEDIATE_WORKFLOW_RUN_OUTPUT_TOOL: &str =
    "validate_and_submit_intermediate_workflow_run_output";
pub const READ_WORKFLOW_TURN_CONTEXT_TOOL: &str = "read_workflow_turn_context";
pub const WORKFLOW_CONSOLE_READ_TOOL: &str = "workflow_console_read";
pub const WORKFLOW_CONSOLE_WRITE_TOOL: &str = "workflow_console_write";
pub const WORKFLOW_CONSOLE_CLEAR_TOOL: &str = "workflow_console_clear";
pub const AGENT_APP_ACTION_TOOL: &str = "agent_app_action";
pub const AGENT_APP_ACTION_TOOL_QUALIFIED: &str = "arroba.agent_app_action";
pub const LIST_EXTENSIONS_TOOL: &str = "arroba.list_extensions";
pub const REQUEST_EXTENSION_TOOL: &str = "arroba.request_extension";
pub const REGISTER_MCP_TOOL: &str = "arroba.register_mcp";
pub const REGISTER_SKILL_PATH_TOOL: &str = "arroba.register_skill_path";
pub const REGISTER_ENVIRONMENT_TOOL: &str = "arroba.register_environment";
pub const REGISTER_SCRIPT_PATH_TOOL: &str = "arroba.register_script_path";
pub const REGISTER_CONNECTOR_PATH_TOOL: &str = "arroba.register_connector_path";
pub const REGISTER_CONNECTOR_ADAPTER_PATH_TOOL: &str = "arroba.register_connector_adapter_path";
pub const SEARCH_RECALL_TOOL: &str = "arroba.search_recall";
pub const QUERY_RECALL_TOOL: &str = "arroba.query_recall";
pub const LIST_CREDENTIAL_HANDLES_TOOL: &str = "arroba.list_credential_handles";
pub const LIST_CREDENTIAL_HANDLES_TOOL_ALIAS: &str = "list_credential_handles";
pub const CREATE_GENERATED_CREDENTIAL_TOOL: &str = "arroba.create_generated_credential";
pub const CREATE_GENERATED_CREDENTIAL_TOOL_ALIAS: &str = "create_generated_credential";
pub const REQUEST_CREDENTIAL_SECRET_TOOL: &str = "arroba.request_credential_secret";
pub const REQUEST_CREDENTIAL_SECRET_TOOL_ALIAS: &str = "request_credential_secret";
pub const HTTP_REQUEST_WITH_CREDENTIAL_TOOL: &str = "arroba.http_request_with_credential";
pub const HTTP_REQUEST_WITH_CREDENTIAL_TOOL_ALIAS: &str = "http_request_with_credential";
pub const SEND_SECRET_TO_TERMINAL_TOOL: &str = "arroba.send_secret_to_terminal";
pub const SEND_SECRET_TO_TERMINAL_TOOL_ALIAS: &str = "send_secret_to_terminal";
pub const PASTE_SECRET_TO_SLICE_TOOL: &str = "arroba.paste_secret_to_slice";
pub const PASTE_SECRET_TO_SLICE_TOOL_ALIAS: &str = "paste_secret_to_slice";
pub const REQUEST_POPUP_TOOL: &str = "arroba.request_popup";
pub const REQUEST_POPUP_TOOL_ALIAS: &str = "request_popup";
pub const SLICE_SCREEN_STATUS_TOOL: &str = "arroba.slice_screen_status";
pub const SLICE_SCREEN_STATUS_TOOL_ALIAS: &str = "slice_screen_status";
pub const SLICE_SCREENSHOT_TOOL: &str = "arroba.slice_screenshot";
pub const SLICE_SCREENSHOT_TOOL_ALIAS: &str = "slice_screenshot";
pub const SLICE_OCR_TOOL: &str = "arroba.slice_ocr";
pub const SLICE_OCR_TOOL_ALIAS: &str = "slice_ocr";
pub const SLICE_FIND_TEXT_TOOL: &str = "arroba.slice_find_text";
pub const SLICE_FIND_TEXT_TOOL_ALIAS: &str = "slice_find_text";
pub const SLICE_MOUSE_TOOL: &str = "arroba.slice_mouse";
pub const SLICE_MOUSE_TOOL_ALIAS: &str = "slice_mouse";
pub const SLICE_KEYBOARD_TOOL: &str = "arroba.slice_keyboard";
pub const SLICE_KEYBOARD_TOOL_ALIAS: &str = "slice_keyboard";
pub const SLICE_OPEN_URL_TOOL: &str = "arroba.slice_open_url";
pub const SLICE_OPEN_URL_TOOL_ALIAS: &str = "slice_open_url";
pub const SLICE_BROWSER_STATUS_TOOL: &str = "arroba.slice_browser_status";
pub const SLICE_BROWSER_STATUS_TOOL_ALIAS: &str = "slice_browser_status";
pub const SLICE_BROWSER_FIND_TOOL: &str = "arroba.slice_browser_find";
pub const SLICE_BROWSER_FIND_TOOL_ALIAS: &str = "slice_browser_find";
pub const SLICE_BROWSER_FILL_TOOL: &str = "arroba.slice_browser_fill";
pub const SLICE_BROWSER_FILL_TOOL_ALIAS: &str = "slice_browser_fill";
pub const SLICE_BROWSER_CLICK_TOOL: &str = "arroba.slice_browser_click";
pub const SLICE_BROWSER_CLICK_TOOL_ALIAS: &str = "slice_browser_click";
pub const SLICE_BROWSER_SUBMIT_TOOL: &str = "arroba.slice_browser_submit";
pub const SLICE_BROWSER_SUBMIT_TOOL_ALIAS: &str = "slice_browser_submit";
pub const SLICE_BROWSER_TEXT_TOOL: &str = "arroba.slice_browser_text";
pub const SLICE_BROWSER_TEXT_TOOL_ALIAS: &str = "slice_browser_text";
pub const SAVE_SLICE_STATE_TOOL: &str = "arroba.save_slice_state";
pub const SAVE_SLICE_STATE_TOOL_ALIAS: &str = "save_slice_state";
pub const CREATE_SLICE_BACKUP_TOOL: &str = "arroba.create_slice_backup";
pub const CREATE_SLICE_BACKUP_TOOL_ALIAS: &str = "create_slice_backup";

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
    pub allowed_handoff_schema_refs: Vec<String>,
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
pub struct ValidateWorkflowHandoffArgs {
    pub handoff_schema_ref: String,
    pub handoff_json: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ReadWorkflowTurnContextArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowConsoleWriteArgs {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListExtensionsArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestExtensionArgs {
    pub kind: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_body: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterMcpArgs {
    pub config: crate::mcp::ArrobaMcpServerConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant_to_current_agent: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterSkillPathArgs {
    pub path: std::path::PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant_to_current_agent: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterEnvironmentArgs {
    pub config: crate::script::ArrobaEnvironmentConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterScriptPathArgs {
    pub path: std::path::PathBuf,
    pub environment: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant_to_current_agent: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterConnectorPathArgs {
    pub path: std::path::PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant_to_current_agent: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterConnectorAdapterPathArgs {
    pub path: std::path::PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchRecallArgs {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_sequence: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryRecallArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_sequence: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_sequence: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SaveSliceStateArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slice_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CreateSliceBackupArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slice_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCredentialConfigInput {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<crate::config::UserCredentialSourceConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_hosts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_uses: Vec<crate::config::UserCredentialUse>,
    pub injection: crate::config::UserCredentialInjectionConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateGeneratedCredentialArgs {
    pub credential: RuntimeCredentialConfigInput,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator: Option<GeneratedCredentialSecretGeneratorArgs>,
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneratedCredentialSecretGeneratorArgs {
    #[serde(default = "default_generated_secret_kind")]
    pub kind: String,
    #[serde(default = "default_generated_secret_length")]
    pub length: usize,
    #[serde(default = "default_generated_secret_symbols")]
    pub symbols: bool,
    #[serde(default)]
    pub avoid_ambiguous: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestCredentialSecretArgs {
    pub credential: RuntimeCredentialConfigInput,
    pub prompt: RequestCredentialSecretPromptArgs,
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestCredentialSecretPromptArgs {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_length: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_sec: Option<u64>,
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
    #[serde(default = "default_http_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_http_max_response_bytes")]
    pub max_response_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendSecretToTerminalArgs {
    pub credential_id: String,
    #[serde(default = "default_append_newline")]
    pub append_newline: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasteSecretToSliceArgs {
    pub credential_id: String,
    #[serde(default)]
    pub submit: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestPopupChoiceArgs {
    pub id: String,
    pub label: String,
    pub reply: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<crate::session::RuntimeInteractionChoiceStyle>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestPopupCustomChoiceArgs {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_length: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestPopupArgs {
    pub message: String,
    pub choices: Vec<RequestPopupChoiceArgs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_choice: Option<RequestPopupCustomChoiceArgs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<crate::session::RuntimeInteractionLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_sec: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_on_timeout: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceScreenshotArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default)]
    pub return_image_base64: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceOcrArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceFindTextArgs {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceMouseArgs {
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_x: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_y: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceKeyboardArgs {
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceOpenUrlArgs {
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceBrowserFindArgs {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceBrowserFillArgs {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceBrowserClickArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceBrowserSubmitArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_id: Option<String>,
}

fn default_append_newline() -> bool {
    true
}

fn default_generated_secret_kind() -> String {
    "password".to_string()
}

fn default_generated_secret_length() -> usize {
    32
}

fn default_generated_secret_symbols() -> bool {
    true
}

fn default_http_method() -> String {
    "GET".to_string()
}

fn default_http_timeout_ms() -> u64 {
    30_000
}

fn default_http_max_response_bytes() -> u64 {
    1_048_576
}

pub fn extension_runtime_tool_specs() -> Vec<RuntimeToolSpec> {
    let canonical = vec![
        RuntimeToolSpec {
            name: LIST_EXTENSIONS_TOOL.to_string(),
            description: "List Arroba-managed extensions available in this workspace, including whether they are already granted to the current agent. Use this before requesting an extension.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "kind": {
                        "type": "string",
                        "enum": ["mcp", "skill", "script", "connector", "all"]
                    }
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: REQUEST_EXTENSION_TOOL.to_string(),
            description: "Request access to an Arroba-managed MCP, skill, script, or connector for the current agent. Script requests require an environment. Connector requests may include an allow safety level and credential handle.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["kind", "name"],
                "properties": {
                    "kind": {
                        "type": "string",
                        "enum": ["mcp", "skill", "script", "connector"]
                    },
                    "name": {"type": "string"},
                    "reason": {"type": "string"},
                    "return_body": {"type": "boolean"},
                    "environment": {"type": "string"},
                    "allow": {
                        "type": "string",
                        "enum": ["read", "write", "admin"]
                    },
                    "credential": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: REGISTER_MCP_TOOL.to_string(),
            description: "Register or update a global Arroba-managed MCP definition. Set grant_to_current_agent to also grant it to this agent in the same operation.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["config"],
                "properties": {
                    "config": {"type": "object"},
                    "grant_to_current_agent": {"type": "boolean"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: REGISTER_SKILL_PATH_TOOL.to_string(),
            description: "Register or update a global Arroba skill from a directory containing SKILL.md. Set grant_to_current_agent to also grant it to this agent in the same operation.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": {"type": "string"},
                    "grant_to_current_agent": {"type": "boolean"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: REGISTER_ENVIRONMENT_TOOL.to_string(),
            description: "Register or update a global Arroba script execution environment.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["config"],
                "properties": {
                    "config": {"type": "object"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: REGISTER_SCRIPT_PATH_TOOL.to_string(),
            description: "Register a global Arroba script extension from a Python or TypeScript file. Set grant_to_current_agent to also grant it to this agent in the same operation.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["path", "environment"],
                "properties": {
                    "path": {"type": "string"},
                    "environment": {"type": "string"},
                    "name": {"type": "string"},
                    "grant_to_current_agent": {"type": "boolean"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: REGISTER_CONNECTOR_PATH_TOOL.to_string(),
            description: "Register or update a global Arroba connector from connector YAML. Set grant_to_current_agent to also grant it to this agent in the same operation.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": {"type": "string"},
                    "grant_to_current_agent": {"type": "boolean"},
                    "credential": {"type": "string"},
                    "allow": {"type": "string", "enum": ["read", "write", "destructive"]}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: REGISTER_CONNECTOR_ADAPTER_PATH_TOOL.to_string(),
            description: "Register or update a global Arroba connector adapter from an adapter YAML/package.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
    ];
    canonical
}

pub fn recall_runtime_tool_specs() -> Vec<RuntimeToolSpec> {
    vec![
        RuntimeToolSpec {
            name: SEARCH_RECALL_TOOL.to_string(),
            description: "Search Arroba recall events for prior conversation, workflow, Git, and runtime records. Defaults to the current session; set scope to all only when broader recall is needed.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["query"],
                "properties": {
                    "query": {"type": "string"},
                    "mode": {
                        "type": "string",
                        "enum": ["keyword", "semantic", "agent"]
                    },
                    "scope": {
                        "type": "string",
                        "enum": ["current_session", "all"]
                    },
                    "session_id": {"type": "string"},
                    "agent_id": {"type": "string"},
                    "provider": {"type": "string"},
                    "model": {"type": "string"},
                    "workflow_id": {"type": "string"},
                    "kind": {"type": "string"},
                    "cursor": {"type": "string"},
                    "after_sequence": {"type": "integer", "minimum": 0},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 50}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: QUERY_RECALL_TOOL.to_string(),
            description: "Load structured Arroba recall events by metadata and sequence filters, typically to inspect context around an event returned by arroba.search_recall.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "scope": {
                        "type": "string",
                        "enum": ["current_session", "all"]
                    },
                    "session_id": {"type": "string"},
                    "agent_id": {"type": "string"},
                    "provider": {"type": "string"},
                    "model": {"type": "string"},
                    "workflow_id": {"type": "string"},
                    "kind": {"type": "string"},
                    "text": {"type": "string"},
                    "after_sequence": {"type": "integer", "minimum": 0},
                    "before_sequence": {"type": "integer", "minimum": 0},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 50}
                },
                "additionalProperties": false
            }),
        },
    ]
}

pub fn canonical_recall_tool_name(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        SEARCH_RECALL_TOOL
        | "arroba_search_recall"
        | "mcp__arroba__search_recall"
        | "mcp__arroba__arroba_search_recall" => Some(SEARCH_RECALL_TOOL),
        QUERY_RECALL_TOOL
        | "arroba_query_recall"
        | "mcp__arroba__query_recall"
        | "mcp__arroba__arroba_query_recall" => Some(QUERY_RECALL_TOOL),
        _ => None,
    }
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
            name: CREATE_GENERATED_CREDENTIAL_TOOL.to_string(),
            description: "Create or update a vault-backed Arroba credential handle with a kernel-generated random password. The generated secret is stored in the vault and is never returned to the model.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["credential"],
                "properties": {
                    "credential": credential_creation_schema(),
                    "generator": {
                        "type": "object",
                        "properties": {
                            "kind": {"type": "string", "enum": ["password"]},
                            "length": {"type": "integer", "minimum": 12, "maximum": 256},
                            "symbols": {"type": "boolean"},
                            "avoid_ambiguous": {"type": "boolean"}
                        },
                        "additionalProperties": false
                    },
                    "overwrite": {"type": "boolean"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: REQUEST_CREDENTIAL_SECRET_TOOL.to_string(),
            description: "Ask the user for a credential secret through a redacted Arroba interaction, then store it as a vault-backed credential handle. The typed secret is never returned to the model.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["credential", "prompt"],
                "properties": {
                    "credential": credential_creation_schema(),
                    "prompt": {
                        "type": "object",
                        "required": ["message"],
                        "properties": {
                            "title": {"type": "string"},
                            "message": {"type": "string"},
                            "placeholder": {"type": "string"},
                            "min_length": {"type": "integer", "minimum": 1},
                            "max_length": {"type": "integer", "minimum": 1},
                            "timeout_sec": {"type": "integer", "minimum": 1}
                        },
                        "additionalProperties": false
                    },
                    "overwrite": {"type": "boolean"}
                },
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
                    "body_json": {},
                    "timeout_ms": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Optional per-call timeout. Defaults to 30000."
                    },
                    "max_response_bytes": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Optional response body cap. Defaults to 1048576."
                    }
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
        RuntimeToolSpec {
            name: PASTE_SECRET_TO_SLICE_TOOL.to_string(),
            description: "Paste a browser credential into an Arroba slice browser field after validating the current browser target. The secret value is resolved inside the kernel and is not returned to the model.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["credential_id"],
                "properties": {
                    "credential_id": {"type": "string"},
                    "submit": {"type": "boolean", "description": "Press Enter after pasting. Defaults to false."},
                    "expected_host": {"type": "string", "description": "Optional expected current browser host. The paste fails before secret resolution if the browser is on a different host."},
                    "expected_url": {"type": "string", "description": "Optional expected current browser URL prefix. The paste fails before secret resolution if the browser URL does not start with this value."},
                    "selector": {"type": "string", "description": "Optional CSS selector for the intended fillable field."},
                    "field_id": {"type": "string", "description": "Optional field id returned by slice_browser_find; equivalent to selector."}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: REQUEST_POPUP_TOOL.to_string(),
            description: "Request a synchronous Arroba popup in the current agent pane. The tool call blocks until the user answers or a timeout/default resolves it, then returns the selected reply.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["message", "choices"],
                "properties": {
                    "title": {"type": "string"},
                    "message": {"type": "string"},
                    "level": {
                        "type": "string",
                        "enum": ["info", "warning", "critical"]
                    },
                    "timeout_sec": {"type": "integer", "minimum": 1},
                    "default_on_timeout": {"type": "string"},
                    "custom_choice": {
                        "type": "object",
                        "required": ["id", "label"],
                        "properties": {
                            "id": {"type": "string"},
                            "label": {"type": "string"},
                            "placeholder": {"type": "string"},
                            "min_length": {"type": "integer", "minimum": 1},
                            "max_length": {"type": "integer", "minimum": 1}
                        },
                        "additionalProperties": false
                    },
                    "choices": {
                        "type": "array",
                        "minItems": 2,
                        "items": {
                            "type": "object",
                            "required": ["id", "label", "reply"],
                            "properties": {
                                "id": {"type": "string"},
                                "label": {"type": "string"},
                                "reply": {"type": "string"},
                                "style": {
                                    "type": "string",
                                    "enum": ["primary", "secondary", "danger"]
                                }
                            },
                            "additionalProperties": false
                        }
                    }
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

fn credential_creation_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "required": ["id", "injection"],
        "properties": {
            "id": {"type": "string"},
            "description": {"type": "string"},
            "source": {
                "type": "object",
                "required": ["type", "key"],
                "properties": {
                    "type": {"type": "string", "enum": ["vault"]},
                    "key": {"type": "string"}
                },
                "additionalProperties": false
            },
            "allowed_hosts": {
                "type": "array",
                "items": {"type": "string"}
            },
            "allowed_uses": {
                "type": "array",
                "items": {
                    "type": "string",
                    "enum": ["http", "pty", "connector", "browser", "mcp"]
                }
            },
            "injection": {
                "type": "object",
                "required": ["kind"],
                "properties": {
                    "kind": {
                        "type": "string",
                        "enum": ["header", "query", "basic", "hmac", "pty", "browser"]
                    },
                    "name": {"type": "string"},
                    "value": {"type": "string"},
                    "username": {"type": "string"},
                    "timestamp_header": {"type": "string"},
                    "signature_header": {"type": "string"}
                },
                "additionalProperties": false
            }
        },
        "additionalProperties": false
    })
}

fn credential_alias_spec(spec: &RuntimeToolSpec) -> Option<RuntimeToolSpec> {
    let alias = match spec.name.as_str() {
        LIST_CREDENTIAL_HANDLES_TOOL => LIST_CREDENTIAL_HANDLES_TOOL_ALIAS,
        CREATE_GENERATED_CREDENTIAL_TOOL => CREATE_GENERATED_CREDENTIAL_TOOL_ALIAS,
        REQUEST_CREDENTIAL_SECRET_TOOL => REQUEST_CREDENTIAL_SECRET_TOOL_ALIAS,
        HTTP_REQUEST_WITH_CREDENTIAL_TOOL => HTTP_REQUEST_WITH_CREDENTIAL_TOOL_ALIAS,
        SEND_SECRET_TO_TERMINAL_TOOL => SEND_SECRET_TO_TERMINAL_TOOL_ALIAS,
        PASTE_SECRET_TO_SLICE_TOOL => PASTE_SECRET_TO_SLICE_TOOL_ALIAS,
        REQUEST_POPUP_TOOL => REQUEST_POPUP_TOOL_ALIAS,
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
        CREATE_GENERATED_CREDENTIAL_TOOL
        | CREATE_GENERATED_CREDENTIAL_TOOL_ALIAS
        | "arroba_create_generated_credential"
        | "mcp__arroba__create_generated_credential"
        | "mcp__arroba__arroba_create_generated_credential" => {
            Some(CREATE_GENERATED_CREDENTIAL_TOOL)
        }
        REQUEST_CREDENTIAL_SECRET_TOOL
        | REQUEST_CREDENTIAL_SECRET_TOOL_ALIAS
        | "arroba_request_credential_secret"
        | "mcp__arroba__request_credential_secret"
        | "mcp__arroba__arroba_request_credential_secret" => Some(REQUEST_CREDENTIAL_SECRET_TOOL),
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
        PASTE_SECRET_TO_SLICE_TOOL
        | PASTE_SECRET_TO_SLICE_TOOL_ALIAS
        | "arroba_paste_secret_to_slice"
        | "mcp__arroba__paste_secret_to_slice"
        | "mcp__arroba__arroba_paste_secret_to_slice" => Some(PASTE_SECRET_TO_SLICE_TOOL),
        REQUEST_POPUP_TOOL
        | REQUEST_POPUP_TOOL_ALIAS
        | "arroba_request_popup"
        | "mcp__arroba__request_popup"
        | "mcp__arroba__arroba_request_popup" => Some(REQUEST_POPUP_TOOL),
        _ => None,
    }
}

pub fn canonical_extension_tool_name(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        LIST_EXTENSIONS_TOOL
        | "arroba_list_extensions"
        | "mcp__arroba__list_extensions"
        | "mcp__arroba__arroba_list_extensions" => Some(LIST_EXTENSIONS_TOOL),
        REQUEST_EXTENSION_TOOL
        | "arroba_request_extension"
        | "mcp__arroba__request_extension"
        | "mcp__arroba__arroba_request_extension" => Some(REQUEST_EXTENSION_TOOL),
        REGISTER_MCP_TOOL
        | "arroba_register_mcp"
        | "mcp__arroba__register_mcp"
        | "mcp__arroba__arroba_register_mcp" => Some(REGISTER_MCP_TOOL),
        REGISTER_SKILL_PATH_TOOL
        | "arroba_register_skill_path"
        | "mcp__arroba__register_skill_path"
        | "mcp__arroba__arroba_register_skill_path" => Some(REGISTER_SKILL_PATH_TOOL),
        REGISTER_ENVIRONMENT_TOOL
        | "arroba_register_environment"
        | "mcp__arroba__register_environment"
        | "mcp__arroba__arroba_register_environment" => Some(REGISTER_ENVIRONMENT_TOOL),
        REGISTER_SCRIPT_PATH_TOOL
        | "arroba_register_script_path"
        | "mcp__arroba__register_script_path"
        | "mcp__arroba__arroba_register_script_path" => Some(REGISTER_SCRIPT_PATH_TOOL),
        REGISTER_CONNECTOR_PATH_TOOL
        | "arroba_register_connector_path"
        | "mcp__arroba__register_connector_path"
        | "mcp__arroba__arroba_register_connector_path" => Some(REGISTER_CONNECTOR_PATH_TOOL),
        REGISTER_CONNECTOR_ADAPTER_PATH_TOOL
        | "arroba_register_connector_adapter_path"
        | "mcp__arroba__register_connector_adapter_path"
        | "mcp__arroba__arroba_register_connector_adapter_path" => {
            Some(REGISTER_CONNECTOR_ADAPTER_PATH_TOOL)
        }
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidateAndSubmitWorkflowRunOutputArgs {
    pub workflow_output_json: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentAppActionArgs {
    pub action_id: String,
    #[serde(default)]
    pub input: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_token: Option<String>,
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
            name: VALIDATE_WORKFLOW_HANDOFF_TOOL.to_string(),
            description: "Validate workflow handoff JSON against an allowed handoff schema ref for the current workflow turn.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["handoff_schema_ref", "handoff_json"],
                "properties": {
                    "handoff_schema_ref": {"type": "string"},
                    "handoff_json": {"type": "string"},
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
            name: READ_WORKFLOW_TURN_CONTEXT_TOOL.to_string(),
            description: "Read the current workflow turn context, including invocation prompt and upstream handoff messages for this node run.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
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
        RuntimeToolSpec {
            name: AGENT_APP_ACTION_TOOL_QUALIFIED.to_string(),
            description: "Call a route-scoped Agent App action exposed by the current published workflow invocation. The action must be allowed by the matched endpoint route.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["action_id", "input"],
                "properties": {
                    "action_id": {"type": "string"},
                    "input": {"type": "object"},
                    "delivery_token": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
    ]
}

pub fn canonical_workflow_tool_name(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        ACK_WORKFLOW_TURN_TOOL => Some(ACK_WORKFLOW_TURN_TOOL),
        VALIDATE_WORKFLOW_HANDOFF_TOOL => Some(VALIDATE_WORKFLOW_HANDOFF_TOOL),
        VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL => {
            Some(VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL)
        }
        VALIDATE_AND_SUBMIT_INTERMEDIATE_WORKFLOW_RUN_OUTPUT_TOOL => {
            Some(VALIDATE_AND_SUBMIT_INTERMEDIATE_WORKFLOW_RUN_OUTPUT_TOOL)
        }
        READ_WORKFLOW_TURN_CONTEXT_TOOL => Some(READ_WORKFLOW_TURN_CONTEXT_TOOL),
        WORKFLOW_CONSOLE_READ_TOOL => Some(WORKFLOW_CONSOLE_READ_TOOL),
        WORKFLOW_CONSOLE_WRITE_TOOL => Some(WORKFLOW_CONSOLE_WRITE_TOOL),
        WORKFLOW_CONSOLE_CLEAR_TOOL => Some(WORKFLOW_CONSOLE_CLEAR_TOOL),
        AGENT_APP_ACTION_TOOL
        | AGENT_APP_ACTION_TOOL_QUALIFIED
        | "arroba_agent_app_action"
        | "mcp__arroba__agent_app_action"
        | "mcp__arroba__arroba_agent_app_action" => Some(AGENT_APP_ACTION_TOOL),
        _ => None,
    }
}
