use super::*;

use crate::transport::runtime_tools::{
    MetaCommandDocsArgs, MetaCommandSearchArgs, MetaSessionOverviewArgs, RuntimeToolResult,
    META_ACK_EVENT_TOOL, META_COMMAND_DOCS_TOOL, META_LIST_COMMANDS_TOOL, META_LIST_EVENTS_TOOL,
    META_LIST_SUBSCRIPTIONS_TOOL, META_READ_EVENT_TOOL, META_RESOLVE_RUNTIME_INTERACTION_TOOL,
    META_RUN_COMMAND_TOOL, META_SEARCH_COMMANDS_TOOL, META_SESSION_OVERVIEW_TOOL,
    META_SUBSCRIBE_EVENTS_TOOL, META_TURN_BLOB_TOOL, META_TURN_OVERVIEW_TOOL,
    META_UNSUBSCRIBE_EVENTS_TOOL,
};

#[derive(Debug, Clone, Copy)]
struct MetaCommandDoc {
    name: &'static str,
    aliases: &'static [&'static str],
    usage: &'static str,
    examples: &'static [&'static str],
    tags: &'static [&'static str],
    scope: &'static str,
    mutates: bool,
    policy: &'static str,
    description: &'static str,
}

const META_COMMANDS: &[MetaCommandDoc] = &[
    MetaCommandDoc {
        name: "session overview",
        aliases: &["context", "agent list", "workflow list"],
        usage: "Use arroba.meta.session_overview for current session state.",
        examples: &["arroba.meta.session_overview({})"],
        tags: &["inspect", "session", "agents", "workflows"],
        scope: "session",
        mutates: false,
        policy: "allow",
        description: "Inspect current session, owned agents, workflow runs, pending interactions, and event counts.",
    },
    MetaCommandDoc {
        name: "prompt",
        aliases: &["prompt <agent-ref> <text>"],
        usage: "prompt [agent-ref] <prompt> [--wait] [--show-reply|--show-summary]",
        examples: &["prompt agent-2 \"Investigate this failure\" --wait"],
        tags: &["prompt", "agent", "orchestration"],
        scope: "session",
        mutates: true,
        policy: "allow",
        description: "Submit a normal Arroba prompt to one of this user's regular agents through the existing prompt path.",
    },
    MetaCommandDoc {
        name: "agent spawn",
        aliases: &["agent spawn"],
        usage: "agent spawn [alias] [model] [--dir <directory>] [--worktree <directory> --branch <branch>] [--machine <machine-ref>|--kernel <kernel-ref>]",
        examples: &["agent spawn reviewer gpt-5.2"],
        tags: &["agent", "spawn", "orchestration"],
        scope: "session",
        mutates: true,
        policy: "allow",
        description: "Create a regular agent owned by the current user. Metaagents cannot spawn another metaagent.",
    },
    MetaCommandDoc {
        name: "workflow",
        aliases: &["workflow new", "workflow run", "workflow cancel", "workflow resume"],
        usage: "workflow <new|list|show|run|runs|cancel|resume> ...",
        examples: &["workflow run qa-flow default \"Run QA\""],
        tags: &["workflow", "orchestration"],
        scope: "session",
        mutates: true,
        policy: "allow",
        description: "Create, edit, run, cancel, resume, and observe workflows from above. Metaagents cannot be workflow nodes.",
    },
    MetaCommandDoc {
        name: "mcp",
        aliases: &["mcp grant", "mcp revoke", "mcp list"],
        usage: "mcp <install|list|show|import|grant|revoke|grants> ...",
        examples: &["mcp grant playwright --agent agent-2"],
        tags: &["extension", "mcp", "capability"],
        scope: "session",
        mutates: true,
        policy: "allow",
        description: "Manage MCP extension grants for this user's agents through existing kernel extension policy.",
    },
    MetaCommandDoc {
        name: "skill",
        aliases: &["skill grant", "skill list", "skills"],
        usage: "skill <install|list|show|import|grant|revoke|grants> ...",
        examples: &["skill grant browser-qa --agent agent-2"],
        tags: &["extension", "skill", "capability"],
        scope: "session",
        mutates: true,
        policy: "allow",
        description: "Manage skill grants for this user's agents through existing kernel extension policy.",
    },
    MetaCommandDoc {
        name: "slice",
        aliases: &["slice save", "slice restart", "slice stop"],
        usage: "slice <list|show|save-state|start|stop|reset-state|backup> ...",
        examples: &["slice save-state dev --restart-agents"],
        tags: &["slice", "environment"],
        scope: "session",
        mutates: true,
        policy: "allow",
        description: "Inspect and manage slices. Metaagents cannot run inside a slice but can manage authorized slices.",
    },
    MetaCommandDoc {
        name: "credential",
        aliases: &["credential set", "credential list"],
        usage: "credential <list|get|upsert|remove|set-secret|delete-secret> ...",
        examples: &["credential list"],
        tags: &["credential", "vault", "sensitive"],
        scope: "global",
        mutates: true,
        policy: "approval",
        description: "Manage credential handles and vault-backed values. Sensitive mutations should require policy approval.",
    },
    MetaCommandDoc {
        name: "session new",
        aliases: &["session create"],
        usage: "session new ...",
        examples: &[],
        tags: &["session", "denied"],
        scope: "global",
        mutates: true,
        policy: "deny",
        description: "Denied for metaagents. Metaagents must operate inside their containing session.",
    },
];

