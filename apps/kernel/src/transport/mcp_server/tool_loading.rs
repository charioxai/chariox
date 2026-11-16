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
/// messages), plus what Claude Code needs to run a permission prompt.
const AGENT_UPFRONT_TOOLS: [&str; 9] = [
    "chariox.events.ack",
    "chariox.events.blocked",
    "chariox.events.inbox",
    "chariox.events.process",
    "chariox.events.timer",
    "chariox.events.yield",
    "chariox.send_agent_message",
    "chariox.vault.use_env",
    PERMISSION_PROMPT_TOOL,
];

/// An agent another agent started reads its task, answers and asks for help;
/// waiting, messaging and Vault-filled commands load when it needs them.
const SUB_AGENT_UPFRONT_TOOLS: [&str; 4] = [
    "chariox.events.ack",
    "chariox.events.blocked",
    "chariox.events.inbox",
    PERMISSION_PROMPT_TOOL,
];

/// A workflow node must not have to search for the tools its turn ends with.
const WORKFLOW_TURN_UPFRONT_TOOLS: [&str; 8] = [
    ACK_WORKFLOW_TURN_TOOL,
    VALIDATE_WORKFLOW_HANDOFF_TOOL,
    VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL,
    VALIDATE_AND_SUBMIT_INTERMEDIATE_WORKFLOW_RUN_OUTPUT_TOOL,
    READ_WORKFLOW_TURN_CONTEXT_TOOL,
    WORKFLOW_CONSOLE_READ_TOOL,
    WORKFLOW_CONSOLE_WRITE_TOOL,
    WORKFLOW_CONSOLE_CLEAR_TOOL,
];

/// Shown to Claude Code at session start, before any tool is loaded: it says
/// that more tools exist and how to load them.
const CLAUDE_SERVER_INSTRUCTIONS: &str = "Chariox runtime tools. The tools for your inbox and wakes (chariox.events.*), Vault-filled commands (chariox.vault.use_env) and messages (chariox.send_agent_message) are loaded. Every other Chariox tool is loaded on demand: load it with ToolSearch before use, by purpose or with `select:mcp__chariox__chariox_<name with dots as underscores>`, for example `select:mcp__chariox__chariox_vault_generate`. Tools are grouped by purpose: agents, room commands, guides and history; workflows; secrets, Vault and HTTP with credentials; questions to the person and visuals; terminals; files, jobs and machines; environment and extensions; resources.";

const CLAUDE_SUB_AGENT_SERVER_INSTRUCTIONS: &str = "Chariox runtime tools. Your inbox tools (chariox.events.inbox, chariox.events.ack, chariox.events.blocked) are loaded. Every other Chariox tool is loaded on demand: load it with ToolSearch before use, by purpose or with `select:mcp__chariox__chariox_<name with dots as underscores>`, for example `select:mcp__chariox__chariox_send_agent_message`. Waiting (chariox.events.yield, process, timer), messages to other agents (chariox.send_agent_message) and Vault-filled commands (chariox.vault.use_env) are the ones a worker most often needs.";

/// How one Claude Code run loads the runtime tools.
#[derive(Debug, Clone, Copy)]
pub(super) struct ClaudeLoading {
    sub_agent: bool,
}

impl ClaudeLoading {
    /// `None` for a run that is not Claude Code: it loads its tools its own way.
    pub(super) fn for_run(
        router: &crate::runtime::router::CommandRouter,
        auth_token: &str,
    ) -> Option<Self> {
        let run = router.runtime_mcp_provider_run(auth_token)?;
        (run.adapter_key() == "claude").then(|| Self {
            sub_agent: run.is_sub_agent(),
        })
    }

    pub(super) fn instructions(self) -> &'static str {
        if self.sub_agent {
            CLAUDE_SUB_AGENT_SERVER_INSTRUCTIONS
        } else {
            CLAUDE_SERVER_INSTRUCTIONS
        }
    }

    pub(super) fn loads_upfront(self, tool_name: &str) -> bool {
        let own: &[&str] = if self.sub_agent {
            &SUB_AGENT_UPFRONT_TOOLS
        } else {
            &AGENT_UPFRONT_TOOLS
        };
        own.contains(&tool_name) || WORKFLOW_TURN_UPFRONT_TOOLS.contains(&tool_name)
    }
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
            for sub_agent in [false, true] {
                assert!(
                    ClaudeLoading { sub_agent }.loads_upfront(&spec.name),
                    "{} is deferred",
                    spec.name
                );
            }
        }
    }
}
