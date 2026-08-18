use super::result::meta_command_error;
use super::spawn_args::parse_meta_agent_spawn_args;
use super::*;
use crate::local::{
    CreateSliceBackupRequest, ListSlicesRequest, SliceStateSaveMode, SliceStateSaveRequest,
    SliceStateSaveScope, SliceStateStatusRequest,
};

pub(super) fn meta_agent_request(
    session: &crate::session::RuntimeSession,
    metaagent: &crate::agent::AgentInstance,
    args: &[String],
    agents: &[crate::agent::AgentInstance],
) -> Result<LocalDaemonRequest, DaemonError> {
    match args.first().map(String::as_str) {
        Some("list" | "ls") => Ok(LocalDaemonRequest::ListAgents(ListAgentsRequest {
            session_id: session.id().to_string(),
        })),
        Some("spawn") => {
            let spawn = parse_meta_agent_spawn_args(&args[1..], session)?;
            if spawn.slice_create.is_some() {
                return Err(meta_command_error(
                    "agent spawn --slice new requires composed Meta-mode spawn dispatch",
                ));
            }
            Ok(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                account_profile: None,
                session_id: session.id().to_string(),
                alias: spawn.alias,
                provider: spawn
                    .provider
                    .or_else(|| Some(metaagent.provider().to_string())),
                model: spawn
                    .model
                    .or_else(|| metaagent.model().map(str::to_string)),
                effort: spawn
                    .effort
                    .or_else(|| metaagent.effort().map(str::to_string)),
                execution_mode: metaagent.execution_mode_override(),
                permission_level: metaagent.permission_level_override(),
                worktree_id: spawn
                    .worktree_id
                    .or_else(|| metaagent.worktree_id().map(str::to_string)),
                kernel_ref: spawn.kernel_ref,
                slice_ref: spawn.slice_ref,
                worktree_placement: spawn.worktree_placement,
                metaagent: false,
            }))
        }
        Some("focus") => {
            let Some(reference) = args.get(1) else {
                return Err(meta_command_error("usage: agent focus <owned-agent-ref>"));
            };
            if args.len() > 2 {
                return Err(meta_command_error("usage: agent focus <owned-agent-ref>"));
            }
            let agent = meta_owned_regular_agent_from_session(agents, metaagent, reference)?;
            Ok(LocalDaemonRequest::FocusAgent(FocusAgentRequest {
                session_id: session.id().to_string(),
                agent_id: agent.id().to_string(),
            }))
        }
        Some("alias" | "name") => {
            if args.len() < 3 {
                return Err(meta_command_error(
                    "usage: agent alias <owned-agent-ref> <alias|clear>",
                ));
            }
            let agent = meta_owned_regular_agent_from_session(agents, metaagent, &args[1])?;
            let alias = args[2..].join(" ");
            let alias = if matches!(alias.as_str(), "clear" | "none" | "-") {
                String::new()
            } else {
                alias
            };
            Ok(LocalDaemonRequest::AliasAgent(AliasAgentRequest {
                session_id: session.id().to_string(),
                agent_id: agent.id().to_string(),
                alias,
            }))
        }
        Some("delete" | "destroy" | "remove") => {
            let Some(reference) = args.get(1) else {
                return Err(meta_command_error("usage: agent delete <owned-agent-ref>"));
            };
            if args.len() > 2 {
                return Err(meta_command_error("usage: agent delete <owned-agent-ref>"));
            }
            let agent = meta_owned_regular_agent_from_session(agents, metaagent, reference)?;
            Ok(LocalDaemonRequest::DestroyAgent(DestroyAgentRequest {
                session_id: session.id().to_string(),
                agent_id: agent.id().to_string(),
            }))
        }
        _ => Err(meta_command_error(
            "usage: agent <list|spawn|focus|alias|delete|destroy> ...",
        )),
    }
}

