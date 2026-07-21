use crate::transport::runtime_tools::{
    MetaCommandDocsArgs, MetaCommandListArgs, MetaCommandSearchArgs,
};

mod catalog;
#[cfg(test)]
mod tests;

pub(crate) use catalog::AGENT_SPAWN_USAGE;

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
    pub(crate) intents: &'static [&'static str],
    pub(crate) scope: &'static str,
    pub(crate) mutates: bool,
    pub(crate) policy: MetaCommandPolicy,
    pub(crate) authority: &'static str,
    pub(crate) routed: bool,
    pub(crate) description: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MetaCommandExecutionPolicy {
    Routed,
    Denied { message: &'static str },
    NotRouted { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetaCommandParseError {
    message: &'static str,
}

impl MetaCommandParseError {
    pub(crate) fn message(&self) -> &'static str {
        self.message
    }
}

pub(crate) fn tokenize_command(input: &str) -> Result<Vec<String>, MetaCommandParseError> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaping = false;
    for ch in input.trim().chars() {
        if escaping {
            current.push(ch);
            escaping = false;
            continue;
        }
        if ch == '\\' && quote != Some('\'') {
            escaping = true;
            continue;
        }
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            } else {
                current.push(ch);
            }
            continue;
        }
        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            continue;
        }
        if ch.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(ch);
    }
    if escaping {
        current.push('\\');
    }
    if quote.is_some() {
        return Err(MetaCommandParseError {
            message: "unterminated quote",
        });
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

fn meta_commands() -> impl Iterator<Item = &'static MetaCommandDoc> {
    catalog::commands()
}

pub(crate) fn search_commands(args: MetaCommandSearchArgs) -> Vec<serde_json::Value> {
    let query_terms = args.query.as_deref().map(search_terms);
    let tag = args.tag.map(|value| value.to_lowercase());
    let scope = args.scope.map(|value| value.to_lowercase());
    let policy = args.policy.map(|value| value.to_lowercase());
    let limit = args.limit.unwrap_or(50).clamp(1, 100);

    let mut commands: Vec<(&MetaCommandDoc, Option<CommandSearchMatch>)> = meta_commands()
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
        .filter_map(|command| match query_terms.as_ref() {
            Some(terms) => score_command(command, terms).map(|score| (command, Some(score))),
            None => Some((command, None)),
        })
        .collect();

    if query_terms.is_some() {
        commands.sort_by(|(left, left_match), (right, right_match)| {
            right_match
                .as_ref()
                .map_or(0, |search_match| search_match.score)
                .cmp(
                    &left_match
                        .as_ref()
                        .map_or(0, |search_match| search_match.score),
                )
                .then_with(|| right.routed.cmp(&left.routed))
                .then_with(|| left.policy.as_str().cmp(right.policy.as_str()))
                .then_with(|| left.name.cmp(right.name))
        });
    }

    commands
        .into_iter()
        .take(limit)
        .map(|(command, search_match)| {
            search_match.map_or_else(
                || command_json(command),
                |search_match| command_search_json(command, search_match),
            )
        })
        .collect()
}

