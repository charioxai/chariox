use serde::{Deserialize, Serialize};

use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendAgentMessageArgs {
    pub agent: String,
    /// Another room of the same owner on this kernel; the sender's room when absent.
    #[serde(default)]
    pub room: Option<String>,
    /// Another kernel of the same owner, by id or name; `room` then names a room there.
    #[serde(default)]
    pub kernel: Option<String>,
    pub message: String,
    pub origin_prompt_id: String,
    #[serde(default)]
    pub urgent: bool,
    #[serde(default)]
    pub reply_requested: bool,
    #[serde(default)]
    pub attachments: Vec<crate::session::PromptAttachment>,
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromoteAgentMessageArgs {
    /// The `message_id` a queued send answered.
    pub message_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetSessionAgentArgs {
    pub agent: String,
}

pub fn agent_messaging_runtime_tool_specs() -> Vec<RuntimeToolSpec> {
    vec![
        RuntimeToolSpec {
            name: LIST_SESSION_AGENTS_TOOL.to_string(),
            description: "List the agents in the current Chariox session with their stable ids, unique aliases, provider configuration, runtime availability, queue depth, extension capabilities, and local or remote placement. Use this before addressing another agent.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: GET_SESSION_AGENT_TOOL.to_string(),
            description: "Read the safe configuration and current runtime status of one existing agent in the current Chariox session. Address the target by its unique alias, agent ref, or agent id. Provider credentials and resume state are never returned.".to_string(),
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
            description: "Send a visible, human-readable message to another existing agent in the current Chariox session. Include the origin_prompt_id supplied in this turn's hidden context. Address the target by its unique alias, agent ref, or agent id. Keep message as natural-language text instead of serializing an envelope as JSON; send images and files through attachments. Messages enter the durable event inbox. A message waits in the receiver's queue and is delivered in its next turn, whatever starts that turn, unless you set urgent to true, which steers the running turn or starts one if the receiver is idle. A queued message can be promoted to urgent with chariox.promote_agent_message, or by sending it again with the same idempotency_key and urgent true. No reply is requested unless reply_requested is true; never send a courtesy reply. Explicitly stopped receivers remain stopped with pending events. This tool never creates agents. Use chariox.list_session_agents first when the target is not already known.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["agent", "message", "origin_prompt_id"],
                "properties": {
                    "agent": {
                        "type": "string",
                        "description": "Unique agent alias (with or without @), agent ref, or agent id in the current session, or in `room` when given."
                    },
                    "room": {
                        "type": "string",
                        "description": "Optional id of another room of the same owner on this kernel. Its ordinary agents are addressable; sub-agents are not."
                    },
                    "kernel": {
                        "type": "string",
                        "description": "Optional id or name of another kernel of the same owner; `room` then names a room on that kernel. To answer a message from another kernel, use its sender_kernel_id, sender_room_id and sender_agent_id."
                    },
                    "message": {
                        "type": "string",
                        "description": "Natural-language text to send to the target agent. Do not wrap it in a JSON envelope."
                    },
                    "origin_prompt_id": {
                        "type": "string",
                        "description": "Exact kernel prompt ID supplied for this turn in hidden context."
                    },
                    "attachments": {
                        "type": "array",
                        "description": "Optional images or files displayed above the message, using the same attachment format as a user prompt.",
                        "items": {
                            "type": "object",
                            "required": ["url", "mime"],
                            "properties": {
                                "url": {
                                    "type": "string",
                                    "description": "Attachment URL or data URL."
                                },
                                "mime": {
                                    "type": "string",
                                    "description": "Attachment MIME type."
                                },
                                "filename": {
                                    "type": "string",
                                    "description": "Optional display filename."
                                },
                                "contents_base64": {
                                    "type": "string",
                                    "description": "Optional base64-encoded attachment contents."
                                }
                            },
                            "additionalProperties": false
                        }
                    },
                    "urgent": { "type": "boolean", "default": false, "description": "Steer the receiver's running turn, or start a turn if it is idle. When false, the message is queued for the receiver's next turn and never starts or steers one." },
                    "reply_requested": { "type": "boolean", "default": false, "description": "Request one correlated reply. Default messages require no courtesy response." },
                    "idempotency_key": {
                        "type": "string",
                        "description": "Optional stable key for safely retrying the same send without creating another prompt."
                    }
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: PROMOTE_AGENT_MESSAGE_TOOL.to_string(),
            description: "Make a message you sent urgent after it queued: the same message steers the receiver's running turn, or starts a turn if the receiver is idle, and is delivered once. Only the sender can promote it. A message that was already delivered is left as it is and the result says so.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["message_id"],
                "properties": {
                    "message_id": {
                        "type": "string",
                        "description": "The message_id the queued send returned."
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
        | "chariox_list_session_agents"
        | "mcp__chariox__list_session_agents"
        | "mcp__chariox__chariox_list_session_agents" => Some(LIST_SESSION_AGENTS_TOOL),
        GET_SESSION_AGENT_TOOL
        | "chariox_get_session_agent"
        | "mcp__chariox__get_session_agent"
        | "mcp__chariox__chariox_get_session_agent" => Some(GET_SESSION_AGENT_TOOL),
        SEND_AGENT_MESSAGE_TOOL
        | "chariox_send_agent_message"
        | "mcp__chariox__send_agent_message"
        | "mcp__chariox__chariox_send_agent_message" => Some(SEND_AGENT_MESSAGE_TOOL),
        PROMOTE_AGENT_MESSAGE_TOOL
        | "chariox_promote_agent_message"
        | "mcp__chariox__promote_agent_message"
        | "mcp__chariox__chariox_promote_agent_message" => Some(PROMOTE_AGENT_MESSAGE_TOOL),
        _ => None,
    }
}