pub(super) fn meta_slice_request(args: &[String]) -> Result<LocalDaemonRequest, DaemonError> {
    match args.first().map(String::as_str) {
        Some("list" | "ls") => {
            if args.len() != 1 {
                return Err(meta_command_error("usage: slice list"));
            }
            Ok(LocalDaemonRequest::ListSlices(ListSlicesRequest))
        }
        Some("show" | "get") => {
            let Some(slice_ref) = args.get(1) else {
                return Err(meta_command_error("usage: slice show <slice-ref>"));
            };
            if args.len() != 2 {
                return Err(meta_command_error("usage: slice show <slice-ref>"));
            }
            Ok(LocalDaemonRequest::GetSlice(SliceRefRequest {
                slice_ref: slice_ref.clone(),
            }))
        }
        Some("start") => meta_slice_ref_request(args, "start", LocalDaemonRequest::StartSlice),
        Some("stop") => meta_slice_ref_request(args, "stop", LocalDaemonRequest::StopSlice),
        Some("status") => {
            let Some(slice_ref) = args.get(1) else {
                return Err(meta_command_error("usage: slice status <slice-ref>"));
            };
            if args.len() != 2 {
                return Err(meta_command_error("usage: slice status <slice-ref>"));
            }
            Ok(LocalDaemonRequest::GetSliceStateStatus(
                SliceStateStatusRequest {
                    slice_ref: slice_ref.clone(),
                },
            ))
        }
        Some("save" | "save-state") => meta_slice_save_request(args),
        Some("backup") => {
            let Some(slice_ref) = args.get(1) else {
                return Err(meta_command_error("usage: slice backup <slice-ref> [name]"));
            };
            if args.len() > 3 {
                return Err(meta_command_error("usage: slice backup <slice-ref> [name]"));
            }
            Ok(LocalDaemonRequest::CreateSliceBackup(
                CreateSliceBackupRequest {
                    slice_ref: slice_ref.clone(),
                    name: args.get(2).cloned(),
                },
            ))
        }
        _ => Err(meta_command_error(
            "usage: slice <list|show|start|stop|save-state|status|backup> ...",
        )),
    }
}

fn meta_slice_ref_request(
    args: &[String],
    command: &str,
    request: impl FnOnce(SliceRefRequest) -> LocalDaemonRequest,
) -> Result<LocalDaemonRequest, DaemonError> {
    let Some(slice_ref) = args.get(1) else {
        return Err(meta_command_error(format!(
            "usage: slice {command} <slice-ref>"
        )));
    };
    if args.len() != 2 {
        return Err(meta_command_error(format!(
            "usage: slice {command} <slice-ref>"
        )));
    }
    Ok(request(SliceRefRequest {
        slice_ref: slice_ref.clone(),
    }))
}

fn meta_slice_save_request(args: &[String]) -> Result<LocalDaemonRequest, DaemonError> {
    let Some(slice_ref) = args.get(1) else {
        return Err(meta_command_error(
            "usage: slice save-state <slice-ref> [--restart-agents|--shutdown] [--this-slice|--future-slices]",
        ));
    };
    let mut mode = None;
    let mut scope = None;
    for flag in &args[2..] {
        match flag.as_str() {
            "--restart-agents" => mode = Some(SliceStateSaveMode::RestartAgents),
            "--shutdown" => mode = Some(SliceStateSaveMode::Shutdown),
            "--this-slice" => scope = Some(SliceStateSaveScope::ThisSlice),
            "--future-slices" => scope = Some(SliceStateSaveScope::FutureSlices),
            _ => {
                return Err(meta_command_error(format!(
                    "unknown slice save-state option `{flag}`"
                )));
            }
        }
    }
    Ok(LocalDaemonRequest::SaveSliceState(SliceStateSaveRequest {
        slice_ref: slice_ref.clone(),
        mode,
        scope,
    }))
}

fn meta_owned_regular_agent_from_session(
    agents: &[crate::agent::AgentInstance],
    metaagent: &crate::agent::AgentInstance,
    reference: &str,
) -> Result<crate::agent::AgentInstance, DaemonError> {
    agents
        .iter()
        .find(|agent| {
            !agent.is_metaagent()
                && agent.controlled_by_metaagent_id() == Some(metaagent.id())
                && (agent.id() == reference
                    || agent.agent_ref() == reference
                    || agent.alias() == Some(reference))
        })
        .cloned()
        .ok_or_else(|| {
            meta_command_error(format!(
                "agent `{reference}` is not an owned regular agent in this session"
            ))
        })
}

