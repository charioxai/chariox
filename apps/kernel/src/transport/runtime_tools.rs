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
pub const MANAGE_CREDENTIAL_VAULT_TOOL: &str = "arroba.manage_credential_vault";
pub const MANAGE_CREDENTIAL_VAULT_TOOL_ALIAS: &str = "manage_credential_vault";
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
pub const SLICE_BROWSER_WAIT_FOR_TEXT_TOOL: &str = "arroba.slice_browser_wait_for_text";
pub const SLICE_BROWSER_WAIT_FOR_TEXT_TOOL_ALIAS: &str = "slice_browser_wait_for_text";
pub const SLICE_BROWSER_WAIT_FOR_SELECTOR_TOOL: &str = "arroba.slice_browser_wait_for_selector";
pub const SLICE_BROWSER_WAIT_FOR_SELECTOR_TOOL_ALIAS: &str = "slice_browser_wait_for_selector";
pub const SLICE_BROWSER_WAIT_FOR_IDLE_TOOL: &str = "arroba.slice_browser_wait_for_idle";
pub const SLICE_BROWSER_WAIT_FOR_IDLE_TOOL_ALIAS: &str = "slice_browser_wait_for_idle";
pub const META_SESSION_OVERVIEW_TOOL: &str = "arroba.meta.session_overview";
pub const META_SEARCH_COMMANDS_TOOL: &str = "arroba.meta.search_commands";
pub const META_LIST_COMMANDS_TOOL: &str = "arroba.meta.list_commands";
pub const META_COMMAND_DOCS_TOOL: &str = "arroba.meta.command_docs";
pub const META_SEARCH_GUIDES_TOOL: &str = "arroba.meta.search_guides";
pub const META_LIST_GUIDES_TOOL: &str = "arroba.meta.list_guides";
pub const META_READ_GUIDE_TOOL: &str = "arroba.meta.read_guide";
pub const META_RUN_COMMAND_TOOL: &str = "arroba.meta.run_command";
pub const META_LIST_EVENTS_TOOL: &str = "arroba.meta.list_events";
pub const META_READ_EVENT_TOOL: &str = "arroba.meta.read_event";
pub const META_ACK_EVENT_TOOL: &str = "arroba.meta.ack_event";
pub const META_TURN_OVERVIEW_TOOL: &str = "arroba.meta.turn_overview";
pub const META_TURN_BLOB_TOOL: &str = "arroba.meta.turn_blob";
pub const META_SUBSCRIBE_TRACE_TOOL: &str = "arroba.meta.subscribe_trace";
pub const META_POLL_TRACE_TOOL: &str = "arroba.meta.poll_trace";
pub const META_WAIT_TRACE_TOOL: &str = "arroba.meta.wait_trace";
pub const META_UNSUBSCRIBE_TRACE_TOOL: &str = "arroba.meta.unsubscribe_trace";
pub const META_SUBSCRIBE_EVENTS_TOOL: &str = "arroba.meta.subscribe_events";
pub const META_UNSUBSCRIBE_EVENTS_TOOL: &str = "arroba.meta.unsubscribe_events";
pub const META_LIST_SUBSCRIPTIONS_TOOL: &str = "arroba.meta.list_subscriptions";
pub const META_RESOLVE_RUNTIME_INTERACTION_TOOL: &str = "arroba.meta.resolve_runtime_interaction";
pub const META_READ_TASK_TOOL: &str = "arroba.meta.read_task";
pub const META_UPDATE_TASK_TOOL: &str = "arroba.meta.update_task";
pub const META_READ_PLAN_TOOL: &str = "arroba.meta.read_plan";
pub const META_UPDATE_PLAN_TOOL: &str = "arroba.meta.update_plan";
pub const META_COMPLETE_TASK_TOOL: &str = "arroba.meta.complete_task";
pub const META_MARK_BLOCKED_TOOL: &str = "arroba.meta.mark_blocked";
pub const META_WORKFLOW_CODE_CREATE_TOOL: &str = "arroba.meta.workflow_code.create";
pub const META_WORKFLOW_CODE_READ_TOOL: &str = "arroba.meta.workflow_code.read";
pub const META_WORKFLOW_CODE_LIST_TOOL: &str = "arroba.meta.workflow_code.list";
pub const META_WORKFLOW_CODE_UPDATE_TOOL: &str = "arroba.meta.workflow_code.update";
pub const META_WORKFLOW_CODE_DELETE_TOOL: &str = "arroba.meta.workflow_code.delete";
pub const META_WORKFLOW_CODE_VALIDATE_TOOL: &str = "arroba.meta.workflow_code.validate";
pub const META_WORKFLOW_CODE_APPLY_TOOL: &str = "arroba.meta.workflow_code.apply";
pub const META_WORKFLOW_CODE_RUN_TOOL: &str = "arroba.meta.workflow_code.run";
pub const META_WORKFLOW_CODE_EXPORT_TOOL: &str = "arroba.meta.workflow_code.export";
pub const META_WORKFLOW_CODE_IMPORT_TOOL: &str = "arroba.meta.workflow_code.import";