impl KernelRuntimeState {
    pub(crate) fn metaagent_context_for_auth_token(
        &self,
        auth_token: &str,
    ) -> Result<
        (
            crate::provider::RuntimeProviderRun,
            crate::session::RuntimeSession,
            crate::agent::AgentInstance,
        ),
        DaemonError,
    > {
        let provider_runs = self
            .owned
            .provider_store
            .get_runs_by_runtime_mcp_auth_token(auth_token);
        let [provider_run] = provider_runs.as_slice() else {
            return Err(DaemonError::LocalTransport {
                operation: "runtime_tool_meta",
                message: "metaagent runtime tools require one active provider run".to_string(),
            });
        };
        let (session, agent) = self.metaagent_for_provider_run(provider_run)?;
        Ok((provider_run.clone(), session, agent))
    }

    pub(super) fn meta_runtime_tool_specs_enabled_for_auth_token(&self, auth_token: &str) -> bool {
        let provider_runs = self
            .owned
            .provider_store
            .get_runs_by_runtime_mcp_auth_token(auth_token);
        let [provider_run] = provider_runs.as_slice() else {
            return false;
        };
        self.metaagent_for_provider_run(provider_run).is_ok()
    }

    pub(super) async fn dispatch_meta_runtime_tool_call(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let (session, agent) = self.metaagent_for_provider_run(provider_run)?;
        match tool_name {
            META_SESSION_OVERVIEW_TOOL => {
                let args = serde_json::from_value::<MetaSessionOverviewArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                self.meta_session_overview(&session, &agent, args)
            }
            META_SEARCH_COMMANDS_TOOL | META_LIST_COMMANDS_TOOL => {
                let args = serde_json::from_value::<MetaCommandSearchArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "commands": filter_meta_commands(args),
                    }),
                })
            }
            META_COMMAND_DOCS_TOOL => {
                let args = serde_json::from_value::<MetaCommandDocsArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                Ok(meta_command_docs(args.command))
            }
            META_RUN_COMMAND_TOOL
            | META_LIST_EVENTS_TOOL
            | META_READ_EVENT_TOOL
            | META_ACK_EVENT_TOOL
            | META_TURN_OVERVIEW_TOOL
            | META_TURN_BLOB_TOOL
            | META_SUBSCRIBE_EVENTS_TOOL
            | META_UNSUBSCRIBE_EVENTS_TOOL
            | META_LIST_SUBSCRIPTIONS_TOOL
            | META_RESOLVE_RUNTIME_INTERACTION_TOOL => Ok(RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "error": "metaagent tool is registered but not implemented in this slice",
                    "tool": tool_name,
                    "agent_ref": agent.agent_ref(),
                }),
            }),
            _ => Err(DaemonError::LocalTransport {
                operation: "runtime_tool_meta",
                message: format!("unsupported metaagent tool `{tool_name}`"),
            }),
        }
    }

    fn metaagent_for_provider_run(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
    ) -> Result<(crate::session::RuntimeSession, crate::agent::AgentInstance), DaemonError> {
        let Some(agent_id) = provider_run.agent_instance_id() else {
            return Err(DaemonError::LocalTransport {
                operation: "runtime_tool_meta",
                message: "metaagent tools require an agent-bound provider run".to_string(),
            });
        };
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if !agent.is_metaagent() {
            return Err(DaemonError::LocalTransport {
                operation: "runtime_tool_meta",
                message: "metaagent tools are only available to session metaagents".to_string(),
            });
        }
        let session = self
            .owned
            .session_store
            .get_session(provider_run.session_id())?;
        Ok((session, agent))
    }

    fn meta_session_overview(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaSessionOverviewArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let include_workflows = args.include_workflows.unwrap_or(true);
        let include_events = args.include_events.unwrap_or(true);
        let session_agents = self.owned.agent_store.get_session_agents(session.id());
        let owned_agents = session_agents
            .iter()
            .filter(|candidate| candidate.owner_user_id() == agent.owner_user_id())
            .collect::<Vec<_>>();
        let pending_interactions = session
            .active_interactions()
            .iter()
            .filter(|interaction| interaction.agent_id() != agent.id())
            .collect::<Vec<_>>();
        Ok(RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "session": {
                    "id": session.id(),
                    "workspace_id": session.workspace_id(),
                    "worktree_id": session.worktree_id(),
                    "status": session.status(),
                    "owner_user_id": session.owner_user_id(),
                    "focused_agent_id": session.focused_agent_id(),
                },
                "metaagent": {
                    "id": agent.id(),
                    "agent_ref": agent.agent_ref(),
                    "alias": agent.alias(),
                    "provider": agent.provider(),
                    "model": agent.model(),
                    "owner_user_id": agent.owner_user_id(),
                    "status": agent.state(),
                    "is_processing": agent.is_processing(),
                },
                "agents": {
                    "total": session_agents.len(),
                    "owned_total": owned_agents.len(),
                    "owned": owned_agents,
                },
                "workflows": if include_workflows {
                    serde_json::json!({
                        "definitions": session.workflows(),
                        "runs": session.workflow_runs(),
                    })
                } else {
                    serde_json::Value::Null
                },
                "pending_interactions": pending_interactions,
                "events": if include_events {
                    serde_json::json!({
                        "inbox_total": 0,
                        "unacked_total": 0,
                        "note": "metaagent event inbox will be populated by the event injection slice"
                    })
                } else {
                    serde_json::Value::Null
                },
            }),
        })
    }
}