pub(super) fn meta_workflow_request(
    session: &crate::session::RuntimeSession,
    metaagent: &crate::agent::AgentInstance,
    args: &[String],
    agents: &[crate::agent::AgentInstance],
) -> Result<LocalDaemonRequest, DaemonError> {
    match args.first().map(String::as_str) {
        Some("list" | "ls") | None => Ok(LocalDaemonRequest::ListWorkflows(ListWorkflowsRequest {
            session_id: session.id().to_string(),
        })),
        Some("new" | "create") => {
            if args.len() > 2 {
                return Err(meta_command_error("usage: workflow new [alias]"));
            }
            if args.get(1).is_some_and(|arg| arg.starts_with('-')) {
                return Err(meta_command_error("usage: workflow new [alias]"));
            }
            Ok(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
                session_id: session.id().to_string(),
                alias: args.get(1).cloned(),
            }))
        }
        Some("resolve" | "show" | "get") => {
            let Some(workflow_ref) = args.get(1) else {
                return Err(meta_command_error("usage: workflow resolve <workflow-ref>"));
            };
            if args.len() > 2 {
                return Err(meta_command_error("usage: workflow resolve <workflow-ref>"));
            }
            Ok(LocalDaemonRequest::ResolveWorkflow(
                ResolveWorkflowRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow_ref.clone(),
                },
            ))
        }
        Some("alias" | "name") => {
            if args.len() < 3 {
                return Err(meta_command_error(
                    "usage: workflow alias <workflow-ref> <alias>",
                ));
            }
            Ok(LocalDaemonRequest::AliasWorkflow(AliasWorkflowRequest {
                session_id: session.id().to_string(),
                workflow_ref: args[1].clone(),
                alias: args[2..].join(" "),
                expected_workflow_revision: None,
            }))
        }
        Some("node") => match args.get(1).map(String::as_str) {
            Some("add") => {
                if args.len() != 4 {
                    return Err(meta_command_error(
                        "usage: workflow node add <workflow-ref> <owned-agent-ref>",
                    ));
                }
                let agent = meta_owned_regular_agent_from_session(agents, metaagent, &args[3])?;
                Ok(LocalDaemonRequest::AddWorkflowNode(
                    AddWorkflowNodeRequest {
                        session_id: session.id().to_string(),
                        workflow_ref: args[2].clone(),
                        agent_id: agent.id().to_string(),
                        expected_workflow_revision: None,
                    },
                ))
            }
            Some("remove" | "delete") => {
                if args.len() != 4 {
                    return Err(meta_command_error(
                        "usage: workflow node remove <workflow-ref> <node-id>",
                    ));
                }
                Ok(LocalDaemonRequest::RemoveWorkflowNode(
                    RemoveWorkflowNodeRequest {
                        session_id: session.id().to_string(),
                        workflow_ref: args[2].clone(),
                        node_id: args[3].clone(),
                        expected_workflow_revision: None,
                    },
                ))
            }
            Some("instructions" | "instruct") => {
                if args.len() < 4 {
                    return Err(meta_command_error(
                        "usage: workflow node instructions <workflow-ref> <node-id> [instructions]",
                    ));
                }
                let instructions = (!args[4..].is_empty())
                    .then(|| args[4..].join(" "))
                    .filter(|value| !matches!(value.as_str(), "clear" | "none" | "-"));
                Ok(LocalDaemonRequest::UpdateWorkflowNodeInstructions(
                    UpdateWorkflowNodeInstructionsRequest {
                        session_id: session.id().to_string(),
                        workflow_ref: args[2].clone(),
                        node_id: args[3].clone(),
                        instructions,
                        expected_workflow_revision: None,
                    },
                ))
            }
            Some("can-complete" | "complete") => {
                if args.len() != 5 {
                    return Err(meta_command_error(
                        "usage: workflow node can-complete <workflow-ref> <node-id> <true|false>",
                    ));
                }
                Ok(LocalDaemonRequest::SetWorkflowNodeCanCompleteRun(
                    SetWorkflowNodeCanCompleteRunRequest {
                        session_id: session.id().to_string(),
                        workflow_ref: args[2].clone(),
                        node_id: args[3].clone(),
                        can_complete_workflow_run: parse_meta_bool(&args[4])?,
                        expected_workflow_revision: None,
                    },
                ))
            }
            Some("intermediate-output" | "intermediate") => {
                if args.len() != 5 {
                    return Err(meta_command_error(
                        "usage: workflow node intermediate-output <workflow-ref> <node-id> <true|false>",
                    ));
                }
                Ok(
                    LocalDaemonRequest::SetWorkflowNodeCanEmitIntermediateOutput(
                        SetWorkflowNodeCanEmitIntermediateOutputRequest {
                            session_id: session.id().to_string(),
                            workflow_ref: args[2].clone(),
                            node_id: args[3].clone(),
                            can_emit_intermediate_workflow_run_output: parse_meta_bool(&args[4])?,
                            expected_workflow_revision: None,
                        },
                    ),
                )
            }
            Some("wait-for-all-inputs" | "wait-all" | "join") => {
                if args.len() != 5 {
                    return Err(meta_command_error(
                        "usage: workflow node wait-for-all-inputs <workflow-ref> <node-id> <true|false>",
                    ));
                }
                Ok(LocalDaemonRequest::SetWorkflowNodeWaitForAllInputs(
                    SetWorkflowNodeWaitForAllInputsRequest {
                        session_id: session.id().to_string(),
                        workflow_ref: args[2].clone(),
                        node_id: args[3].clone(),
                        wait_for_all_inputs: parse_meta_bool(&args[4])?,
                        expected_workflow_revision: None,
                    },
                ))
            }
            Some("max-turns") => {
                if args.len() != 5 {
                    return Err(meta_command_error(
                        "usage: workflow node max-turns <workflow-ref> <node-id> <number|none>",
                    ));
                }
                let max_turns = match args[4].as_str() {
                    "none" | "clear" | "-" => None,
                    value => Some(value.parse::<u32>().map_err(|error| {
                        meta_command_error(format!("invalid max turns `{value}`: {error}"))
                    })?),
                };
                Ok(LocalDaemonRequest::SetWorkflowNodeMaxTurns(
                    SetWorkflowNodeMaxTurnsRequest {
                        session_id: session.id().to_string(),
                        workflow_ref: args[2].clone(),
                        node_id: args[3].clone(),
                        max_turns,
                        expected_workflow_revision: None,
                    },
                ))
            }
            _ => Err(meta_command_error(
                "usage: workflow node <add|remove|instructions|can-complete|intermediate-output|wait-for-all-inputs|max-turns> ...",
            )),
        },
        Some("endpoint") => match args.get(1).map(String::as_str) {
            Some("new" | "create") => {
                if args.len() < 4 || args.len() > 5 {
                    return Err(meta_command_error(
                        "usage: workflow endpoint new <workflow-ref> <entry-node-id> [alias]",
                    ));
                }
                Ok(LocalDaemonRequest::CreateWorkflowEndpoint(
                    CreateWorkflowEndpointRequest {
                        session_id: session.id().to_string(),
                        workflow_ref: args[2].clone(),
                        entry_node_id: args[3].clone(),
                        alias: args.get(4).cloned(),
                        expected_workflow_revision: None,
                    },
                ))
            }
            Some("alias" | "name") => {
                if args.len() < 5 {
                    return Err(meta_command_error(
                        "usage: workflow endpoint alias <workflow-ref> <endpoint-ref> <alias>",
                    ));
                }
                Ok(LocalDaemonRequest::AliasWorkflowEndpoint(
                    AliasWorkflowEndpointRequest {
                        session_id: session.id().to_string(),
                        workflow_ref: args[2].clone(),
                        endpoint_ref: args[3].clone(),
                        alias: args[4..].join(" "),
                        expected_workflow_revision: None,
                    },
                ))
            }
            _ => Err(meta_command_error(
                "usage: workflow endpoint <new|alias> ...",
            )),
        },
        Some("edge") => match args.get(1).map(String::as_str) {
            Some("add") => {
                if args.len() != 5 {
                    return Err(meta_command_error(
                        "usage: workflow edge add <workflow-ref> <from-node-id> <to-node-id>",
                    ));
                }
                Ok(LocalDaemonRequest::AddWorkflowEdge(
                    AddWorkflowEdgeRequest {
                        session_id: session.id().to_string(),
                        workflow_ref: args[2].clone(),
                        from_node_id: args[3].clone(),
                        to_node_id: args[4].clone(),
                        handoff_schema_ref: None,
                        validation_policy: None,
                        source_side: None,
                        target_side: None,
                        expected_workflow_revision: None,
                    },
                ))
            }
            Some("remove" | "delete") => {
                if args.len() != 4 {
                    return Err(meta_command_error(
                        "usage: workflow edge remove <workflow-ref> <edge-id>",
                    ));
                }
                Ok(LocalDaemonRequest::RemoveWorkflowEdge(
                    RemoveWorkflowEdgeRequest {
                        session_id: session.id().to_string(),
                        workflow_ref: args[2].clone(),
                        edge_id: args[3].clone(),
                        expected_workflow_revision: None,
                    },
                ))
            }
            _ => Err(meta_command_error("usage: workflow edge <add|remove> ...")),
        },
        Some("run") if args.get(1).map(String::as_str) == Some("get") => {
            let Some(workflow_run_ref) = args.get(2) else {
                return Err(meta_command_error("usage: workflow run get <run-ref>"));
            };
            if args.len() > 3 {
                return Err(meta_command_error("usage: workflow run get <run-ref>"));
            }
            Ok(LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: workflow_run_ref.clone(),
            }))
        }
        Some("run" | "start") => {
            if args.len() < 3 {
                return Err(meta_command_error(
                    "usage: workflow run <workflow-ref> <endpoint-ref> [prompt]",
                ));
            }
            Ok(LocalDaemonRequest::InvokeWorkflowEndpoint(
                InvokeWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: args[1].clone(),
                    endpoint_ref: args[2].clone(),
                    queue_ref: None,
                    prompt: (!args[3..].is_empty()).then(|| args[3..].join(" ")),
                    publication_invocation: None,
                },
            ))
        }
        Some("runs") => Ok(LocalDaemonRequest::ListWorkflowRuns(
            ListWorkflowRunsRequest {
                session_id: session.id().to_string(),
                workflow_ref: args.get(1).cloned(),
            },
        )),
        Some("get-run" | "run-status") => {
            let Some(workflow_run_ref) = args.get(1) else {
                return Err(meta_command_error("usage: workflow get-run <run-ref>"));
            };
            if args.len() > 2 {
                return Err(meta_command_error("usage: workflow get-run <run-ref>"));
            }
            Ok(LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: workflow_run_ref.clone(),
            }))
        }
        Some("cancel") => {
            let Some(workflow_run_ref) = args.get(1) else {
                return Err(meta_command_error("usage: workflow cancel <run-ref>"));
            };
            if args.len() > 2 {
                return Err(meta_command_error("usage: workflow cancel <run-ref>"));
            }
            Ok(LocalDaemonRequest::CancelWorkflowRun(
                CancelWorkflowRunRequest {
                    session_id: session.id().to_string(),
                    workflow_run_ref: workflow_run_ref.clone(),
                },
            ))
        }
        Some("resume") => {
            let Some(workflow_run_ref) = args.get(1) else {
                return Err(meta_command_error("usage: workflow resume <run-ref>"));
            };
            if args.len() > 2 {
                return Err(meta_command_error("usage: workflow resume <run-ref>"));
            }
            Ok(LocalDaemonRequest::ResumeWorkflowRun(
                ResumeWorkflowRunRequest {
                    session_id: session.id().to_string(),
                    workflow_run_ref: workflow_run_ref.clone(),
                },
            ))
        }
        _ => Err(meta_command_error(
            "usage: workflow <list|new|resolve|alias|node|endpoint|edge|run|runs|get-run|cancel|resume> ...",
        )),
    }
}