pub(crate) fn list_commands(args: MetaCommandListArgs) -> Vec<serde_json::Value> {
    search_commands(MetaCommandSearchArgs {
        query: None,
        tag: args.tag,
        scope: args.scope,
        mutates: args.mutates,
        policy: args.policy,
        limit: args.limit,
    })
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
    if let Some(family_policy) = routed_family_policy(&first, tokens) {
        return family_policy;
    }
    let Some(command) = find_command(&normalized).or_else(|| find_command(tokens[0].as_str()))
    else {
        return MetaCommandExecutionPolicy::NotRouted {
            message: format!(
                "`{}` is not registered for arroba.meta.run_command",
                tokens[0]
            ),
        };
    };
    match command.policy {
        MetaCommandPolicy::Deny => MetaCommandExecutionPolicy::Denied {
            message: "agents in Meta mode cannot create, attach to, switch, or delete sessions",
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
                "list" | "ls" | "new" | "create" | "resolve" | "show" | "get" | "alias"
                | "name" | "run" | "start" | "runs" | "get-run" | "run-status" | "cancel"
                | "resume",
            ) => Some(MetaCommandExecutionPolicy::Routed),
            Some("node")
                if matches!(
                    tokens.get(2).map(String::as_str),
                    Some(
                        "add"
                            | "remove"
                            | "delete"
                            | "instructions"
                            | "instruct"
                            | "can-complete"
                            | "complete"
                            | "intermediate-output"
                            | "intermediate"
                            | "wait-for-all-inputs"
                            | "wait-all"
                            | "join"
                            | "max-turns"
                    )
                ) =>
            {
                Some(MetaCommandExecutionPolicy::Routed)
            }
            Some("endpoint")
                if matches!(
                    tokens.get(2).map(String::as_str),
                    Some("new" | "create" | "alias" | "name")
                ) =>
            {
                Some(MetaCommandExecutionPolicy::Routed)
            }
            Some("edge")
                if matches!(
                    tokens.get(2).map(String::as_str),
                    Some("add" | "remove" | "delete")
                ) =>
            {
                Some(MetaCommandExecutionPolicy::Routed)
            }
            _ => {
                if let Some(documented) = documented_workflow_command(tokens) {
                    Some(policy_for_documented_command(documented))
                } else {
                    Some(MetaCommandExecutionPolicy::NotRouted {
                        message: "routed workflow commands: `workflow list`, `workflow new`, `workflow resolve`, `workflow alias`, `workflow node add/remove/instructions/can-complete/intermediate-output/wait-for-all-inputs/max-turns`, `workflow endpoint new/alias`, `workflow edge add/remove`, `workflow run`, `workflow runs`, `workflow get-run`, `workflow cancel`, and `workflow resume`".to_string(),
                    })
                }
            }
        },
        "mcp" => match tokens.get(1).map(String::as_str) {
            Some(
                "list" | "ls" | "show" | "get" | "install-json" | "update-json"
                | "uninstall" | "remove" | "import" | "grant" | "revoke",
            ) => Some(MetaCommandExecutionPolicy::Routed),
            _ => Some(MetaCommandExecutionPolicy::NotRouted {
                message: "only `mcp list`, `mcp show`, `mcp install-json`, `mcp update-json`, `mcp uninstall`, `mcp import`, `mcp grant`, and `mcp revoke` are routed for metaagent command execution yet".to_string(),
            }),
        },
        "skill" | "skills" => match tokens.get(1).map(String::as_str) {
            Some(
                "list" | "ls" | "show" | "get" | "install" | "update" | "uninstall"
                | "remove" | "import" | "grant" | "revoke",
            ) => Some(MetaCommandExecutionPolicy::Routed),
            _ => Some(MetaCommandExecutionPolicy::NotRouted {
                message: "only `skill list`, `skill show`, `skill install`, `skill update`, `skill uninstall`, `skill import`, `skill grant`, and `skill revoke` are routed for metaagent command execution yet".to_string(),
            }),
        },
        "extension" | "extensions" => match (
            tokens.get(1).map(String::as_str),
            tokens.get(2).map(String::as_str),
        ) {
            (Some("import"), Some("providers")) => Some(MetaCommandExecutionPolicy::Routed),
            _ => Some(MetaCommandExecutionPolicy::NotRouted {
                message: "only `extension import providers` is routed for metaagent extension command execution yet".to_string(),
            }),
        },
        "slice" => match tokens.get(1).map(String::as_str) {
            Some("list" | "ls" | "show" | "get" | "start" | "stop" | "save" | "save-state" | "status" | "backup") => {
                Some(MetaCommandExecutionPolicy::Routed)
            }
            _ => Some(MetaCommandExecutionPolicy::NotRouted {
                message: "routed slice commands: `slice list`, `slice show`, `slice start`, `slice stop`, `slice save-state`, `slice status`, and `slice backup`; create slices with `agent spawn <alias> --slice new`".to_string(),
            }),
        },
        "credential" | "credentials" => match tokens.get(1).map(String::as_str) {
            Some("list" | "ls" | "get" | "show" | "upsert-json" | "remove") => {
                Some(MetaCommandExecutionPolicy::Routed)
            }
            Some("vault") if matches!(tokens.get(2).map(String::as_str), Some("status" | "manage")) => {
                Some(MetaCommandExecutionPolicy::Routed)
            }
            Some("set" | "set-secret" | "delete" | "delete-secret") => {
                Some(MetaCommandExecutionPolicy::Denied {
                    message: "agents in Meta mode cannot pass credential secret values through run_command; use worker credential interactions and resolve them explicitly",
                })
            }
            _ => Some(MetaCommandExecutionPolicy::NotRouted {
                message: "only `credential list`, `credential get`, `credential upsert-json`, `credential remove`, `credential vault status`, and `credential vault manage` are routed for metaagent command execution".to_string(),
            }),
        },
        _ => None,
    }
}