pub const META_EVENT_KIND_AGENT_TURN_COMPLETED: &str = "agent.turn.completed";
pub const META_EVENT_KIND_AGENT_TURN_FAILED: &str = "agent.turn.failed";
pub const META_EVENT_KIND_RUNTIME_INTERACTION: &str = "runtime.interaction";
pub const META_EVENT_KIND_WORKFLOW_RUN_STARTED: &str = "workflow.run.started";
pub const META_EVENT_KIND_WORKFLOW_RUN_UPDATED: &str = "workflow.run.updated";
pub const META_EVENT_KIND_WORKFLOW_RUN_COMPLETED: &str = "workflow.run.completed";
pub const META_EVENT_KIND_WORKFLOW_RUN_FAILED: &str = "workflow.run.failed";
pub const META_EVENT_KIND_WORKFLOW_RUN_CANCELLED: &str = "workflow.run.cancelled";
pub const META_EVENT_KIND_WORKFLOW_OUTPUT_FINAL: &str = "workflow.output.final";
pub const META_EVENT_KIND_WORKFLOW_OUTPUT_INTERMEDIATE: &str = "workflow.output.intermediate";

pub const META_EVENT_KINDS: &[&str] = &[
    META_EVENT_KIND_AGENT_TURN_COMPLETED,
    META_EVENT_KIND_AGENT_TURN_FAILED,
    META_EVENT_KIND_RUNTIME_INTERACTION,
    META_EVENT_KIND_WORKFLOW_RUN_STARTED,
    META_EVENT_KIND_WORKFLOW_RUN_UPDATED,
    META_EVENT_KIND_WORKFLOW_RUN_COMPLETED,
    META_EVENT_KIND_WORKFLOW_RUN_FAILED,
    META_EVENT_KIND_WORKFLOW_RUN_CANCELLED,
    META_EVENT_KIND_WORKFLOW_OUTPUT_FINAL,
    META_EVENT_KIND_WORKFLOW_OUTPUT_INTERMEDIATE,
];