pub(super) fn meta_extension_import_request(
    session: &crate::session::RuntimeSession,
    args: &[String],
) -> Result<LocalDaemonRequest, DaemonError> {
    if args.first().map(String::as_str) != Some("import")
        || args.get(1).map(String::as_str) != Some("providers")
    {
        return Err(meta_command_error(
            "usage: extension import providers [--provider codex|opencode|claude] [--kind all|mcp|skill] [--name <capability>] [--dry-run]",
        ));
    }
    let mut providers = Vec::new();
    let mut kind = None;
    let mut name = None;
    let mut dry_run = false;
    let mut index = 2;
    while index < args.len() {
        match args[index].as_str() {
            "--provider" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(meta_command_error(
                        "usage: extension import providers --provider <provider>",
                    ));
                };
                if value.starts_with("--") {
                    return Err(meta_command_error(
                        "usage: extension import providers --provider <provider>",
                    ));
                }
                providers.push(value.clone());
                index += 2;
            }
            "--kind" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(meta_command_error(
                        "usage: extension import providers --kind all|mcp|skill",
                    ));
                };
                if value.starts_with("--") {
                    return Err(meta_command_error(
                        "usage: extension import providers --kind all|mcp|skill",
                    ));
                }
                kind = Some(value.clone());
                index += 2;
            }
            "--name" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(meta_command_error(
                        "usage: extension import providers --name <capability>",
                    ));
                };
                if value.starts_with("--") {
                    return Err(meta_command_error(
                        "usage: extension import providers --name <capability>",
                    ));
                }
                name = Some(value.clone());
                index += 2;
            }
            "--dry-run" => {
                dry_run = true;
                index += 1;
            }
            other => {
                return Err(meta_command_error(format!(
                    "unsupported extension import option `{other}`"
                )));
            }
        }
    }
    Ok(LocalDaemonRequest::ImportProviderCapabilities(
        ImportProviderCapabilitiesRequest {
            workspace_id: Some(session.workspace_id().to_string()),
            providers,
            kind,
            name,
            dry_run,
        },
    ))
}

