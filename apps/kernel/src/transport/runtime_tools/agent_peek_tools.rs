//! MP-08 / MP-10 / MP-11 peek: an agent sees into another agent of its room the way the pane
//! shows it, a little at a time.
use serde::{Deserialize, Serialize};

use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeekAgentArgs {
    pub agent: String,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub before: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadAgentEntryArgs {
    pub agent: String,
    pub entry_id: String,
    #[serde(default)]
    pub offset: usize,
    #[serde(default)]
    pub limit: Option<usize>,
}

pub fn agent_peek_runtime_tool_specs() -> Vec<RuntimeToolSpec> {
    vec![
        RuntimeToolSpec {
            name: PEEK_AGENT_TOOL.to_string(),
            description: "See what another agent of this room is doing: its latest transcript entries as the pane titles them (prompt, assistant message, thinking, tool call, wait), each with its status, start time and duration (a running entry's duration is how long it has been running). Titles only; read one entry's content with chariox.read_agent_entry. Pass `before` from a result for older entries.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["agent"],
                "properties": {
                    "agent": {"type": "string", "description": "Unique agent alias (with or without @), agent ref, or agent id in the current room."},
                    "limit": {"type": "integer", "minimum": 1, "maximum": crate::local::MAX_AGENT_PEEK_ENTRIES, "description": "Entries to list, newest last; 10 when absent."},
                    "before": {"type": "string", "description": "The `before` of an earlier result: entries older than those."}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: READ_AGENT_ENTRY_TOOL.to_string(),
            description: "Read one entry of another agent's transcript by pages: the entry_id from chariox.peek_agent, from character `offset` (0 first, then the result's next_offset) at most `limit` characters. A tool call reads as its input, then its output.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["agent", "entry_id"],
                "properties": {
                    "agent": {"type": "string", "description": "Unique agent alias (with or without @), agent ref, or agent id in the current room."},
                    "entry_id": {"type": "string"},
                    "offset": {"type": "integer", "minimum": 0},
                    "limit": {"type": "integer", "minimum": 1, "maximum": crate::local::MAX_AGENT_PEEK_PAGE_CHARS, "description": "Characters per page; 4000 when absent."}
                },
                "additionalProperties": false
            }),
        },
    ]
}

pub fn canonical_agent_peek_tool_name(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        PEEK_AGENT_TOOL
        | "chariox_peek_agent"
        | "mcp__chariox__peek_agent"
        | "mcp__chariox__chariox_peek_agent" => Some(PEEK_AGENT_TOOL),
        READ_AGENT_ENTRY_TOOL
        | "chariox_read_agent_entry"
        | "mcp__chariox__read_agent_entry"
        | "mcp__chariox__chariox_read_agent_entry" => Some(READ_AGENT_ENTRY_TOOL),
        _ => None,
    }
}