fn documented_workflow_command(tokens: &[String]) -> Option<&'static MetaCommandDoc> {
    let first = tokens.first()?;
    if first != "workflow" {
        return None;
    }
    let candidates = [
        tokens
            .get(1)
            .map(|subcommand| format!("workflow {}", subcommand.to_lowercase())),
        tokens.get(2).map(|nested| {
            format!(
                "workflow {} {}",
                tokens.get(1).map(String::as_str).unwrap_or_default(),
                nested.to_lowercase()
            )
        }),
    ];
    candidates
        .into_iter()
        .flatten()
        .find_map(|candidate| find_exact_command(&candidate))
}

fn policy_for_documented_command(command: &'static MetaCommandDoc) -> MetaCommandExecutionPolicy {
    match command.policy {
        MetaCommandPolicy::Deny => MetaCommandExecutionPolicy::Denied {
            message: command.description,
        },
        _ if command.routed => MetaCommandExecutionPolicy::Routed,
        _ => MetaCommandExecutionPolicy::NotRouted {
            message: format!(
                "`{}` is documented in the metaagent command registry but is not routed for execution yet: {}",
                command.name, command.description
            ),
        },
    }
}

fn find_command(command: &str) -> Option<&'static MetaCommandDoc> {
    let normalized = command.to_lowercase();
    meta_commands()
        .filter_map(|candidate| {
            command_match_len(candidate, &normalized).map(|len| (len, candidate))
        })
        .max_by_key(|(len, _)| *len)
        .map(|(_, command)| command)
}

fn find_exact_command(command: &str) -> Option<&'static MetaCommandDoc> {
    let normalized = command.to_lowercase();
    meta_commands().find(|candidate| {
        candidate.name == normalized || candidate.aliases.iter().any(|alias| *alias == normalized)
    })
}

fn command_match_len(command: &MetaCommandDoc, normalized: &str) -> Option<usize> {
    let mut matches = Vec::new();
    if token_prefix_matches(normalized, command.name) {
        matches.push(command.name.len());
    }
    matches.extend(command.aliases.iter().filter_map(|alias| {
        let alias = alias.to_lowercase();
        token_prefix_matches(normalized, &alias).then_some(alias.len())
    }));
    matches.into_iter().max()
}

fn token_prefix_matches(normalized: &str, candidate: &str) -> bool {
    normalized == candidate
        || normalized
            .strip_prefix(candidate)
            .is_some_and(|rest| rest.starts_with(char::is_whitespace))
}

#[derive(Debug, Clone)]
struct CommandSearchMatch {
    score: i64,
    matched_fields: Vec<&'static str>,
    matched_terms: Vec<String>,
}

#[derive(Debug, Clone)]
struct SearchTerm {
    original: String,
    expanded: Vec<String>,
}

fn search_terms(query: &str) -> Vec<SearchTerm> {
    normalize_tokens(query)
        .into_iter()
        .map(|term| {
            let mut expanded = vec![term.clone()];
            for synonym in synonyms_for(&term) {
                if !expanded.iter().any(|candidate| candidate == synonym) {
                    expanded.push((*synonym).to_string());
                }
            }
            SearchTerm {
                original: term,
                expanded,
            }
        })
        .collect()
}