pub(super) fn meta_extension_request(
    session: &crate::session::RuntimeSession,
    metaagent: &crate::agent::AgentInstance,
    kind: ExtensionKind,
    family: &str,
    args: &[String],
    agents: &[crate::agent::AgentInstance],
) -> Result<LocalDaemonRequest, DaemonError> {
    match args.first().map(String::as_str) {
        Some("list" | "ls") => match kind {
            ExtensionKind::Mcp => Ok(LocalDaemonRequest::ListMcpServers(ListMcpServersRequest {
                workspace_id: Some(session.workspace_id().to_string()),
            })),
            ExtensionKind::Skill => Ok(LocalDaemonRequest::ListSkills(ListSkillsRequest {
                workspace_id: Some(session.workspace_id().to_string()),
            })),
            _ => Err(meta_command_error(format!(
                "`{family} list` is not supported by Meta-mode run_command"
            ))),
        },
        Some("show" | "get") => {
            let Some(name) = args.get(1) else {
                return Err(meta_command_error(format!("usage: {family} show <name>")));
            };
            if args.len() > 2 {
                return Err(meta_command_error(format!("usage: {family} show <name>")));
            }
            match kind {
                ExtensionKind::Mcp => Ok(LocalDaemonRequest::GetMcpServer(GetMcpServerRequest {
                    workspace_id: Some(session.workspace_id().to_string()),
                    name: name.clone(),
                })),
                ExtensionKind::Skill => Ok(LocalDaemonRequest::GetSkill(GetSkillRequest {
                    workspace_id: Some(session.workspace_id().to_string()),
                    name: name.clone(),
                })),
                _ => Err(meta_command_error(format!(
                    "`{family} show` is not supported by Meta-mode run_command"
                ))),
            }
        }
        Some("install-json" | "update-json") if kind == ExtensionKind::Mcp => {
            let Some(json) = args.get(1) else {
                return Err(meta_command_error(format!(
                    "usage: {family} install-json <mcp-json>"
                )));
            };
            if args.len() > 2 {
                return Err(meta_command_error(format!(
                    "usage: {family} install-json <mcp-json>"
                )));
            }
            let config = serde_json::from_str::<crate::mcp::CharioxMcpServerConfig>(json)
                .map_err(|error| meta_command_error(format!("invalid MCP JSON config: {error}")))?;
            if args.first().map(String::as_str) == Some("install-json") {
                Ok(LocalDaemonRequest::InstallMcpServer(
                    InstallMcpServerRequest {
                        workspace_id: Some(session.workspace_id().to_string()),
                        config,
                    },
                ))
            } else {
                Ok(LocalDaemonRequest::UpdateMcpServer(
                    UpdateMcpServerRequest {
                        workspace_id: Some(session.workspace_id().to_string()),
                        config,
                    },
                ))
            }
        }
        Some("install" | "update") if kind == ExtensionKind::Skill => {
            let Some(source_path) = args.get(1) else {
                return Err(meta_command_error(format!(
                    "usage: {family} install <path>"
                )));
            };
            if args.len() > 2 {
                return Err(meta_command_error(format!(
                    "usage: {family} install <path>"
                )));
            }
            let source_path = std::path::PathBuf::from(source_path);
            if args.first().map(String::as_str) == Some("install") {
                Ok(LocalDaemonRequest::InstallSkill(InstallSkillRequest {
                    workspace_id: Some(session.workspace_id().to_string()),
                    source_path,
                }))
            } else {
                Ok(LocalDaemonRequest::UpdateSkill(UpdateSkillRequest {
                    workspace_id: Some(session.workspace_id().to_string()),
                    source_path,
                }))
            }
        }
        Some("uninstall" | "remove") => {
            let Some(name) = args.get(1) else {
                return Err(meta_command_error(format!(
                    "usage: {family} uninstall <name>"
                )));
            };
            if args.len() > 2 {
                return Err(meta_command_error(format!(
                    "usage: {family} uninstall <name>"
                )));
            }
            match kind {
                ExtensionKind::Mcp => Ok(LocalDaemonRequest::UninstallMcpServer(
                    UninstallMcpServerRequest {
                        workspace_id: Some(session.workspace_id().to_string()),
                        name: name.clone(),
                    },
                )),
                ExtensionKind::Skill => {
                    Ok(LocalDaemonRequest::UninstallSkill(UninstallSkillRequest {
                        workspace_id: Some(session.workspace_id().to_string()),
                        name: name.clone(),
                    }))
                }
                _ => Err(meta_command_error(format!(
                    "`{family} uninstall` is not supported by Meta-mode run_command"
                ))),
            }
        }
        Some("import") => {
            let Some(provider) = args.get(1) else {
                return Err(meta_command_error(format!(
                    "usage: {family} import <provider> [name]"
                )));
            };
            if args.len() > 3 {
                return Err(meta_command_error(format!(
                    "usage: {family} import <provider> [name]"
                )));
            }
            match kind {
                ExtensionKind::Mcp => Ok(LocalDaemonRequest::ImportMcpServers(
                    ImportMcpServersRequest {
                        workspace_id: Some(session.workspace_id().to_string()),
                        provider: provider.clone(),
                        name: args.get(2).cloned(),
                    },
                )),
                ExtensionKind::Skill => Ok(LocalDaemonRequest::ImportSkills(ImportSkillsRequest {
                    workspace_id: Some(session.workspace_id().to_string()),
                    provider: provider.clone(),
                    name: args.get(2).cloned(),
                })),
                _ => Err(meta_command_error(format!(
                    "`{family} import` is not supported by Meta-mode run_command"
                ))),
            }
        }
        Some("grant") => {
            if args.len() != 3 {
                return Err(meta_command_error(format!(
                    "usage: {family} grant <owned-agent-ref> <name>"
                )));
            }
            let agent = meta_owned_regular_agent_from_session(agents, metaagent, &args[1])?;
            Ok(LocalDaemonRequest::GrantAgentExtension(
                GrantAgentExtensionRequest {
                    workspace_id: Some(session.workspace_id().to_string()),
                    agent_ref: agent.agent_ref().to_string(),
                    kind,
                    name: args[2].clone(),
                    environment: None,
                    credential: None,
                    max_safety: None,
                },
            ))
        }
        Some("revoke") => {
            if args.len() != 3 {
                return Err(meta_command_error(format!(
                    "usage: {family} revoke <owned-agent-ref> <name>"
                )));
            }
            let agent = meta_owned_regular_agent_from_session(agents, metaagent, &args[1])?;
            Ok(LocalDaemonRequest::RevokeAgentExtension(
                RevokeAgentExtensionRequest {
                    agent_ref: agent.agent_ref().to_string(),
                    kind,
                    name: args[2].clone(),
                },
            ))
        }
        _ => Err(meta_command_error(format!(
            "usage: {family} <list|show|install|update|uninstall|import|grant|revoke> ..."
        ))),
    }
}

