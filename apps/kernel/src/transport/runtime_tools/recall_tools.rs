use serde::{Deserialize, Serialize};

use super::*;

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

pub fn recall_runtime_tool_specs() -> Vec<RuntimeToolSpec> {
    vec![
        RuntimeToolSpec {
            name: SEARCH_RECALL_TOOL.to_string(),
            description: "Search Chariox recall events for prior conversation, workflow, Git, and runtime records. Defaults to the current session; set scope to all only when broader recall is needed.".to_string(),
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
            description: "Load structured Chariox recall events by metadata and sequence filters, typically to inspect context around an event returned by chariox.search_recall.".to_string(),
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
        | "chariox_search_recall"
        | "mcp__chariox__search_recall"
        | "mcp__chariox__chariox_search_recall" => Some(SEARCH_RECALL_TOOL),
        QUERY_RECALL_TOOL
        | "chariox_query_recall"
        | "mcp__chariox__query_recall"
        | "mcp__chariox__chariox_query_recall" => Some(QUERY_RECALL_TOOL),
        _ => None,
    }
}
