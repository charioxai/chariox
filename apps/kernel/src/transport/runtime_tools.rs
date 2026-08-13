use serde::{Deserialize, Serialize};
use serde_json::Value;

mod agent_messaging_tools;
mod credential_tools;
mod extension_tools;
mod meta_tool_args;
mod meta_tool_names;
mod meta_tool_specs;
mod recall_tools;
mod slice_tools;
mod validation;
mod workflow_tools;
mod workspace_live_sync_tools;
pub use agent_messaging_tools::*;
pub use credential_tools::*;
pub use extension_tools::*;
pub use meta_tool_args::*;
pub use meta_tool_names::canonical_meta_tool_name;
pub use meta_tool_specs::meta_runtime_tool_specs;
pub use recall_tools::*;
pub use slice_tools::{canonical_slice_tool_name, slice_runtime_tool_specs};
pub use validation::{validate_json_output_schema, validate_workflow_handoff_schema};
pub use workflow_tools::*;
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
pub const AGENT_APP_ACTION_TOOL_QUALIFIED: &str = "chariox.agent_app_action";
pub const LIST_EXTENSIONS_TOOL: &str = "chariox.list_extensions";
pub const REQUEST_EXTENSION_TOOL: &str = "chariox.request_extension";
pub const REGISTER_MCP_TOOL: &str = "chariox.register_mcp";
pub const REGISTER_SKILL_PATH_TOOL: &str = "chariox.register_skill_path";
pub const REGISTER_ENVIRONMENT_TOOL: &str = "chariox.register_environment";
pub const REGISTER_SCRIPT_PATH_TOOL: &str = "chariox.register_script_path";
pub const REGISTER_CONNECTOR_PATH_TOOL: &str = "chariox.register_connector_path";
pub const REGISTER_CONNECTOR_ADAPTER_PATH_TOOL: &str = "chariox.register_connector_adapter_path";
pub const SEARCH_RECALL_TOOL: &str = "chariox.search_recall";
pub const QUERY_RECALL_TOOL: &str = "chariox.query_recall";
pub const LIST_CREDENTIAL_HANDLES_TOOL: &str = "chariox.list_credential_handles";
pub const LIST_CREDENTIAL_HANDLES_TOOL_ALIAS: &str = "list_credential_handles";
pub const CREATE_GENERATED_CREDENTIAL_TOOL: &str = "chariox.create_generated_credential";
pub const CREATE_GENERATED_CREDENTIAL_TOOL_ALIAS: &str = "create_generated_credential";
pub const REQUEST_CREDENTIAL_SECRET_TOOL: &str = "chariox.request_credential_secret";
pub const REQUEST_CREDENTIAL_SECRET_TOOL_ALIAS: &str = "request_credential_secret";
pub const HTTP_REQUEST_WITH_CREDENTIAL_TOOL: &str = "chariox.http_request_with_credential";
pub const HTTP_REQUEST_WITH_CREDENTIAL_TOOL_ALIAS: &str = "http_request_with_credential";
pub const SEND_SECRET_TO_TERMINAL_TOOL: &str = "chariox.send_secret_to_terminal";
pub const SEND_SECRET_TO_TERMINAL_TOOL_ALIAS: &str = "send_secret_to_terminal";
pub const PASTE_SECRET_TO_SLICE_TOOL: &str = "chariox.paste_secret_to_slice";
pub const PASTE_SECRET_TO_SLICE_TOOL_ALIAS: &str = "paste_secret_to_slice";
pub const MANAGE_CREDENTIAL_VAULT_TOOL: &str = "chariox.manage_credential_vault";
pub const MANAGE_CREDENTIAL_VAULT_TOOL_ALIAS: &str = "manage_credential_vault";
pub const REQUEST_POPUP_TOOL: &str = "chariox.request_popup";
pub const REQUEST_POPUP_TOOL_ALIAS: &str = "request_popup";
pub const LIST_SESSION_AGENTS_TOOL: &str = "chariox.list_session_agents";
pub const GET_SESSION_AGENT_TOOL: &str = "chariox.get_session_agent";
pub const SEND_AGENT_MESSAGE_TOOL: &str = "chariox.send_agent_message";
pub const SLICE_SCREEN_STATUS_TOOL: &str = "chariox.slice_screen_status";
pub const SLICE_SCREEN_STATUS_TOOL_ALIAS: &str = "slice_screen_status";
pub const SLICE_SCREENSHOT_TOOL: &str = "chariox.slice_screenshot";
pub const SLICE_SCREENSHOT_TOOL_ALIAS: &str = "slice_screenshot";
pub const SLICE_OCR_TOOL: &str = "chariox.slice_ocr";
pub const SLICE_OCR_TOOL_ALIAS: &str = "slice_ocr";
pub const SLICE_FIND_TEXT_TOOL: &str = "chariox.slice_find_text";
pub const SLICE_FIND_TEXT_TOOL_ALIAS: &str = "slice_find_text";
pub const SLICE_MOUSE_TOOL: &str = "chariox.slice_mouse";
pub const SLICE_MOUSE_TOOL_ALIAS: &str = "slice_mouse";
pub const SLICE_KEYBOARD_TOOL: &str = "chariox.slice_keyboard";
pub const SLICE_KEYBOARD_TOOL_ALIAS: &str = "slice_keyboard";
pub const SLICE_OPEN_URL_TOOL: &str = "chariox.slice_open_url";
pub const SLICE_OPEN_URL_TOOL_ALIAS: &str = "slice_open_url";
pub const SLICE_BROWSER_STATUS_TOOL: &str = "chariox.slice_browser_status";
pub const SLICE_BROWSER_STATUS_TOOL_ALIAS: &str = "slice_browser_status";
pub const SLICE_BROWSER_FIND_TOOL: &str = "chariox.slice_browser_find";
pub const SLICE_BROWSER_FIND_TOOL_ALIAS: &str = "slice_browser_find";
pub const SLICE_BROWSER_FILL_TOOL: &str = "chariox.slice_browser_fill";
pub const SLICE_BROWSER_FILL_TOOL_ALIAS: &str = "slice_browser_fill";
pub const SLICE_BROWSER_CLICK_TOOL: &str = "chariox.slice_browser_click";
pub const SLICE_BROWSER_CLICK_TOOL_ALIAS: &str = "slice_browser_click";
pub const SLICE_BROWSER_SUBMIT_TOOL: &str = "chariox.slice_browser_submit";
pub const SLICE_BROWSER_SUBMIT_TOOL_ALIAS: &str = "slice_browser_submit";
pub const SLICE_BROWSER_DIALOG_TOOL: &str = "chariox.slice_browser_dialog";
pub const SLICE_BROWSER_DIALOG_TOOL_ALIAS: &str = "slice_browser_dialog";
pub const SLICE_BROWSER_TEXT_TOOL: &str = "chariox.slice_browser_text";
pub const SLICE_BROWSER_TEXT_TOOL_ALIAS: &str = "slice_browser_text";
pub const SLICE_BROWSER_WAIT_FOR_TEXT_TOOL: &str = "chariox.slice_browser_wait_for_text";
pub const SLICE_BROWSER_WAIT_FOR_TEXT_TOOL_ALIAS: &str = "slice_browser_wait_for_text";
pub const SLICE_BROWSER_WAIT_FOR_SELECTOR_TOOL: &str = "chariox.slice_browser_wait_for_selector";
pub const SLICE_BROWSER_WAIT_FOR_SELECTOR_TOOL_ALIAS: &str = "slice_browser_wait_for_selector";
pub const SLICE_BROWSER_WAIT_FOR_IDLE_TOOL: &str = "chariox.slice_browser_wait_for_idle";
pub const SLICE_BROWSER_WAIT_FOR_IDLE_TOOL_ALIAS: &str = "slice_browser_wait_for_idle";
pub const META_SESSION_OVERVIEW_TOOL: &str = "chariox.meta.session_overview";
pub const META_SEARCH_COMMANDS_TOOL: &str = "chariox.meta.search_commands";
pub const META_LIST_COMMANDS_TOOL: &str = "chariox.meta.list_commands";
pub const META_COMMAND_DOCS_TOOL: &str = "chariox.meta.command_docs";
pub const META_SEARCH_GUIDES_TOOL: &str = "chariox.meta.search_guides";
pub const META_LIST_GUIDES_TOOL: &str = "chariox.meta.list_guides";
pub const META_READ_GUIDE_TOOL: &str = "chariox.meta.read_guide";
pub const META_RUN_COMMAND_TOOL: &str = "chariox.meta.run_command";
pub const META_LIST_EVENTS_TOOL: &str = "chariox.meta.list_events";
pub const META_READ_EVENT_TOOL: &str = "chariox.meta.read_event";
pub const META_ACK_EVENT_TOOL: &str = "chariox.meta.ack_event";
pub const META_TURN_OVERVIEW_TOOL: &str = "chariox.meta.turn_overview";
pub const META_TURN_BLOB_TOOL: &str = "chariox.meta.turn_blob";
pub const META_SUBSCRIBE_TRACE_TOOL: &str = "chariox.meta.subscribe_trace";
pub const META_POLL_TRACE_TOOL: &str = "chariox.meta.poll_trace";
pub const META_WAIT_TRACE_TOOL: &str = "chariox.meta.wait_trace";
pub const META_UNSUBSCRIBE_TRACE_TOOL: &str = "chariox.meta.unsubscribe_trace";
pub const META_SUBSCRIBE_EVENTS_TOOL: &str = "chariox.meta.subscribe_events";
pub const META_UNSUBSCRIBE_EVENTS_TOOL: &str = "chariox.meta.unsubscribe_events";
pub const META_LIST_SUBSCRIPTIONS_TOOL: &str = "chariox.meta.list_subscriptions";
pub const META_RESOLVE_RUNTIME_INTERACTION_TOOL: &str = "chariox.meta.resolve_runtime_interaction";
pub const META_READ_TASK_TOOL: &str = "chariox.meta.read_task";
pub const META_UPDATE_TASK_TOOL: &str = "chariox.meta.update_task";
pub const META_READ_PLAN_TOOL: &str = "chariox.meta.read_plan";
pub const META_UPDATE_PLAN_TOOL: &str = "chariox.meta.update_plan";
pub const META_COMPLETE_TASK_TOOL: &str = "chariox.meta.complete_task";
pub const META_MARK_BLOCKED_TOOL: &str = "chariox.meta.mark_blocked";
pub const META_WORKFLOW_CODE_CREATE_TOOL: &str = "chariox.meta.workflow_code.create";
pub const META_WORKFLOW_CODE_READ_TOOL: &str = "chariox.meta.workflow_code.read";
pub const META_WORKFLOW_CODE_LIST_TOOL: &str = "chariox.meta.workflow_code.list";
pub const META_WORKFLOW_CODE_UPDATE_TOOL: &str = "chariox.meta.workflow_code.update";
pub const META_WORKFLOW_CODE_DELETE_TOOL: &str = "chariox.meta.workflow_code.delete";
pub const META_WORKFLOW_CODE_VALIDATE_TOOL: &str = "chariox.meta.workflow_code.validate";
pub const META_WORKFLOW_CODE_APPLY_TOOL: &str = "chariox.meta.workflow_code.apply";
pub const META_WORKFLOW_CODE_RUN_TOOL: &str = "chariox.meta.workflow_code.run";
pub const META_WORKFLOW_CODE_EXPORT_TOOL: &str = "chariox.meta.workflow_code.export";
pub const META_WORKFLOW_CODE_IMPORT_TOOL: &str = "chariox.meta.workflow_code.import";
pub const META_WORKFLOW_CODE_PACKAGE_EXPORT_TOOL: &str =
    "chariox.meta.workflow_code.package_export";
