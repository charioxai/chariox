use serde::{Deserialize, Serialize};

use super::*;

pub const AGENTS_RUN_TOOL: &str = "chariox.agents.run";

/// Most workers one call starts; a larger plan is two calls.
pub const AGENTS_RUN_MAX_WORKERS: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentsRunWorker {
    /// The worker's alias in this room.
    pub name: String,
    /// The worker's whole task: it is the worker's first prompt.
    pub brief: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentsRunArgs {
    pub workers: Vec<AgentsRunWorker>,
    pub origin_prompt_id: String,
}

pub fn agent_run_tool_specs() -> Vec<RuntimeToolSpec> {
    vec![RuntimeToolSpec {
        name: AGENTS_RUN_TOOL.to_string(),
        description: format!(
            "Delegate to sub-agents and get their answers back in this one call: each worker is started as your sub-agent with its brief as its prompt, all run in parallel, and the call returns when they have finished, with each worker's final answer, status and task_id. Give each worker one whole independent area and start no more workers than there are independent areas, never one per file or step. A task you can finish in about 10 tool calls stays with you. No inbox, ack, subscription or yield is needed for a worker answered here. Waits at most {} seconds: a worker still running then is returned as status running with a registration_id; end your turn with chariox.events.yield on those registration_ids and its answer arrives as a child_finished event. For work you already know takes longer, start the worker with `agent spawn` and a message and wait with chariox.events.yield instead. Include the origin_prompt_id supplied in this turn's hidden context.",
            crate::room_terminal::MAX_WAIT.as_secs()
        ),
        input_schema: serde_json::json!({
            "type": "object",
            "required": ["workers", "origin_prompt_id"],
            "properties": {
                "workers": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": AGENTS_RUN_MAX_WORKERS,
                    "items": {
                        "type": "object",
                        "required": ["name", "brief"],
                        "properties": {
                            "name": {"type": "string", "description": "Unique alias for the worker: letters, digits, - and _."},
                            "brief": {"type": "string", "description": "One deliverable: the task, the files it owns, constraints, and the answer as paths and facts in at most 30 lines. For a large file on this kernel, ask the worker to save it in its own TMPDIR, publish with chariox.agents.result and return the artifact_id in its final answer. Pass the artifact ID to the same operation with action fetch."},
                            "model": {"type": "string", "description": "Optional model; defaults to yours."},
                            "effort": {"type": "string", "description": "Optional effort; defaults to yours."}
                        },
                        "additionalProperties": false
                    }
                },
                "origin_prompt_id": {
                    "type": "string",
                    "description": "Exact kernel prompt ID supplied for this turn in hidden context."
                }
            },
            "additionalProperties": false
        }),
    }]
}

pub fn canonical_agent_run_tool_name(tool_name: &str) -> Option<&'static str> {
    let name = tool_name
        .strip_prefix("mcp__chariox__")
        .unwrap_or(tool_name);
    (name == AGENTS_RUN_TOOL || name == AGENTS_RUN_TOOL.replace('.', "_"))
        .then_some(AGENTS_RUN_TOOL)
}
