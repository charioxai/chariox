//! Transitional room names reuse the existing typed handlers and schemas.
use super::*;

pub(crate) fn room_name(meta: &str) -> Option<String> {
    let name = match meta {
        META_SESSION_OVERVIEW_TOOL => "chariox.room.overview".into(),
        META_RUN_COMMAND_TOOL => "chariox.room.run_command".into(),
        META_SEARCH_COMMANDS_TOOL => "chariox.commands.search".into(),
        META_LIST_COMMANDS_TOOL => "chariox.commands.list".into(),
        META_COMMAND_DOCS_TOOL => "chariox.commands.docs".into(),
        META_SEARCH_GUIDES_TOOL => "chariox.guides.search".into(),
        META_LIST_GUIDES_TOOL => "chariox.guides.list".into(),
        META_READ_GUIDE_TOOL => "chariox.guides.read".into(),
        META_TURN_OVERVIEW_TOOL => "chariox.history.turn".into(),
        META_TURN_BLOB_TOOL => "chariox.history.read".into(),
        META_SUBSCRIBE_TRACE_TOOL => "chariox.trace.subscribe".into(),
        META_POLL_TRACE_TOOL => "chariox.trace.poll".into(),
        META_WAIT_TRACE_TOOL => "chariox.trace.wait".into(),
        META_UNSUBSCRIBE_TRACE_TOOL => "chariox.trace.unsubscribe".into(),
        _ if meta.starts_with("chariox.meta.workflow_code.")
            || meta.starts_with("chariox.meta.workflow_registry.") =>
        {
            meta.replacen("chariox.meta.", "chariox.", 1)
        }
        _ => return None,
    };
    Some(name)
}

pub(crate) fn canonical_room_tool_name(tool: &str) -> Option<&'static str> {
    // Names are provider-neutral; official harnesses may normalize dots to underscores.
    let tool = tool.strip_prefix("mcp__chariox__").unwrap_or(tool);
    static NAMES: std::sync::OnceLock<Vec<(&'static str, String, String)>> =
        std::sync::OnceLock::new();
    NAMES
        .get_or_init(|| {
            meta_runtime_tool_specs()
                .into_iter()
                .filter_map(|spec| {
                    let name = room_name(&spec.name)?;
                    let canonical =
                        super::meta_tool_names::canonical_legacy_meta_tool_name(&spec.name)?;
                    Some((canonical, name.clone(), name.replace('.', "_")))
                })
                .collect()
        })
        .iter()
        .find_map(|(canonical, dotted, normalized)| {
            (tool == dotted || tool == normalized).then_some(*canonical)
        })
}

pub(crate) fn room_runtime_tool_specs() -> Vec<RuntimeToolSpec> {
    meta_runtime_tool_specs().into_iter().filter_map(|mut spec| {
        spec.name = room_name(&spec.name)?;
        spec.description = format!("{}: operate only in the current room. Read/message/prompt any peer; workflow execution bindings require self or immutable direct children. Spawn regular agents and create workflows without an agent quota or a task plan. Mutations require self-created workflow objects or immutable direct-child creators; agent rename/delete require a direct child. Use chariox.commands.docs for command syntax. No sudo or capability authority is inherited.", spec.name);
        Some(spec)
    }).collect()
}
