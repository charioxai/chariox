use serde::{Deserialize, Serialize};

use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendAgentMessageArgs {
    pub agent: String,
    pub message: String,
}

pub fn agent_messaging_runtime_tool_specs() -> Vec<RuntimeToolSpec> {
    vec![RuntimeToolSpec {
        name: SEND_AGENT_MESSAGE_TOOL.to_string(),
        description: "Send a visible prompt to another existing agent in the current Arroba session. Address the target by its unique alias, agent ref, or agent id. The prompt starts immediately when the target is idle and enters its normal queue when it is busy. This tool never creates agents.".to_string(),
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
    }]
}

pub fn canonical_agent_messaging_tool_name(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        SEND_AGENT_MESSAGE_TOOL
        | "arroba_send_agent_message"
        | "mcp__arroba__send_agent_message"
        | "mcp__arroba__arroba_send_agent_message" => Some(SEND_AGENT_MESSAGE_TOOL),
        _ => None,
    }
}