pub fn is_known_metaagent_event_kind(kind: &str) -> bool {
    META_EVENT_KINDS.contains(&kind)
}

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ManageCredentialVaultArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
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
pub struct MetaSessionOverviewArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_workflows: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_events: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MetaCommandSearchArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mutates: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MetaCommandListArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mutates: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaCommandDocsArgs {
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MetaGuideSearchArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MetaGuideListArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaReadGuideArgs {
    pub guide: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaRunCommandArgs {
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MetaListEventsArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaReadEventArgs {
    pub event_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MetaAckEventArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub up_to_sequence: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MetaTurnOverviewArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turns_back: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaTurnBlobArgs {
    pub blob_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaSubscribeTraceArgs {
    pub agent_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MetaPollTraceArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaUnsubscribeTraceArgs {
    pub subscription_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaSubscribeEventsArgs {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaUnsubscribeEventsArgs {
    pub subscription_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaResolveRuntimeInteractionArgs {
    pub interaction_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choice_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MetaReadTaskArgs {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaUpdateTaskArgs {
    pub markdown: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MetaReadPlanArgs {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaUpdatePlanArgs {
    pub markdown: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MetaCompleteTaskArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaMarkBlockedArgs {
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaWorkflowCodeCreateArgs {
    pub name: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<crate::workflow_code::WorkflowCodeLanguage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaWorkflowCodeReadArgs {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MetaWorkflowCodeListArgs {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaWorkflowCodeUpdateArgs {
    pub name: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<crate::workflow_code::WorkflowCodeLanguage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaWorkflowCodeDeleteArgs {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MetaWorkflowCodeValidateArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<crate::workflow_code::WorkflowCodeLanguage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_path: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_rebindings: Vec<crate::workflow_code::WorkflowCodeProviderRebinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MetaWorkflowCodeApplyArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<crate::workflow_code::WorkflowCodeLanguage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_path: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_rebindings: Vec<crate::workflow_code::WorkflowCodeProviderRebinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MetaWorkflowCodeRunArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<crate::workflow_code::WorkflowCodeLanguage>,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_path: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_rebindings: Vec<crate::workflow_code::WorkflowCodeProviderRebinding>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetaWorkflowCodeExportArgs {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetaWorkflowCodeImportArgs {
    pub package: crate::workflow_code::WorkflowCodeArtifactPackage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overwrite: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_path: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceBrowserWaitForTextArgs {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceBrowserWaitForSelectorArgs {
    pub selector: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceBrowserWaitForIdleArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
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

pub fn meta_runtime_tool_specs() -> Vec<RuntimeToolSpec> {
    vec![
        RuntimeToolSpec {
            name: META_SESSION_OVERVIEW_TOOL.to_string(),
            description: "Return a compact overview of the current session for the metaagent: owned agents, agent status, workflow state, pending interactions, and event counts.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "include_workflows": {"type": "boolean"},
                    "include_events": {"type": "boolean"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_SEARCH_COMMANDS_TOOL.to_string(),
            description: "Search Arroba commands available to this metaagent by natural-language goal, name, usage, intent, tag, scope, mutation behavior, or metaagent policy.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "tag": {"type": "string"},
                    "scope": {"type": "string", "enum": ["session", "global", "external"]},
                    "mutates": {"type": "boolean"},
                    "policy": {"type": "string", "enum": ["allow", "approval", "deny"]},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 100}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_LIST_COMMANDS_TOOL.to_string(),
            description: "List Arroba commands available to this metaagent, with optional filtering by tag, scope, mutation behavior, or policy.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "tag": {"type": "string"},
                    "scope": {"type": "string", "enum": ["session", "global", "external"]},
                    "mutates": {"type": "boolean"},
                    "policy": {"type": "string", "enum": ["allow", "approval", "deny"]},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 100}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_COMMAND_DOCS_TOOL.to_string(),
            description: "Return exact usage, examples, tags, scope, mutation behavior, and policy for one Arroba command.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["command"],
                "properties": {
                    "command": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_SEARCH_GUIDES_TOOL.to_string(),
            description: "Search concise Arroba operational guides for workflows, agent apps, events, supervision, and common failures.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "tag": {"type": "string"},
                    "command": {"type": "string"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 100}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_LIST_GUIDES_TOOL.to_string(),
            description: "List concise Arroba operational guides, optionally filtered by tag or command reference.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "tag": {"type": "string"},
                    "command": {"type": "string"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 100}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_READ_GUIDE_TOOL.to_string(),
            description: "Read one Arroba operational guide by id or exact title, including its Markdown body.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["guide"],
                "properties": {
                    "guide": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_RUN_COMMAND_TOOL.to_string(),
            description: "Run one allowed Arroba command inside this session as the metaagent. Session creation, cross-session targeting, and self-approval are denied.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["command"],
                "properties": {
                    "command": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_LIST_EVENTS_TOOL.to_string(),
            description: "List metaagent event inbox records. Event prompts are visible runtime prompts; this tool is for replay and detail lookup.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "limit": {"type": "integer", "minimum": 1, "maximum": 100},
                    "status": {"type": "string"},
                    "kind": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_READ_EVENT_TOOL.to_string(),
            description: "Read full detail for one metaagent event by event id.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["event_id"],
                "properties": {"event_id": {"type": "string"}},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_ACK_EVENT_TOOL.to_string(),
            description: "Acknowledge one or more metaagent events for bookkeeping and replay control.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "event_id": {"type": "string"},
                    "event_ids": {"type": "array", "items": {"type": "string"}},
                    "up_to_sequence": {"type": "integer", "minimum": 0}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_TURN_OVERVIEW_TOOL.to_string(),
            description: "Return an ordered overview of a turn trace: assistant messages, reasoning entries, tool calls, tool results, status, and errors.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_ref": {"type": "string"},
                    "turn_ref": {"type": "string"},
                    "turns_back": {"type": "integer", "minimum": 0},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 200}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_TURN_BLOB_TOOL.to_string(),
            description: "Return exact content for a selected turn blob when policy allows it.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["blob_id"],
                "properties": {"blob_id": {"type": "string"}},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_SUBSCRIBE_TRACE_TOOL.to_string(),
            description: "Attach this metaagent to the live terminal stream for one owned regular agent. Subscribe before prompting the worker so provider output is routed to the supervision stream.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["agent_ref"],
                "properties": {
                    "agent_ref": {"type": "string"},
                    "mode": {"type": "string", "enum": ["compact", "verbose"]}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_POLL_TRACE_TOOL.to_string(),
            description: "Drain currently buffered live trace records from a metaagent supervision stream without waiting. Compact mode returns summaries and short excerpts; verbose mode returns capped raw text. Use wait_trace for normal worker supervision.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "subscription_id": {"type": "string"},
                    "agent_ref": {"type": "string"},
                    "mode": {"type": "string", "enum": ["compact", "verbose"]},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 100}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_WAIT_TRACE_TOOL.to_string(),
            description: "Wait briefly for live worker trace records, then drain them. Prefer this after prompting a worker: it blocks until activity, worker output, completion, error, or timeout instead of returning an empty snapshot immediately.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "subscription_id": {"type": "string"},
                    "agent_ref": {"type": "string"},
                    "mode": {"type": "string", "enum": ["compact", "verbose"]},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 100},
                    "wait_ms": {"type": "integer", "minimum": 1, "maximum": 60000},
                    "until": {"type": "string", "enum": ["any", "activity", "worker_output", "completion", "error"]}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_UNSUBSCRIBE_TRACE_TOOL.to_string(),
            description: "Detach a metaagent live trace subscription and discard any pending compact stream records for it.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["subscription_id"],
                "properties": {"subscription_id": {"type": "string"}},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_SUBSCRIBE_EVENTS_TOOL.to_string(),
            description: format!(
                "Subscribe the metaagent to an optional session event. Valid event kinds: {}.",
                META_EVENT_KINDS.join(", ")
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["kind"],
                "properties": {
                    "kind": {"type": "string", "enum": META_EVENT_KINDS},
                    "filter": {"type": "object"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_UNSUBSCRIBE_EVENTS_TOOL.to_string(),
            description: "Remove an optional metaagent event subscription. Required agent turn and interaction subscriptions cannot be removed.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["subscription_id"],
                "properties": {"subscription_id": {"type": "string"}},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_LIST_SUBSCRIPTIONS_TOOL.to_string(),
            description: "List required and optional event subscriptions for this metaagent.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_READ_TASK_TOOL.to_string(),
            description: "Read this metaagent's kernel-managed task document and status. Returns status none when no task exists.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_UPDATE_TASK_TOOL.to_string(),
            description: "Update this metaagent's kernel-managed task markdown. Creates the task if it does not exist.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["markdown"],
                "properties": {
                    "markdown": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_READ_PLAN_TOOL.to_string(),
            description: "Read this metaagent's kernel-managed plan markdown and task status.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_UPDATE_PLAN_TOOL.to_string(),
            description: "Update this metaagent's kernel-managed plan markdown. Creates an empty active task if needed.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["markdown"],
                "properties": {
                    "markdown": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_COMPLETE_TASK_TOOL.to_string(),
            description: "Mark this metaagent's active task completed with an optional summary.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "summary": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_MARK_BLOCKED_TOOL.to_string(),
            description: "Mark this metaagent's task blocked with the concrete reason progress cannot continue.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["reason"],
                "properties": {
                    "reason": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_WORKFLOW_CODE_CREATE_TOOL.to_string(),
            description: "Create a saved workflow-code artifact in this session from JS/TS source after kernel compilation and validation. node_path is optional; the kernel discovers Node.js when omitted.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["name", "source"],
                "properties": {
                    "name": {"type": "string"},
                    "source": {"type": "string"},
                    "language": {"type": "string", "enum": ["java_script", "typescript", "type_script"]},
                    "node_path": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_WORKFLOW_CODE_READ_TOOL.to_string(),
            description: "Read one saved workflow-code artifact, including source, compiled workflow definition, and validation metadata.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["name"],
                "properties": {"name": {"type": "string"}},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_WORKFLOW_CODE_LIST_TOOL.to_string(),
            description: "List saved workflow-code artifacts visible to this session.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_WORKFLOW_CODE_UPDATE_TOOL.to_string(),
            description: "Update a saved workflow-code artifact after recompiling and validating the supplied JS/TS source.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["name", "source"],
                "properties": {
                    "name": {"type": "string"},
                    "source": {"type": "string"},
                    "language": {"type": "string", "enum": ["java_script", "typescript", "type_script"]},
                    "node_path": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_WORKFLOW_CODE_DELETE_TOOL.to_string(),
            description: "Delete one saved workflow-code artifact visible to this session.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["name"],
                "properties": {"name": {"type": "string"}},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_WORKFLOW_CODE_VALIDATE_TOOL.to_string(),
            description: "Validate workflow-code without mutating session workflow state. Pass either saved artifact name or inline source; node_path is optional for inline source.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string"},
                    "source": {"type": "string"},
                    "language": {"type": "string", "enum": ["java_script", "typescript", "type_script"]},
                    "node_path": {"type": "string"},
                    "provider_rebindings": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["node", "provider"],
                            "properties": {
                                "node": {"type": "string"},
                                "provider": {"type": "string"},
                                "model": {"type": "string"},
                                "effort": {"type": "string"},
                                "account_profile": {"type": "string"}
                            },
                            "additionalProperties": false
                        }
                    }
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_WORKFLOW_CODE_APPLY_TOOL.to_string(),
            description: "Apply saved or inline workflow-code into the current session. Applying creates a new workflow with fresh kernel ids and generated agents as needed.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string"},
                    "source": {"type": "string"},
                    "language": {"type": "string", "enum": ["java_script", "typescript", "type_script"]},
                    "node_path": {"type": "string"},
                    "provider_rebindings": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["node", "provider"],
                            "properties": {
                                "node": {"type": "string"},
                                "provider": {"type": "string"},
                                "model": {"type": "string"},
                                "effort": {"type": "string"},
                                "account_profile": {"type": "string"}
                            },
                            "additionalProperties": false
                        }
                    }
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_WORKFLOW_CODE_RUN_TOOL.to_string(),
            description: "Apply saved or inline workflow-code into the current session and invoke one endpoint. endpoint may be a script endpoint handle or a kernel endpoint ref; when omitted, the script must define exactly one endpoint.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["prompt"],
                "properties": {
                    "name": {"type": "string"},
                    "source": {"type": "string"},
                    "language": {"type": "string", "enum": ["java_script", "typescript", "type_script"]},
                    "prompt": {"type": "string"},
                    "endpoint": {"type": "string"},
                    "queue": {"type": "string"},
                    "node_path": {"type": "string"},
                    "provider_rebindings": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["node", "provider"],
                            "properties": {
                                "node": {"type": "string"},
                                "provider": {"type": "string"},
                                "model": {"type": "string"},
                                "effort": {"type": "string"},
                                "account_profile": {"type": "string"}
                            },
                            "additionalProperties": false
                        }
                    }
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_WORKFLOW_CODE_EXPORT_TOOL.to_string(),
            description: "Export a saved workflow-code artifact as a portable package without local filesystem paths.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["name"],
                "properties": {
                    "name": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_WORKFLOW_CODE_IMPORT_TOOL.to_string(),
            description: "Import a portable workflow-code package after checking package integrity and validating the embedded workflow definition on this kernel. name overrides the package name; overwrite replaces an existing saved artifact with the target name.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["package"],
                "properties": {
                    "package": {"type": "object"},
                    "name": {"type": "string"},
                    "overwrite": {"type": "boolean"},
                    "node_path": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_RESOLVE_RUNTIME_INTERACTION_TOOL.to_string(),
            description: "Resolve a kernel-owned runtime interaction for one of this user's regular agents. A metaagent can never resolve its own interactions.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["interaction_id"],
                "properties": {
                    "interaction_id": {"type": "string"},
                    "choice_id": {"type": "string"},
                    "input": {"type": "string"}
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

pub fn canonical_meta_tool_name(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        META_SESSION_OVERVIEW_TOOL
        | "arroba_meta_session_overview"
        | "mcp__arroba__meta_session_overview"
        | "mcp__arroba__arroba_meta_session_overview" => Some(META_SESSION_OVERVIEW_TOOL),
        META_SEARCH_COMMANDS_TOOL
        | "arroba_meta_search_commands"
        | "mcp__arroba__meta_search_commands"
        | "mcp__arroba__arroba_meta_search_commands" => Some(META_SEARCH_COMMANDS_TOOL),
        META_LIST_COMMANDS_TOOL
        | "arroba_meta_list_commands"
        | "mcp__arroba__meta_list_commands"
        | "mcp__arroba__arroba_meta_list_commands" => Some(META_LIST_COMMANDS_TOOL),
        META_COMMAND_DOCS_TOOL
        | "arroba_meta_command_docs"
        | "mcp__arroba__meta_command_docs"
        | "mcp__arroba__arroba_meta_command_docs" => Some(META_COMMAND_DOCS_TOOL),
        META_SEARCH_GUIDES_TOOL
        | "arroba_meta_search_guides"
        | "mcp__arroba__meta_search_guides"
        | "mcp__arroba__arroba_meta_search_guides" => Some(META_SEARCH_GUIDES_TOOL),
        META_LIST_GUIDES_TOOL
        | "arroba_meta_list_guides"
        | "mcp__arroba__meta_list_guides"
        | "mcp__arroba__arroba_meta_list_guides" => Some(META_LIST_GUIDES_TOOL),
        META_READ_GUIDE_TOOL
        | "arroba_meta_read_guide"
        | "mcp__arroba__meta_read_guide"
        | "mcp__arroba__arroba_meta_read_guide" => Some(META_READ_GUIDE_TOOL),
        META_RUN_COMMAND_TOOL
        | "arroba_meta_run_command"
        | "mcp__arroba__meta_run_command"
        | "mcp__arroba__arroba_meta_run_command" => Some(META_RUN_COMMAND_TOOL),
        META_LIST_EVENTS_TOOL
        | "arroba_meta_list_events"
        | "mcp__arroba__meta_list_events"
        | "mcp__arroba__arroba_meta_list_events" => Some(META_LIST_EVENTS_TOOL),
        META_READ_EVENT_TOOL
        | "arroba_meta_read_event"
        | "mcp__arroba__meta_read_event"
        | "mcp__arroba__arroba_meta_read_event" => Some(META_READ_EVENT_TOOL),
        META_ACK_EVENT_TOOL
        | "arroba_meta_ack_event"
        | "mcp__arroba__meta_ack_event"
        | "mcp__arroba__arroba_meta_ack_event" => Some(META_ACK_EVENT_TOOL),
        META_TURN_OVERVIEW_TOOL
        | "arroba_meta_turn_overview"
        | "mcp__arroba__meta_turn_overview"
        | "mcp__arroba__arroba_meta_turn_overview" => Some(META_TURN_OVERVIEW_TOOL),
        META_TURN_BLOB_TOOL
        | "arroba_meta_turn_blob"
        | "mcp__arroba__meta_turn_blob"
        | "mcp__arroba__arroba_meta_turn_blob" => Some(META_TURN_BLOB_TOOL),
        META_SUBSCRIBE_TRACE_TOOL
        | "arroba_meta_subscribe_trace"
        | "mcp__arroba__meta_subscribe_trace"
        | "mcp__arroba__arroba_meta_subscribe_trace" => Some(META_SUBSCRIBE_TRACE_TOOL),
        META_POLL_TRACE_TOOL
        | "arroba_meta_poll_trace"
        | "mcp__arroba__meta_poll_trace"
        | "mcp__arroba__arroba_meta_poll_trace" => Some(META_POLL_TRACE_TOOL),
        META_WAIT_TRACE_TOOL
        | "arroba_meta_wait_trace"
        | "mcp__arroba__meta_wait_trace"
        | "mcp__arroba__arroba_meta_wait_trace" => Some(META_WAIT_TRACE_TOOL),
        META_UNSUBSCRIBE_TRACE_TOOL
        | "arroba_meta_unsubscribe_trace"
        | "mcp__arroba__meta_unsubscribe_trace"
        | "mcp__arroba__arroba_meta_unsubscribe_trace" => Some(META_UNSUBSCRIBE_TRACE_TOOL),
        META_SUBSCRIBE_EVENTS_TOOL
        | "arroba_meta_subscribe_events"
        | "mcp__arroba__meta_subscribe_events"
        | "mcp__arroba__arroba_meta_subscribe_events" => Some(META_SUBSCRIBE_EVENTS_TOOL),
        META_UNSUBSCRIBE_EVENTS_TOOL
        | "arroba_meta_unsubscribe_events"
        | "mcp__arroba__meta_unsubscribe_events"
        | "mcp__arroba__arroba_meta_unsubscribe_events" => Some(META_UNSUBSCRIBE_EVENTS_TOOL),
        META_LIST_SUBSCRIPTIONS_TOOL
        | "arroba_meta_list_subscriptions"
        | "mcp__arroba__meta_list_subscriptions"
        | "mcp__arroba__arroba_meta_list_subscriptions" => Some(META_LIST_SUBSCRIPTIONS_TOOL),
        META_READ_TASK_TOOL
        | "arroba_meta_read_task"
        | "mcp__arroba__meta_read_task"
        | "mcp__arroba__arroba_meta_read_task" => Some(META_READ_TASK_TOOL),
        META_UPDATE_TASK_TOOL
        | "arroba_meta_update_task"
        | "mcp__arroba__meta_update_task"
        | "mcp__arroba__arroba_meta_update_task" => Some(META_UPDATE_TASK_TOOL),
        META_READ_PLAN_TOOL
        | "arroba_meta_read_plan"
        | "mcp__arroba__meta_read_plan"
        | "mcp__arroba__arroba_meta_read_plan" => Some(META_READ_PLAN_TOOL),
        META_UPDATE_PLAN_TOOL
        | "arroba_meta_update_plan"
        | "mcp__arroba__meta_update_plan"
        | "mcp__arroba__arroba_meta_update_plan" => Some(META_UPDATE_PLAN_TOOL),
        META_COMPLETE_TASK_TOOL
        | "arroba_meta_complete_task"
        | "mcp__arroba__meta_complete_task"
        | "mcp__arroba__arroba_meta_complete_task" => Some(META_COMPLETE_TASK_TOOL),
        META_MARK_BLOCKED_TOOL
        | "arroba_meta_mark_blocked"
        | "mcp__arroba__meta_mark_blocked"
        | "mcp__arroba__arroba_meta_mark_blocked" => Some(META_MARK_BLOCKED_TOOL),
        META_WORKFLOW_CODE_CREATE_TOOL
        | "arroba_meta_workflow_code_create"
        | "mcp__arroba__meta_workflow_code_create"
        | "mcp__arroba__arroba_meta_workflow_code_create" => Some(META_WORKFLOW_CODE_CREATE_TOOL),
        META_WORKFLOW_CODE_READ_TOOL
        | "arroba_meta_workflow_code_read"
        | "mcp__arroba__meta_workflow_code_read"
        | "mcp__arroba__arroba_meta_workflow_code_read" => Some(META_WORKFLOW_CODE_READ_TOOL),
        META_WORKFLOW_CODE_LIST_TOOL
        | "arroba_meta_workflow_code_list"
        | "mcp__arroba__meta_workflow_code_list"
        | "mcp__arroba__arroba_meta_workflow_code_list" => Some(META_WORKFLOW_CODE_LIST_TOOL),
        META_WORKFLOW_CODE_UPDATE_TOOL
        | "arroba_meta_workflow_code_update"
        | "mcp__arroba__meta_workflow_code_update"
        | "mcp__arroba__arroba_meta_workflow_code_update" => Some(META_WORKFLOW_CODE_UPDATE_TOOL),
        META_WORKFLOW_CODE_DELETE_TOOL
        | "arroba_meta_workflow_code_delete"
        | "mcp__arroba__meta_workflow_code_delete"
        | "mcp__arroba__arroba_meta_workflow_code_delete" => Some(META_WORKFLOW_CODE_DELETE_TOOL),
        META_WORKFLOW_CODE_VALIDATE_TOOL
        | "arroba_meta_workflow_code_validate"
        | "mcp__arroba__meta_workflow_code_validate"
        | "mcp__arroba__arroba_meta_workflow_code_validate" => {
            Some(META_WORKFLOW_CODE_VALIDATE_TOOL)
        }
        META_WORKFLOW_CODE_APPLY_TOOL
        | "arroba_meta_workflow_code_apply"
        | "mcp__arroba__meta_workflow_code_apply"
        | "mcp__arroba__arroba_meta_workflow_code_apply" => Some(META_WORKFLOW_CODE_APPLY_TOOL),
        META_WORKFLOW_CODE_RUN_TOOL
        | "arroba_meta_workflow_code_run"
        | "mcp__arroba__meta_workflow_code_run"
        | "mcp__arroba__arroba_meta_workflow_code_run" => Some(META_WORKFLOW_CODE_RUN_TOOL),
        META_WORKFLOW_CODE_EXPORT_TOOL
        | "arroba_meta_workflow_code_export"
        | "mcp__arroba__meta_workflow_code_export"
        | "mcp__arroba__arroba_meta_workflow_code_export" => Some(META_WORKFLOW_CODE_EXPORT_TOOL),
        META_WORKFLOW_CODE_IMPORT_TOOL
        | "arroba_meta_workflow_code_import"
        | "mcp__arroba__meta_workflow_code_import"
        | "mcp__arroba__arroba_meta_workflow_code_import" => Some(META_WORKFLOW_CODE_IMPORT_TOOL),
        META_RESOLVE_RUNTIME_INTERACTION_TOOL
        | "arroba_meta_resolve_runtime_interaction"
        | "mcp__arroba__meta_resolve_runtime_interaction"
        | "mcp__arroba__arroba_meta_resolve_runtime_interaction" => {
            Some(META_RESOLVE_RUNTIME_INTERACTION_TOOL)
        }
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
                    "field_id": {"type": "string", "description": "Optional opaque field id returned by slice_browser_find or slice_browser_status."}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: MANAGE_CREDENTIAL_VAULT_TOOL.to_string(),
            description: "Check, lock, or request the Arroba Vault unlock/extend popup for the current session. Passphrases and secrets are never returned to the model.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["status", "lock", "popup"],
                        "description": "Defaults to popup. status returns locked/unlocked metadata; lock clears in-memory vault keys; popup asks the user to unlock, extend, lock, or dismiss."
                    }
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
        MANAGE_CREDENTIAL_VAULT_TOOL => MANAGE_CREDENTIAL_VAULT_TOOL_ALIAS,
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
        MANAGE_CREDENTIAL_VAULT_TOOL
        | MANAGE_CREDENTIAL_VAULT_TOOL_ALIAS
        | "arroba_manage_credential_vault"
        | "mcp__arroba__manage_credential_vault"
        | "mcp__arroba__arroba_manage_credential_vault" => Some(MANAGE_CREDENTIAL_VAULT_TOOL),
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