fn invalid_meta_args(error: serde_json::Error) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "runtime_tool_meta",
        message: format!("invalid tool arguments: {error}"),
    }
}

fn filter_meta_commands(args: MetaCommandSearchArgs) -> Vec<serde_json::Value> {
    let query = args.query.map(|value| value.to_lowercase());
    let tag = args.tag.map(|value| value.to_lowercase());
    let scope = args.scope.map(|value| value.to_lowercase());
    let policy = args.policy.map(|value| value.to_lowercase());
    let limit = args.limit.unwrap_or(50).clamp(1, 100);
    META_COMMANDS
        .iter()
        .filter(|command| {
            query.as_ref().is_none_or(|query| {
                command.name.contains(query)
                    || command.usage.to_lowercase().contains(query)
                    || command.description.to_lowercase().contains(query)
                    || command
                        .aliases
                        .iter()
                        .any(|alias| alias.to_lowercase().contains(query))
            })
        })
        .filter(|command| {
            tag.as_ref().is_none_or(|tag| {
                command
                    .tags
                    .iter()
                    .any(|candidate| candidate.to_lowercase() == *tag)
            })
        })
        .filter(|command| scope.as_ref().is_none_or(|scope| command.scope == scope))
        .filter(|command| {
            args.mutates
                .is_none_or(|mutates| command.mutates == mutates)
        })
        .filter(|command| {
            policy
                .as_ref()
                .is_none_or(|policy| command.policy == policy)
        })
        .take(limit)
        .map(meta_command_json)
        .collect()
}

fn meta_command_docs(command: String) -> RuntimeToolResult {
    let normalized = command.to_lowercase();
    let Some(command) = META_COMMANDS.iter().find(|candidate| {
        candidate.name == normalized
            || candidate
                .aliases
                .iter()
                .any(|alias| alias.to_lowercase() == normalized)
    }) else {
        return RuntimeToolResult {
            ok: false,
            payload: serde_json::json!({
                "error": format!("unknown metaagent command `{command}`")
            }),
        };
    };
    RuntimeToolResult {
        ok: true,
        payload: meta_command_json(command),
    }
}

fn meta_command_json(command: &MetaCommandDoc) -> serde_json::Value {
    serde_json::json!({
        "name": command.name,
        "aliases": command.aliases,
        "usage": command.usage,
        "examples": command.examples,
        "tags": command.tags,
        "scope": command.scope,
        "mutates": command.mutates,
        "metaagent_policy": command.policy,
        "description": command.description,
    })
}