fn parse_meta_bool(value: &str) -> Result<bool, DaemonError> {
    match value {
        "true" | "yes" | "on" | "1" => Ok(true),
        "false" | "no" | "off" | "0" => Ok(false),
        _ => Err(meta_command_error(format!(
            "expected true or false, got `{value}`"
        ))),
    }
}

pub(super) fn meta_credential_request(
    session: &crate::session::RuntimeSession,
    metaagent: &crate::agent::AgentInstance,
    args: &[String],
) -> Result<LocalDaemonRequest, DaemonError> {
    match args.first().map(String::as_str) {
        Some("list" | "ls") | None => {
            Ok(LocalDaemonRequest::ListCredentials(ListCredentialsRequest))
        }
        Some("get" | "show") => {
            let Some(id) = args.get(1) else {
                return Err(meta_command_error("usage: credential get <id>"));
            };
            if args.len() > 2 {
                return Err(meta_command_error("usage: credential get <id>"));
            }
            Ok(LocalDaemonRequest::GetCredential(GetCredentialRequest {
                id: id.clone(),
            }))
        }
        Some("upsert-json") => {
            let Some(json) = args.get(1) else {
                return Err(meta_command_error(
                    "usage: credential upsert-json <credential-json>",
                ));
            };
            if args.len() > 2 {
                return Err(meta_command_error(
                    "usage: credential upsert-json <credential-json>",
                ));
            }
            let credential = serde_json::from_str::<crate::config::UserCredentialConfig>(json)
                .map_err(|error| {
                    meta_command_error(format!("invalid credential JSON config: {error}"))
                })?;
            Ok(LocalDaemonRequest::UpsertCredential(
                UpsertCredentialRequest { credential },
            ))
        }
        Some("remove") => {
            let Some(id) = args.get(1) else {
                return Err(meta_command_error("usage: credential remove <id>"));
            };
            if args.len() > 2 {
                return Err(meta_command_error("usage: credential remove <id>"));
            }
            Ok(LocalDaemonRequest::RemoveCredential(
                RemoveCredentialRequest { id: id.clone() },
            ))
        }
        Some("vault") => match args.get(1).map(String::as_str) {
            Some("status") => Ok(LocalDaemonRequest::GetCredentialVaultStatus(
                GetCredentialVaultStatusRequest,
            )),
            Some("manage") => Ok(LocalDaemonRequest::ManageCredentialVault(
                ManageCredentialVaultRequest {
                    session_id: session.id().to_string(),
                    agent_id: Some(metaagent.id().to_string()),
                },
            )),
            _ => Err(meta_command_error(
                "usage: credential vault <status|manage>",
            )),
        },
        _ => Err(meta_command_error(
            "usage: credential <list|get|upsert-json|remove|vault> ...",
        )),
    }
}

