//! Which runtime tools a Claude Code run has before its first call.
//!
//! Claude Code loads MCP tools on demand through its own `ToolSearch` and
//! lists only their names. The tools below are marked to load upfront with the
//! documented `anthropic/alwaysLoad` tool metadata; every other tool costs a
//! name until the agent asks for it.
use crate::transport::runtime_tools::{
    ACK_WORKFLOW_TURN_TOOL, PERMISSION_PROMPT_TOOL, READ_WORKFLOW_TURN_CONTEXT_TOOL,
    VALIDATE_AND_SUBMIT_INTERMEDIATE_WORKFLOW_RUN_OUTPUT_TOOL,
    VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL, VALIDATE_WORKFLOW_HANDOFF_TOOL,
    WORKFLOW_CONSOLE_CLEAR_TOOL, WORKFLOW_CONSOLE_READ_TOOL, WORKFLOW_CONSOLE_WRITE_TOOL,
};

pub(super) const ALWAYS_LOAD_META_KEY: &str = "anthropic/alwaysLoad";

/// What an agent does on every turn (inbox, wake sources, Vault-filled commands,
/// messages), what Claude Code needs to run a permission prompt, and the tools
/// a workflow node must have without searching.
const UPFRONT_TOOLS: [&str; 17] = [
    "chariox.events.ack",
    "chariox.events.blocked",
    "chariox.events.inbox",
    "chariox.events.process",
    "chariox.events.timer",
    "chariox.events.yield",
    "chariox.send_agent_message",
    "chariox.vault.use_env",
    PERMISSION_PROMPT_TOOL,
    ACK_WORKFLOW_TURN_TOOL,
    VALIDATE_WORKFLOW_HANDOFF_TOOL,
    VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL,
    VALIDATE_AND_SUBMIT_INTERMEDIATE_WORKFLOW_RUN_OUTPUT_TOOL,
    READ_WORKFLOW_TURN_CONTEXT_TOOL,
    WORKFLOW_CONSOLE_READ_TOOL,
    WORKFLOW_CONSOLE_WRITE_TOOL,
    WORKFLOW_CONSOLE_CLEAR_TOOL,
];

pub(super) fn loads_upfront(tool_name: &str) -> bool {
    UPFRONT_TOOLS.contains(&tool_name)
}

/// Shown to Claude Code at session start, before any tool is loaded: it says
/// that more tools exist and how to load them.
pub(super) const CLAUDE_SERVER_INSTRUCTIONS: &str = "Chariox runtime tools. The tools for your inbox and wakes (chariox.events.*), Vault-filled commands (chariox.vault.use_env) and messages (chariox.send_agent_message) are loaded. Every other Chariox tool is loaded on demand: load it with ToolSearch before use, by purpose or with `select:mcp__chariox__chariox_<name with dots as underscores>`, for example `select:mcp__chariox__chariox_vault_generate`. Tools are grouped by purpose: agents, room commands, guides and history; workflows; secrets, Vault and HTTP with credentials; questions to the person and visuals; terminals; files, jobs and machines; environment and extensions; resources.";

pub(super) fn is_claude(router: &crate::runtime::router::CommandRouter, auth_token: &str) -> bool {
    router.runtime_mcp_provider_adapter(auth_token).as_deref() == Some("claude")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A workflow node must not have to search for the tools its turn ends with.
    #[test]
    fn every_workflow_turn_tool_loads_upfront() {
        for spec in crate::transport::runtime_tools::workflow_runtime_tool_specs() {
            if spec.name == crate::transport::runtime_tools::AGENT_APP_ACTION_TOOL_QUALIFIED {
                continue;
            }
            assert!(loads_upfront(&spec.name), "{} is deferred", spec.name);
        }
    }
}
