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
            name: OWNER_VIEW_TOOL.to_string(),
            description: "Watch the owner's hosted Web Navigator and Settings. The owner sees who is watching and can end the view. Use watch to attach, read for the latest presentation, and leave to detach.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["operation"],
                "properties": { "operation": { "enum": ["watch", "read", "leave"] } },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: LIST_SESSION_AGENTS_TOOL.to_string(),
            description: "List Room agents (compact rows) and tasks, newest first; get_session_agent shows one agent. include_history adds finished tasks and settled obligations; task_id, a task's. Resume with next_*_cursor.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agents_cursor": {"type":"string","maxLength":512},
                    "tasks_cursor": {"type":"integer","minimum":1},
                    "include_history": {"type":"boolean","default":false},
                    "task_id": {"type":"string"},
                    "obligations_cursor": {"type":"integer","minimum":1}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: GET_SESSION_AGENT_TOOL.to_string(),
            description: "Read the safe configuration and current runtime status of one existing agent in the current Chariox session. Includes job summaries for yourself and your same-owner direct children. Address the target by its unique alias, agent ref, or agent id. Provider credentials and resume state are never returned.".to_string(),
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
            description: "Send a visible, human-readable message to another existing agent in the current Chariox session. Include the origin_prompt_id supplied in this turn's hidden context. Address the target by its unique alias, agent ref, or agent id. Keep message as plain text, not a JSON envelope; send images and files as attachments. Without urgent, it is delivered in the receiver's next turn, whatever starts that turn. chariox.promote_agent_message, or sending again with the same idempotency_key and urgent true, makes a queued message urgent. No reply is requested unless reply_requested is true; to answer a request, send its sender a message beginning with Reply to message <sequence>: and your answer; never send a courtesy reply. A stopped receiver stays stopped. This tool never creates agents. Use chariox.list_session_agents first when the target is not already known.".to_string(),
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
                        "description": "Optional id of another room of the same owner on this kernel. Its agents, sub-agents included, are addressable when you are the owner's agent; a guest of a room reaches only its own room."
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
                    "urgent": { "type": "boolean", "default": false, "description": "Steer the receiver's running turn, or start one if it is idle. When false, the message queues for the next turn and never starts or steers one." },
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
        OWNER_VIEW_TOOL | "chariox_owner_view" | "mcp__chariox__chariox_owner_view" => {
            Some(OWNER_VIEW_TOOL)
        }
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