pub(super) fn meta_kernel_command(
    provider_run: Option<&crate::provider::RuntimeProviderRun>,
    metaagent: &crate::agent::AgentInstance,
    request: &LocalDaemonRequest,
) -> KernelCommand {
    let mut command = meta_kernel_command_without_request(metaagent, request);
    command.provider_run_id = provider_run.map(|run| run.id().to_string());
    command
}

pub(super) fn meta_kernel_command_without_request(
    metaagent: &crate::agent::AgentInstance,
    request: &LocalDaemonRequest,
) -> KernelCommand {
    KernelCommand::from_local_request_with_caller(
        format!(
            "metaagent-{}-{}",
            metaagent.id(),
            crate::session::unix_epoch_ms()
        ),
        KernelCommandSource::DaemonBackground,
        KernelCaller {
            caller_id: format!("metaagent:{}", metaagent.id()),
            caller_kind: KernelCallerKind::Metaagent,
            user_id: Some(metaagent.owner_user_id().to_string()),
            client_id: Some(metaagent_command_client_id(metaagent.id())),
            machine_id: None,
            realm_id: None,
            public_key_thumbprint: None,
            metaagent_id: Some(metaagent.id().to_string()),
        },
        None,
        Some(metaagent.id().to_string()),
        request,
    )
}

pub(super) fn metaagent_command_client_id(metaagent_id: &str) -> String {
    format!("metaagent:{metaagent_id}:commands")
}
