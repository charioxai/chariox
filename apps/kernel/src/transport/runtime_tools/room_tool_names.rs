//! Transitional room names reuse the existing typed handlers and schemas.
use super::*;

pub(crate) fn room_name(meta: &str) -> Option<String> {
    let name = match meta {
        META_HISTORY_SEARCH_TOOL => "chariox.history.search".into(),
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
        if spec.name == "chariox.history.search" { return Some(spec); }
        if spec.name == "chariox.history.read" {
            spec.description = "Read a sanitized public event_ref returned by chariox.history.search. Room authorization and current protection are revalidated; no filesystem/blob fallback. An explicit other same-owner session_id requires live sudo.".into();
            spec.input_schema = serde_json::json!({"type":"object","required":["event_ref"],"properties":{"event_ref":{"type":"string"},"session_id":{"type":"string"}},"additionalProperties":false});
            return Some(spec);
        }
        if spec.name == "chariox.history.turn" {
            spec.description = "Read a room peer's sanitized public turn events. Private reasoning, hidden context and unknown legacy provenance are excluded.".into();
            return Some(spec);
        }
        spec.description = format!("{}: operate only in the current room. Read/message/run any peer; spawn regular agents and create workflows without an agent quota or a task plan. Mutations require self-created workflow objects or immutable direct-child creators; agent rename/delete require a direct child. Use chariox.commands.docs for command syntax. No sudo or capability authority is inherited.", spec.name);
        Some(spec)
    }).collect()
}

#[cfg(test)]
mod history_search_tests {
    use super::*;

    #[test]
    fn public_history_search_is_a_room_tool() {
        // MP-08 / MP-10 / MP-11, A09: fail first on the PR1 catalog.
        let specs = room_runtime_tool_specs();
        assert!(specs
            .iter()
            .any(|spec| spec.name == "chariox.history.search"));
    }
    #[test]
    fn public_history_protocol_453_schema_snapshot() {
        use sha2::{Digest, Sha256};
        assert_eq!(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, 472);
        let specs = room_runtime_tool_specs();
        let search = specs
            .iter()
            .find(|s| s.name == "chariox.history.search")
            .unwrap();
        let read = specs
            .iter()
            .find(|s| s.name == "chariox.history.read")
            .unwrap();
        let shape = serde_json::json!({"search":search.input_schema,"read":read.input_schema});
        let hash = format!("{:x}", Sha256::digest(serde_json::to_vec(&shape).unwrap()));
        assert_eq!(
            hash,
            "aa2ffa2de1e41d9c637974136141a4aa27815986b9138caf0019f72dc6291155"
        );
        for name in [
            "chariox.history.search",
            "chariox_history_search",
            "mcp__chariox__chariox_history_search",
        ] {
            assert_eq!(
                canonical_room_tool_name(name),
                Some(META_HISTORY_SEARCH_TOOL)
            );
        }
    }
}