fn score_command(command: &MetaCommandDoc, terms: &[SearchTerm]) -> Option<CommandSearchMatch> {
    if terms.is_empty() {
        return Some(CommandSearchMatch {
            score: 1,
            matched_fields: Vec::new(),
            matched_terms: Vec::new(),
        });
    }

    let fields: [(&'static str, i64, String); 8] = [
        ("name", 90, command.name.to_string()),
        ("aliases", 75, command.aliases.join(" ")),
        ("intents", 55, command.intents.join(" ")),
        ("description", 35, command.description.to_string()),
        ("usage", 30, command.usage.to_string()),
        ("examples", 20, command.examples.join(" ")),
        ("tags", 20, command.tags.join(" ")),
        ("authority", 10, command.authority.to_string()),
    ];
    let field_tokens: Vec<(&'static str, i64, Vec<String>)> = fields
        .iter()
        .map(|(name, weight, text)| (*name, *weight, normalize_tokens(text)))
        .collect();
    let mut score = 0;
    let mut matched_fields = Vec::new();
    let mut matched_terms = Vec::new();

    for term in terms {
        let mut term_matched = false;
        for (field_name, weight, tokens) in &field_tokens {
            let field_hits = term
                .expanded
                .iter()
                .filter(|expanded| tokens.iter().any(|token| token == *expanded))
                .count();
            if field_hits > 0 {
                score += *weight * i64::try_from(field_hits).unwrap_or(1);
                term_matched = true;
                if !matched_fields.contains(field_name) {
                    matched_fields.push(*field_name);
                }
            }
        }
        if term_matched
            && !matched_terms
                .iter()
                .any(|matched| matched == &term.original)
        {
            matched_terms.push(term.original.clone());
        }
    }

    if matched_terms.len() == terms.len() {
        score += 40;
    }
    if terms.iter().any(|term| {
        term.expanded.iter().any(|expanded| {
            command.name == expanded || command.aliases.contains(&expanded.as_str())
        })
    }) {
        score += 50;
    }
    if command.routed {
        score += 5;
    }

    (score > 0).then_some(CommandSearchMatch {
        score,
        matched_fields,
        matched_terms,
    })
}

fn normalize_tokens(input: &str) -> Vec<String> {
    input
        .to_lowercase()
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter_map(|raw| {
            let token = normalize_token(raw);
            (!token.is_empty()).then_some(token)
        })
        .collect()
}

fn normalize_token(raw: &str) -> String {
    if matches!(
        raw,
        "a" | "an"
            | "and"
            | "as"
            | "do"
            | "for"
            | "how"
            | "i"
            | "in"
            | "it"
            | "me"
            | "of"
            | "on"
            | "please"
            | "the"
            | "to"
            | "with"
    ) {
        return String::new();
    }
    if raw.len() > 4 && raw.ends_with('s') {
        raw.trim_end_matches('s').to_string()
    } else {
        raw.to_string()
    }
}

fn synonyms_for(term: &str) -> &'static [&'static str] {
    match term {
        "abort" => &["cancel", "stop"],
        "add" => &["create", "install", "new"],
        "ask" => &["prompt", "delegate"],
        "capability" => &["mcp", "skill", "tool"],
        "continue" => &["resume"],
        "create" => &["spawn", "new", "add", "make"],
        "credential" => &["secret", "vault"],
        "delegate" => &["prompt", "agent", "worker"],
        "execute" => &["run", "start"],
        "give" => &["grant"],
        "make" => &["create", "spawn", "new", "add"],
        "pause" => &["cancel", "stop"],
        "remove" => &["delete", "uninstall"],
        "run" => &["start", "execute"],
        "secret" => &["credential", "vault"],
        "see" => &["list", "inspect"],
        "start" => &["run", "spawn", "create"],
        "stop" => &["cancel", "abort"],
        "store" => &["credential", "vault", "upsert"],
        "task" => &["prompt", "delegate", "workflow"],
        "tell" => &["prompt", "delegate"],
        "tool" => &["mcp", "skill", "capability"],
        "worker" => &["agent"],
        _ => &[],
    }
}

fn command_json(command: &MetaCommandDoc) -> serde_json::Value {
    serde_json::json!({
        "name": command.name,
        "aliases": command.aliases,
        "usage": command.usage,
        "examples": command.examples,
        "tags": command.tags,
        "intents": command.intents,
        "scope": command.scope,
        "mutates": command.mutates,
        "metaagent_policy": command.policy.as_str(),
        "authority": command.authority,
        "routed": command.routed,
        "description": command.description,
    })
}

fn command_search_json(
    command: &MetaCommandDoc,
    search_match: CommandSearchMatch,
) -> serde_json::Value {
    let mut value = command_json(command);
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "search_score".to_string(),
            serde_json::json!(search_match.score),
        );
        object.insert(
            "matched_fields".to_string(),
            serde_json::json!(search_match.matched_fields),
        );
        object.insert(
            "matched_terms".to_string(),
            serde_json::json!(search_match.matched_terms),
        );
    }
    value
}
