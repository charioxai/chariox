use super::*;

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
        META_WORKFLOW_CODE_PACKAGE_EXPORT_TOOL
        | "arroba_meta_workflow_code_package_export"
        | "mcp__arroba__meta_workflow_code_package_export"
        | "mcp__arroba__arroba_meta_workflow_code_package_export" => {
            Some(META_WORKFLOW_CODE_PACKAGE_EXPORT_TOOL)
        }
        META_WORKFLOW_CODE_PACKAGE_IMPORT_TOOL
        | "arroba_meta_workflow_code_package_import"
        | "mcp__arroba__meta_workflow_code_package_import"
        | "mcp__arroba__arroba_meta_workflow_code_package_import" => {
            Some(META_WORKFLOW_CODE_PACKAGE_IMPORT_TOOL)
        }
        META_WORKFLOW_CODE_SOURCE_EXPORT_TOOL
        | "arroba_meta_workflow_code_source_export"
        | "mcp__arroba__meta_workflow_code_source_export"
        | "mcp__arroba__arroba_meta_workflow_code_source_export" => {
            Some(META_WORKFLOW_CODE_SOURCE_EXPORT_TOOL)
        }
        META_WORKFLOW_CODE_SOURCE_EXPORT_DIRECTORY_TOOL
        | META_WORKFLOW_CODE_SOURCE_EXPORT_DIR_ALIAS_TOOL
        | "arroba_meta_workflow_code_source_export_directory"
        | "mcp__arroba__meta_workflow_code_source_export_directory"
        | "mcp__arroba__arroba_meta_workflow_code_source_export_directory"
        | "arroba_meta_workflow_code_source_export_dir"
        | "mcp__arroba__meta_workflow_code_source_export_dir"
        | "mcp__arroba__arroba_meta_workflow_code_source_export_dir" => {
            Some(META_WORKFLOW_CODE_SOURCE_EXPORT_DIRECTORY_TOOL)
        }
        META_WORKFLOW_CODE_CANVAS_CONTRACT_TOOL
        | "arroba_meta_workflow_code_canvas_contract"
        | "mcp__arroba__meta_workflow_code_canvas_contract"
        | "mcp__arroba__arroba_meta_workflow_code_canvas_contract" => {
            Some(META_WORKFLOW_CODE_CANVAS_CONTRACT_TOOL)
        }
        META_WORKFLOW_REGISTRY_LIST_TOOL
        | "arroba_meta_workflow_registry_list"
        | "mcp__arroba__meta_workflow_registry_list"
        | "mcp__arroba__arroba_meta_workflow_registry_list" => {
            Some(META_WORKFLOW_REGISTRY_LIST_TOOL)
        }
        META_WORKFLOW_REGISTRY_GET_TOOL
        | "arroba_meta_workflow_registry_get"
        | "mcp__arroba__meta_workflow_registry_get"
        | "mcp__arroba__arroba_meta_workflow_registry_get" => Some(META_WORKFLOW_REGISTRY_GET_TOOL),
        META_WORKFLOW_REGISTRY_ADD_TOOL
        | "arroba_meta_workflow_registry_add"
        | "mcp__arroba__meta_workflow_registry_add"
        | "mcp__arroba__arroba_meta_workflow_registry_add" => Some(META_WORKFLOW_REGISTRY_ADD_TOOL),
        META_WORKFLOW_REGISTRY_ADD_FROM_WORKFLOW_TOOL
        | "arroba_meta_workflow_registry_add_from_workflow"
        | "mcp__arroba__meta_workflow_registry_add_from_workflow"
        | "mcp__arroba__arroba_meta_workflow_registry_add_from_workflow" => {
            Some(META_WORKFLOW_REGISTRY_ADD_FROM_WORKFLOW_TOOL)
        }
        META_WORKFLOW_REGISTRY_DELETE_TOOL
        | "arroba_meta_workflow_registry_delete"
        | "mcp__arroba__meta_workflow_registry_delete"
        | "mcp__arroba__arroba_meta_workflow_registry_delete" => {
            Some(META_WORKFLOW_REGISTRY_DELETE_TOOL)
        }
        META_WORKFLOW_REGISTRY_LOAD_TOOL
        | "arroba_meta_workflow_registry_load"
        | "mcp__arroba__meta_workflow_registry_load"
        | "mcp__arroba__arroba_meta_workflow_registry_load" => {
            Some(META_WORKFLOW_REGISTRY_LOAD_TOOL)
        }
        META_WORKFLOW_REGISTRY_RUN_TOOL
        | "arroba_meta_workflow_registry_run"
        | "mcp__arroba__meta_workflow_registry_run"
        | "mcp__arroba__arroba_meta_workflow_registry_run" => Some(META_WORKFLOW_REGISTRY_RUN_TOOL),
        META_RESOLVE_RUNTIME_INTERACTION_TOOL
        | "arroba_meta_resolve_runtime_interaction"
        | "mcp__arroba__meta_resolve_runtime_interaction"
        | "mcp__arroba__arroba_meta_resolve_runtime_interaction" => {
            Some(META_RESOLVE_RUNTIME_INTERACTION_TOOL)
        }
        _ => None,
    }
}
