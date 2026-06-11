use crate::transport::runtime_tools::{MetaCommandDocsArgs, MetaCommandSearchArgs};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetaCommandPolicy {
    Allow,
    Approval,
    Deny,
}

impl MetaCommandPolicy {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Approval => "approval",
            Self::Deny => "deny",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct MetaCommandDoc {
    pub(crate) name: &'static str,
    pub(crate) aliases: &'static [&'static str],
    pub(crate) usage: &'static str,
    pub(crate) examples: &'static [&'static str],
    pub(crate) tags: &'static [&'static str],
    pub(crate) scope: &'static str,
    pub(crate) mutates: bool,
    pub(crate) policy: MetaCommandPolicy,
    pub(crate) authority: &'static str,
    pub(crate) routed: bool,
    pub(crate) description: &'static str,
}

pub(crate) const META_COMMANDS: &[MetaCommandDoc] = &[
    MetaCommandDoc {
        name: "session overview",
        aliases: &["context", "agent list", "workflow list"],
        usage: "Use arroba.meta.session_overview for current session state.",
        examples: &["arroba.meta.session_overview({})"],
        tags: &["inspect", "session", "agents", "workflows"],
        scope: "session",
        mutates: false,
        policy: MetaCommandPolicy::Allow,
        authority: "current session",
        routed: true,
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
        policy: MetaCommandPolicy::Allow,
        authority: "owned regular agents",
        routed: true,
        description: "Submit a normal Arroba prompt to one of this user's regular agents through the existing prompt path.",
    },
    MetaCommandDoc {
        name: "agent spawn",
        aliases: &[
            "agent spawn",
            "agent focus",
            "agent alias",
            "agent delete",
            "agent destroy",
        ],
        usage: "agent <list|spawn|focus|alias|delete|destroy> ...",
        examples: &[
            "agent spawn reviewer gpt-5.2",
            "agent alias reviewer code-reviewer",
            "agent delete code-reviewer",
        ],
        tags: &["agent", "spawn", "orchestration"],
        scope: "session",
        mutates: true,
        policy: MetaCommandPolicy::Allow,
        authority: "owned agents only",
        routed: true,
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
        policy: MetaCommandPolicy::Allow,
        authority: "session workflow policy",
        routed: true,
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
        policy: MetaCommandPolicy::Allow,
        authority: "owned regular agents",
        routed: true,
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
        policy: MetaCommandPolicy::Allow,
        authority: "owned regular agents",
        routed: true,
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
        policy: MetaCommandPolicy::Allow,
        authority: "authorized slices",
        routed: true,
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
        policy: MetaCommandPolicy::Approval,
        authority: "configured user approval",
        routed: true,
        description: "List and inspect credential handles. Sensitive credential mutations require approval policy and are not routed by default.",
    },
    MetaCommandDoc {
        name: "session new",
        aliases: &["session create", "session attach", "session use", "session delete"],
        usage: "session new|create|attach|use|delete ...",
        examples: &[],
        tags: &["session", "denied"],
        scope: "global",
        mutates: true,
        policy: MetaCommandPolicy::Deny,
        authority: "denied",
        routed: false,
        description: "Denied for metaagents. Metaagents must operate inside their containing session.",
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MetaCommandExecutionPolicy {
    Routed,
    Denied { message: &'static str },
    NotRouted { message: String },
}

pub(crate) fn search_commands(args: MetaCommandSearchArgs) -> Vec<serde_json::Value> {
    let query = args.query.map(|value| value.to_lowercase());
    let tag = args.tag.map(|value| value.to_lowercase());
    let scope = args.scope.map(|value| value.to_lowercase());
    let policy = args.policy.map(|value| value.to_lowercase());
    let limit = args.limit.unwrap_or(50).clamp(1, 100);
    META_COMMANDS
        .iter()
        .filter(|command| {
            query
                .as_ref()
                .is_none_or(|query| command_matches_query(command, query))
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
                .is_none_or(|policy| command.policy.as_str() == policy)
        })
        .take(limit)
        .map(command_json)
        .collect()
}

pub(crate) fn command_docs(args: MetaCommandDocsArgs) -> Option<serde_json::Value> {
    find_command(&args.command).map(command_json)
}

pub(crate) fn execution_policy(tokens: &[String]) -> MetaCommandExecutionPolicy {
    let Some(first) = tokens.first().map(|token| token.to_lowercase()) else {
        return MetaCommandExecutionPolicy::NotRouted {
            message: "empty metaagent command".to_string(),
        };
    };
    let normalized = if first == "session" {
        tokens
            .get(1)
            .map(|subcommand| format!("session {}", subcommand.to_lowercase()))
            .unwrap_or_else(|| "session".to_string())
    } else if matches!(first.as_str(), "agent" | "workflow") {
        tokens
            .get(1)
            .map(|subcommand| format!("{first} {}", subcommand.to_lowercase()))
            .unwrap_or_else(|| first.clone())
    } else {
        first.clone()
    };
    let Some(command) = find_command(&normalized).or_else(|| find_command(tokens[0].as_str()))
    else {
        return MetaCommandExecutionPolicy::NotRouted {
            message: format!(
                "`{}` is not registered for arroba.meta.run_command",
                tokens[0]
            ),
        };
    };
    if let Some(family_policy) = routed_family_policy(&first, tokens) {
        return family_policy;
    }
    match command.policy {
        MetaCommandPolicy::Deny => MetaCommandExecutionPolicy::Denied {
            message: "metaagents cannot create, attach to, switch, or delete sessions",
        },
        MetaCommandPolicy::Approval if !command.routed => MetaCommandExecutionPolicy::NotRouted {
            message: format!(
                "`{}` requires approval policy and is not routed for metaagent command execution yet",
                command.name
            ),
        },
        _ if command.routed => MetaCommandExecutionPolicy::Routed,
        _ => MetaCommandExecutionPolicy::NotRouted {
            message: format!(
                "`{}` is documented in the metaagent command registry but is not routed for execution yet",
                command.name
            ),
        },
    }
}

fn routed_family_policy(first: &str, tokens: &[String]) -> Option<MetaCommandExecutionPolicy> {
    match first {
        "agent" => match tokens.get(1).map(String::as_str) {
            Some(
                "list" | "ls" | "spawn" | "focus" | "alias" | "name" | "delete" | "destroy"
                | "remove",
            ) => Some(MetaCommandExecutionPolicy::Routed),
            _ => Some(MetaCommandExecutionPolicy::NotRouted {
                message: "only `agent list`, `agent spawn`, `agent focus`, `agent alias`, and `agent delete` are routed for metaagent command execution yet".to_string(),
            }),
        },
        "workflow" => match tokens.get(1).map(String::as_str) {
            None
            | Some(
                "list" | "ls" | "new" | "create" | "run" | "start" | "runs" | "cancel"
                | "resume",
            ) => Some(MetaCommandExecutionPolicy::Routed),
            _ => Some(MetaCommandExecutionPolicy::NotRouted {
                message: "only `workflow list`, `workflow new`, `workflow run`, `workflow runs`, `workflow cancel`, and `workflow resume` are routed for metaagent command execution yet".to_string(),
            }),
        },
        "mcp" => match tokens.get(1).map(String::as_str) {
            Some("list" | "ls" | "show" | "get" | "grant" | "revoke") => {
                Some(MetaCommandExecutionPolicy::Routed)
            }
            _ => Some(MetaCommandExecutionPolicy::NotRouted {
                message: "only `mcp list`, `mcp show`, `mcp grant`, and `mcp revoke` are routed for metaagent command execution yet".to_string(),
            }),
        },
        "skill" | "skills" => match tokens.get(1).map(String::as_str) {
            Some("list" | "ls" | "show" | "get" | "grant" | "revoke") => {
                Some(MetaCommandExecutionPolicy::Routed)
            }
            _ => Some(MetaCommandExecutionPolicy::NotRouted {
                message: "only `skill list`, `skill show`, `skill grant`, and `skill revoke` are routed for metaagent command execution yet".to_string(),
            }),
        },
        "slice" => match tokens.get(1).map(String::as_str) {
            Some(
                "list" | "ls" | "show" | "get" | "start" | "stop" | "save" | "save-state"
                | "status" | "state-status" | "backup",
            ) => Some(MetaCommandExecutionPolicy::Routed),
            _ => Some(MetaCommandExecutionPolicy::NotRouted {
                message: "only `slice list`, `slice show`, `slice start`, `slice stop`, `slice save-state`, `slice status`, and `slice backup` are routed for metaagent command execution yet".to_string(),
            }),
        },
        "credential" | "credentials" => match tokens.get(1).map(String::as_str) {
            Some("list" | "ls" | "get" | "show") => Some(MetaCommandExecutionPolicy::Routed),
            _ => Some(MetaCommandExecutionPolicy::NotRouted {
                message: "only `credential list` and `credential get` are routed for metaagent command execution; credential mutations require approval policy".to_string(),
            }),
        },
        _ => None,
    }
}

fn command_matches_query(command: &MetaCommandDoc, query: &str) -> bool {
    command.name.contains(query)
        || command.usage.to_lowercase().contains(query)
        || command.description.to_lowercase().contains(query)
        || command.authority.to_lowercase().contains(query)
        || command.policy.as_str().contains(query)
        || command
            .aliases
            .iter()
            .any(|alias| alias.to_lowercase().contains(query))
        || command
            .examples
            .iter()
            .any(|example| example.to_lowercase().contains(query))
        || command
            .tags
            .iter()
            .any(|tag| tag.to_lowercase().contains(query))
}

fn find_command(command: &str) -> Option<&'static MetaCommandDoc> {
    let normalized = command.to_lowercase();
    META_COMMANDS.iter().find(|candidate| {
        candidate.name == normalized
            || candidate
                .aliases
                .iter()
                .any(|alias| alias.to_lowercase() == normalized)
            || normalized.starts_with(candidate.name)
            || candidate
                .aliases
                .iter()
                .any(|alias| normalized.starts_with(*alias))
    })
}

fn command_json(command: &MetaCommandDoc) -> serde_json::Value {
    serde_json::json!({
        "name": command.name,
        "aliases": command.aliases,
        "usage": command.usage,
        "examples": command.examples,
        "tags": command.tags,
        "scope": command.scope,
        "mutates": command.mutates,
        "metaagent_policy": command.policy.as_str(),
        "authority": command.authority,
        "routed": command.routed,
        "description": command.description,
    })
}
