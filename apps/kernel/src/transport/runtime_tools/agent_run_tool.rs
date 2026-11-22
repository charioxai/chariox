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
            "Run sub-agents in parallel and return each final answer, status and task_id. Give each worker one independent area; keep tasks you can finish in about 10 calls. After {} seconds a running worker returns a registration_id. Yield on those IDs for child_finished events. For work you know takes longer, use agent spawn and events.yield. Use the origin_prompt_id from this turn's hidden context.",
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
                            "brief": {"type": "string", "description": "One deliverable: the task, the files it owns, constraints, and the answer as paths and facts in at most 30 lines. For a large file on this kernel, have the worker publish it from its TMPDIR with chariox.agents.result and answer with the artifact_id. Fetch it with the same tool."},
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
