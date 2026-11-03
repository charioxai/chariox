//! The `chariox-reach` MCP server: how a kernel owner's Room agent reaches other
//! rooms and kernels. The specs and arguments only; the kernel authorizes each call
//! against the agent's own live run.

use serde::Deserialize;

use super::*;

pub const REACH_KERNELS_TOOL: &str = "kernels";
pub const REACH_REQUEST_TOOL: &str = "request";

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReachRequestArgs {
    /// The kernel to ask, as `kernels` names it; absent means this kernel.
    #[serde(default)]
    pub kernel: Option<String>,
    /// One externally tagged kernel request, for example `{"ListSessions":null}`.
    pub request: Value,
}

pub fn reach_tool_specs() -> Vec<RuntimeToolSpec> {
    vec![
        RuntimeToolSpec {
            name: REACH_KERNELS_TOOL.into(),
            description: "List the kernels of your user that you can reach from this room: name, id, machine and state.".into(),
            input_schema: serde_json::json!({"type": "object", "properties": {}, "additionalProperties": false}),
        },
        RuntimeToolSpec {
            name: REACH_REQUEST_TOOL.into(),
            description: "Send one kernel request for your user to another room or kernel. Without /sudo you may do there what a Room agent without /sudo may do in its own room: list and read sessions, agents and history, attach and message agents, and use machines. What needs /sudo in your own room needs a live /sudo window of your own turn here; owner-only requests are refused. Example: {\"kernel\":\"my-mac\",\"request\":{\"ListSessions\":null}}.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "kernel": {"type": "string", "description": "Kernel name or id from `kernels`; omit for this kernel."},
                    "request": {"type": "object", "description": "One externally tagged kernel request, for example {\"ListSessions\":null}."}
                },
                "required": ["request"],
                "additionalProperties": false
            }),
        },
    ]
}
