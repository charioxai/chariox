//! Which runtime tools a Claude Code run has before its first call.
//!
//! Claude Code loads MCP tools on demand through its own `ToolSearch` and
//! lists only their names. The tools below are marked to load upfront with the
//! documented `anthropic/alwaysLoad` tool metadata; every other tool costs a
//! name until the agent asks for it.
use std::sync::OnceLock;

use crate::transport::runtime_tools::RuntimeToolSpec;

pub(super) const ALWAYS_LOAD_META_KEY: &str = "anthropic/alwaysLoad";

/// The one list of tools a provider's agent holds before its first call
/// (`scripts/checks/agent-tools-on-demand.mjs` holds it to that list: a tool added
/// to it needs a reviewed `budget raised:` line). `agent` is what an agent needs when a wake
/// reaches it plus delegation and the permission prompt; a `sub_agent` reads its task, answers
/// and asks for help; a workflow node must not have to search for the tools its turn ends with.
const UPFRONT_TOOLS: &str = include_str!("../../../agent_upfront_tools.json");

/// Names `provider` holds upfront in `role`; empty for a provider or role the list does not name
/// as a list of names (`"*"` means the whole catalog).
pub(super) fn upfront_names(provider: &str, role: &str) -> &'static [String] {
    type Lists =
        std::collections::BTreeMap<String, std::collections::BTreeMap<String, serde_json::Value>>;
    static LISTS: OnceLock<std::collections::BTreeMap<(String, String), Vec<String>>> =
        OnceLock::new();
    LISTS
        .get_or_init(|| {
            let lists: Lists =
                serde_json::from_str(UPFRONT_TOOLS).expect("agent_upfront_tools.json is JSON");
            lists
                .into_iter()
                .flat_map(|(provider, roles)| {
                    roles.into_iter().filter_map(move |(role, names)| {
                        let names: Vec<String> = serde_json::from_value(names).ok()?;
                        Some(((provider.clone(), role), names))
                    })
                })
                .collect()
        })
        .get(&(provider.to_owned(), role.to_owned()))
        .map_or(&[], Vec::as_slice)
}

/// Shown to Claude Code at session start, before any tool is loaded: it says
/// that more tools exist and how to load them.
const CLAUDE_SERVER_INSTRUCTIONS: &str = "Chariox runtime tools: chariox.agents.run (delegate to sub-agents and get their answers in one call), chariox.events.inbox and .ack are loaded. Load any other (for example chariox.events.yield, chariox.vault.use_env, chariox.send_agent_message) with ToolSearch before use, by purpose or `select:mcp__chariox__chariox_<name with dots as underscores>`, several per call.";

/// A tool listed under both `chariox.<name>` and a bare `<name>` is one tool
/// with two names. Every provider is told only the canonical one: an extra name
/// costs a deferred tool list about 16 tokens and an eager one its whole schema.
pub(super) fn without_alias_twins(tools: Vec<RuntimeToolSpec>) -> Vec<RuntimeToolSpec> {
    let names: std::collections::HashSet<String> =
        tools.iter().map(|tool| tool.name.clone()).collect();
    tools
        .into_iter()
        .filter(|tool| {
            tool.name.starts_with("chariox.") || !names.contains(&format!("chariox.{}", tool.name))
        })
        .collect()
}

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
        let role = if self.sub_agent { "sub_agent" } else { "agent" };
        [
            upfront_names("claude", role),
            upfront_names("claude", "workflow_turn"),
        ]
        .iter()
        .any(|names| names.iter().any(|name| name == tool_name))
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

    fn spec(name: &str) -> RuntimeToolSpec {
        RuntimeToolSpec {
            name: name.to_string(),
            description: String::new(),
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    #[test]
    fn a_bare_name_is_dropped_only_when_its_canonical_twin_is_listed() {
        let listed = without_alias_twins(vec![
            spec("chariox.request_popup"),
            spec("request_popup"),
            spec("patch_artifact"),
            spec("chariox.events.inbox"),
        ]);
        let names: Vec<&str> = listed.iter().map(|tool| tool.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "chariox.request_popup",
                "patch_artifact",
                "chariox.events.inbox"
            ]
        );
    }
}