pub const META_WORKFLOW_CODE_PACKAGE_IMPORT_TOOL: &str =
    "chariox.meta.workflow_code.package_import";
pub const META_WORKFLOW_CODE_SOURCE_EXPORT_TOOL: &str = "chariox.meta.workflow_code.source_export";
pub const META_WORKFLOW_CODE_SOURCE_EXPORT_DIRECTORY_TOOL: &str =
    "chariox.meta.workflow_code.source_export_directory";
pub const META_WORKFLOW_CODE_SOURCE_EXPORT_DIR_ALIAS_TOOL: &str =
    "chariox.meta.workflow_code.source_export_dir";
pub const META_WORKFLOW_CODE_CANVAS_CONTRACT_TOOL: &str =
    "chariox.meta.workflow_code.canvas_contract";
pub const META_WORKFLOW_REGISTRY_LIST_TOOL: &str = "chariox.meta.workflow_registry.list";
pub const META_WORKFLOW_REGISTRY_GET_TOOL: &str = "chariox.meta.workflow_registry.get";
pub const META_WORKFLOW_REGISTRY_ADD_TOOL: &str = "chariox.meta.workflow_registry.add";
pub const META_WORKFLOW_REGISTRY_ADD_FROM_WORKFLOW_TOOL: &str =
    "chariox.meta.workflow_registry.add_from_workflow";
pub const META_WORKFLOW_REGISTRY_DELETE_TOOL: &str = "chariox.meta.workflow_registry.delete";
pub const META_WORKFLOW_REGISTRY_LOAD_TOOL: &str = "chariox.meta.workflow_registry.load";
pub const META_WORKFLOW_REGISTRY_RUN_TOOL: &str = "chariox.meta.workflow_registry.run";

pub const META_EVENT_KIND_AGENT_TURN_COMPLETED: &str = "agent.turn.completed";
pub const META_EVENT_KIND_AGENT_TURN_FAILED: &str = "agent.turn.failed";
pub const META_EVENT_KIND_AGENTS_SPAWNED: &str = "agents.spawned";
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
    META_EVENT_KIND_AGENTS_SPAWNED,
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
