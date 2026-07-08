use super::result::meta_command_error;
use super::*;

pub(super) fn owned_regular_agent_error_message(
    reference: &str,
    owned_agents: &[crate::agent::AgentInstance],
) -> String {
    if owned_agents.is_empty() {
        return format!(
            "agent `{reference}` is not an owned regular agent in this session. No owned regular agents are available; spawn one first with `agent spawn <alias>`."
        );
    }
    let available = owned_agents
        .iter()
        .map(|agent| {
            format!(
                "{} (agent_ref: {}, id: {})",
                meta_agent_prompt_ref(agent),
                agent.agent_ref(),
                agent.id()
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let example_ref = meta_agent_prompt_ref(&owned_agents[0]);
    format!(
        "agent `{reference}` is not an owned regular agent in this session. Available owned agents: {available}. Use a listed alias or agent_ref, for example `prompt {} \"<objective, context, constraints, expected report>\"`.",
        shell_quote_for_meta_command(&example_ref)
    )
}

fn meta_agent_prompt_ref(agent: &crate::agent::AgentInstance) -> String {
    agent
        .alias()
        .filter(|alias| !alias.trim().is_empty())
        .unwrap_or_else(|| agent.agent_ref())
        .to_string()
}

fn shell_quote_for_meta_command(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/' | ':'))
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[derive(Debug, Clone)]
pub(super) struct MetaAgentSpawnArgs {
    pub(super) alias: Option<String>,
    pub(super) provider: Option<String>,
    pub(super) model: Option<String>,
    pub(super) effort: Option<String>,
    pub(super) worktree_id: Option<String>,
    pub(super) kernel_ref: Option<String>,
    pub(super) slice_ref: Option<String>,
    pub(super) worktree_placement: Option<GitWorktreePlacement>,
    pub(super) slice_create: Option<MetaAgentSliceCreate>,
}

#[derive(Debug, Clone)]
pub(super) struct MetaAgentSliceCreate {
    pub(super) display_mode: crate::slice::SliceDisplayMode,
}

pub(super) fn parse_meta_agent_spawn_args(
    args: &[String],
    session: &crate::session::RuntimeSession,
) -> Result<MetaAgentSpawnArgs, DaemonError> {
    let mut positional = Vec::new();
    let mut provider = None;
    let mut explicit_model = None;
    let mut effort = None;
    let mut directory = None;
    let mut git_worktree = None;
    let mut branch = None;
    let mut from_ref = None;
    let mut kernel_ref = None;
    let mut slice_ref = None;
    let mut slice_create = None;
    let mut slice_display_mode = None;

    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        match arg.as_str() {
            "--meta" | "--metaagent" | "--as-metaagent" => {
                return Err(meta_command_error(
                    "agents in Meta mode cannot spawn another Meta-mode controller through run_command",
                ));
            }
            "--provider" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(meta_command_error(
                        "usage: agent spawn --provider <provider>",
                    ));
                };
                if value.starts_with("--") {
                    return Err(meta_command_error(
                        "usage: agent spawn --provider <provider>",
                    ));
                }
                provider = Some(value.clone());
                index += 2;
            }
            "--model" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(meta_command_error("usage: agent spawn --model <model>"));
                };
                if value.starts_with("--") {
                    return Err(meta_command_error("usage: agent spawn --model <model>"));
                }
                explicit_model = Some(value.clone());
                index += 2;
            }
            "--effort" | "--variant" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(meta_command_error(
                        "usage: agent spawn --effort <effort>|--variant <variant>",
                    ));
                };
                if value.starts_with("--") {
                    return Err(meta_command_error(
                        "usage: agent spawn --effort <effort>|--variant <variant>",
                    ));
                }
                effort = Some(value.clone());
                index += 2;
            }
            "--dir" | "--directory" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(meta_command_error(format!(
                        "usage: {}",
                        crate::runtime::metaagent_command_registry::AGENT_SPAWN_USAGE
                    )));
                };
                if value.starts_with("--") {
                    return Err(meta_command_error("usage: agent spawn --dir <directory>"));
                }
                directory = Some(value.clone());
                index += 2;
            }
            "--worktree" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(meta_command_error(
                        "usage: agent spawn --worktree <directory> [--branch <branch>] [--from <ref>]",
                    ));
                };
                if value.starts_with("--") {
                    return Err(meta_command_error(
                        "usage: agent spawn --worktree <directory> [--branch <branch>] [--from <ref>]",
                    ));
                }
                git_worktree = Some(value.clone());
                index += 2;
            }
            "--branch" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(meta_command_error("usage: agent spawn --branch <branch>"));
                };
                if value.starts_with("--") {
                    return Err(meta_command_error("usage: agent spawn --branch <branch>"));
                }
                branch = Some(value.clone());
                index += 2;
            }
            "--from" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(meta_command_error("usage: agent spawn --from <ref>"));
                };
                if value.starts_with("--") {
                    return Err(meta_command_error("usage: agent spawn --from <ref>"));
                }
                from_ref = Some(value.clone());
                index += 2;
            }
            "--machine" | "--kernel" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(meta_command_error(
                        "usage: agent spawn --machine <machine-ref>|--kernel <kernel-ref>",
                    ));
                };
                if value.starts_with("--") {
                    return Err(meta_command_error(
                        "usage: agent spawn --machine <machine-ref>|--kernel <kernel-ref>",
                    ));
                }
                if kernel_ref.is_some() {
                    return Err(meta_command_error(
                        "usage: agent spawn uses either --machine or --kernel, not both",
                    ));
                }
                kernel_ref = Some(value.clone());
                index += 2;
            }
            "--slice" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(meta_command_error(
                        "usage: agent spawn --slice off|new|new:headless|new:headed|<slice-ref>",
                    ));
                };
                if value.starts_with("--") {
                    return Err(meta_command_error(
                        "usage: agent spawn --slice off|new|new:headless|new:headed|<slice-ref>",
                    ));
                }
                match value.as_str() {
                    "off" => {
                        slice_ref = None;
                        slice_create = None;
                    }
                    "new" | "new:headless" => {
                        slice_ref = None;
                        slice_create = Some(MetaAgentSliceCreate {
                            display_mode: crate::slice::SliceDisplayMode::Headless,
                        });
                    }
                    "new:headed" => {
                        slice_ref = None;
                        slice_create = Some(MetaAgentSliceCreate {
                            display_mode: crate::slice::SliceDisplayMode::Headed,
                        });
                    }
                    _ => {
                        slice_ref = Some(value.clone());
                        slice_create = None;
                    }
                }
                index += 2;
            }
            "--slice-display" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(meta_command_error(
                        "usage: agent spawn --slice-display headless|headed",
                    ));
                };
                let mode = match value.as_str() {
                    "headless" => crate::slice::SliceDisplayMode::Headless,
                    "headed" => crate::slice::SliceDisplayMode::Headed,
                    _ => {
                        return Err(meta_command_error(
                            "usage: agent spawn --slice-display headless|headed",
                        ));
                    }
                };
                slice_display_mode = Some(mode);
                index += 2;
            }
            value if value.starts_with("--") => {
                return Err(meta_command_error(format!(
                    "unknown agent spawn option `{value}`; usage: {}",
                    crate::runtime::metaagent_command_registry::AGENT_SPAWN_USAGE
                )));
            }
            _ => {
                positional.push(arg.clone());
                index += 1;
            }
        }
    }

    if positional.len() > 2 {
        return Err(meta_command_error(format!(
            "usage: {}",
            crate::runtime::metaagent_command_registry::AGENT_SPAWN_USAGE
        )));
    }
    if positional.get(1).is_some() && explicit_model.is_some() {
        return Err(meta_command_error(
            "usage: agent spawn accepts either positional [model] or --model <model>, not both",
        ));
    }
    if directory.is_some() && git_worktree.is_some() {
        return Err(meta_command_error(
            "usage: agent spawn uses either --dir or --worktree/--branch, not both",
        ));
    }
    if (branch.is_some() || from_ref.is_some()) && git_worktree.is_none() {
        return Err(meta_command_error(
            "usage: agent spawn --branch/--from require --worktree",
        ));
    }
    if let (Some(slice), None) = (slice_ref.as_deref(), slice_create.as_ref()) {
        if kernel_ref.is_some() {
            return Err(meta_command_error(
                "usage: agent spawn uses either --kernel/--machine or a reusable --slice, not both",
            ));
        }
        if directory.is_some() || git_worktree.is_some() {
            return Err(meta_command_error(
                "usage: agent spawn --slice <slice-ref> does not accept --dir or --worktree",
            ));
        }
        if slice.is_empty() {
            return Err(meta_command_error(
                "usage: agent spawn --slice off|new|new:headless|new:headed|<slice-ref>",
            ));
        }
    }
    if let Some(mode) = slice_display_mode {
        let Some(create) = slice_create.as_mut() else {
            return Err(meta_command_error(
                "usage: agent spawn --slice-display requires --slice new",
            ));
        };
        create.display_mode = mode;
    }
    let worktree_id = directory.map(|directory| resolve_metaagent_directory(session, &directory));
    let worktree_placement = if let Some(target_directory) = git_worktree {
        Some(GitWorktreePlacement {
            target_directory: Some(target_directory),
            branch,
            from_ref,
        })
    } else {
        None
    };

    Ok(MetaAgentSpawnArgs {
        alias: positional.first().cloned(),
        provider,
        model: explicit_model.or_else(|| positional.get(1).cloned()),
        effort,
        worktree_id,
        kernel_ref,
        slice_ref,
        worktree_placement,
        slice_create,
    })
}

fn resolve_metaagent_directory(
    session: &crate::session::RuntimeSession,
    directory: &str,
) -> String {
    let path = std::path::Path::new(directory);
    if path.is_absolute() {
        directory.to_string()
    } else {
        std::path::Path::new(session.worktree_id())
            .join(path)
            .to_string_lossy()
            .to_string()
    }
}

pub(super) fn metaagent_spawn_slice_name(alias: Option<&str>) -> String {
    let base = alias.unwrap_or("metaagent-worker");
    let sanitized = base
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    let base = if sanitized.is_empty() {
        "metaagent-worker".to_string()
    } else {
        sanitized
    };
    let suffix = crate::session::unix_epoch_ms().to_string();
    format!("{base}-slice-{}", &suffix[suffix.len().saturating_sub(5)..])
}
