//! Claude Code `--permission-prompt-tool` contract for headless (`-p`) runs.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PermissionPromptArgs {
    pub tool_name: String,
    #[serde(default)]
    pub input: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
}

pub fn permission_prompt_runtime_tool_spec() -> RuntimeToolSpec {
    RuntimeToolSpec {
        name: PERMISSION_PROMPT_TOOL.to_string(),
        description: "Chariox permission bridge used by Claude Code to ask the user before running a tool. Claude Code calls it itself; do not call it directly.".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "tool_name": { "type": "string" },
                "input": { "type": "object" },
                "tool_use_id": { "type": "string" }
            },
            "required": ["tool_name", "input"],
            "additionalProperties": true
        }),
    }
}
