use serde::{Deserialize, Serialize};

use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendAgentMessageArgs {
    pub agent: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetSessionAgentArgs {
    pub agent: String,
}

pub fn agent_messaging_runtime_tool_specs() -> Vec<RuntimeToolSpec> {
    vec![
        RuntimeToolSpec {
            name: LIST_SESSION_AGENTS_TOOL.to_string(),
            description: "List the agents in the current Arroba session with their stable ids, unique aliases, provider configuration, runtime availability, queue depth, extension capabilities, and local or remote placement. Use this before addressing another agent.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: GET_SESSION_AGENT_TOOL.to_string(),
            description: "Read the safe configuration and current runtime status of one existing agent in the current Arroba session. Address the target by its unique alias, agent ref, or agent id. Provider credentials and resume state are never returned.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["agent"],
                "properties": {
                    "agent": {
                        "type": "string",
                        "description": "Unique agent alias (with or without @), agent ref, or agent id in the current session."
                    }
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: SEND_AGENT_MESSAGE_TOOL.to_string(),
            description: "Send a visible prompt to another existing agent in the current Arroba session. Address the target by its unique alias, agent ref, or agent id. The prompt starts immediately when the target is idle and enters its normal queue when it is busy. This tool never creates agents. Use arroba.list_session_agents first when the target is not already known.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["agent", "message"],
                "properties": {
                    "agent": {
                        "type": "string",
                        "description": "Unique agent alias (with or without @), agent ref, or agent id in the current session."
                    },
                    "message": {
                        "type": "string",
                        "description": "The message that should become the target agent's next visible prompt."
                    }
                },
                "additionalProperties": false
            }),
        },
    ]
}

pub fn canonical_agent_messaging_tool_name(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        LIST_SESSION_AGENTS_TOOL
        | "arroba_list_session_agents"
        | "mcp__arroba__list_session_agents"
        | "mcp__arroba__arroba_list_session_agents" => Some(LIST_SESSION_AGENTS_TOOL),
        GET_SESSION_AGENT_TOOL
        | "arroba_get_session_agent"
        | "mcp__arroba__get_session_agent"
        | "mcp__arroba__arroba_get_session_agent" => Some(GET_SESSION_AGENT_TOOL),
        SEND_AGENT_MESSAGE_TOOL
        | "arroba_send_agent_message"
        | "mcp__arroba__send_agent_message"
        | "mcp__arroba__arroba_send_agent_message" => Some(SEND_AGENT_MESSAGE_TOOL),
        _ => None,
    }
}
