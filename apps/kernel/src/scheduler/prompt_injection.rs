use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::session::{WorkflowMessage, WorkflowOutputValidationPolicy};
use std::path::PathBuf;

pub(crate) struct WorkflowPromptInjectionContext {
    pub endpoint_prompt: String,
    pub workflow_prompt: String,
    pub node_instructions: String,
    pub instruction_ref: Option<String>,
    pub handoff_payloads_json: Option<String>,
    pub outgoing_edge_contracts: String,
    pub control_mailbox: Option<String>,
    pub delivery_token: String,
    pub node_turn: Option<WorkflowNodeTurnPromptContext>,
    pub base_directory: Option<PathBuf>,
    pub hide_in_native_tui: bool,
}

pub(crate) struct WorkflowNodeTurnPromptContext {
    pub turn_index: u32,
    pub max_turns: Option<u32>,
    pub can_complete_workflow_run: bool,
    pub can_emit_intermediate_output: bool,
}

pub(crate) fn render_workflow_turn_prompt_from_messages(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
    node_id: &str,
    endpoint_prompt: &str,
    handoff_messages: Option<&[WorkflowMessage]>,
) -> Result<String, DaemonError> {
    let handoff_payloads_json = serialize_handoff_payloads_json(handoff_messages);
    render_workflow_turn_prompt(
        app,
        session_id,
        workflow_run_id,
        workflow_node_run_id,
        node_id,
        endpoint_prompt,
        handoff_payloads_json.as_deref(),
        None,
    )
}

pub(crate) fn render_workflow_turn_prompt(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
    node_id: &str,
    endpoint_prompt: &str,
    handoff_payloads_json: Option<&str>,
    control_mailbox: Option<&str>,
) -> Result<String, DaemonError> {
    let instruction_ref = workflow_node_instruction_reference(
        app,
        session_id,
        workflow_run_id,
        workflow_node_run_id,
        node_id,
    );
    let mailbox_content = control_mailbox.map(str::to_string).or_else(|| {
        workflow_node_control_contents(
            app,
            session_id,
            workflow_run_id,
            workflow_node_run_id,
            node_id,
        )
    });
    let delivery_token = workflow_turn_delivery_token(workflow_node_run_id);
    Ok(build_workflow_turn_prompt(WorkflowPromptInjectionContext {
        endpoint_prompt: endpoint_prompt.to_string(),
        workflow_prompt: app
            .sessions()
            .resolve_workflow_run_ref(session_id, workflow_run_id)
            .ok()
            .and_then(|run| run.invocation_prompt().map(str::to_string))
            .unwrap_or_default(),
        node_instructions: workflow_node_instructions(app, session_id, workflow_run_id, node_id),
        instruction_ref,
        handoff_payloads_json: handoff_payloads_json.map(str::to_string),
        outgoing_edge_contracts: workflow_outgoing_edge_contracts_block(
            app,
            session_id,
            workflow_run_id,
            node_id,
        ),
        control_mailbox: mailbox_content,
        delivery_token,
        node_turn: workflow_node_prompt_context(app, session_id, workflow_run_id, node_id),
        base_directory: workflow_runtime_base_directory(
            app,
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        ),
        hide_in_native_tui: false,
    }))
}

pub(crate) fn build_workflow_turn_prompt(context: WorkflowPromptInjectionContext) -> String {
    let reference_line = context
        .instruction_ref
        .as_deref()
        .map(|path| format!("Node instruction reference (daemon-managed): {path}\n\n"))
        .unwrap_or_default();
    let control_line = context
        .control_mailbox
        .as_deref()
        .map(|content| {
            format!(
                "Control mailbox:\n{content}\nTreat the control mailbox as authoritative runtime feedback for this node. Fix every listed issue in this turn before you finalize the workflow output.\n\n"
            )
        })
        .unwrap_or_default();
    let payload_block = if context
        .handoff_payloads_json
        .as_deref()
        .is_none_or(|payloads| payloads.trim().is_empty() || payloads.trim() == "[]")
    {
        String::new()
    } else {
        format!(
            "Workflow handoff payloads (JSON array):\n{}\n\n",
            context.handoff_payloads_json.as_deref().unwrap_or("[]")
        )
    };
    let entry_line = if context.endpoint_prompt.trim().is_empty() {
        String::new()
    } else {
        format!("Endpoint prompt:\n{}\n\n", context.endpoint_prompt.trim())
    };
    let system_prompt = render_workflow_system_prompt(
        context.base_directory.as_ref(),
        &context.delivery_token,
        &payload_block,
        &context.outgoing_edge_contracts,
        &reference_line,
        &control_line,
    );
    let system_node_prompt =
        render_workflow_node_system_prompt(context.base_directory.as_ref(), &context.node_turn);
    let workflow_instructions = format!(
        "Workflow-level prompt:\n{}\n\nNode-level instructions:\n{}\n\n{}\n{}",
        context.workflow_prompt,
        context.node_instructions,
        system_prompt,
        system_node_prompt
    );
    if context.hide_in_native_tui {
        format!(
            "{}{}",
            entry_line,
            crate::provider::native_tui_hidden_instructions_block(&workflow_instructions)
        )
    } else {
        format!("{entry_line}{workflow_instructions}")
    }
}

fn workflow_node_instructions(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    node_id: &str,
) -> String {
    app.sessions()
        .resolve_workflow_run_ref(session_id, workflow_run_id)
        .ok()
        .and_then(|workflow_run| {
            app.sessions()
                .resolve_workflow_ref(session_id, workflow_run.workflow_id())
                .ok()
        })
        .and_then(|workflow| {
            workflow
                .node(node_id)
                .and_then(|node| node.instructions())
                .map(str::to_string)
        })
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "No node-specific instructions were configured.".to_string())
}

fn render_workflow_system_prompt(
    base_directory: Option<&PathBuf>,
    delivery_token: &str,
    payload_block: &str,
    edge_contract_block: &str,
    reference_line: &str,
    control_line: &str,
) -> String {
    let template = load_workflow_system_prompt_template(base_directory);
    template
        .replace("{{DELIVERY_TOKEN}}", delivery_token)
        .replace("{{WORKFLOW_HANDOFF_PAYLOADS_BLOCK}}", payload_block)
        .replace("{{OUTGOING_EDGE_CONTRACTS_BLOCK}}", edge_contract_block)
        .replace("{{NODE_INSTRUCTION_REFERENCE_BLOCK}}", reference_line)
        .replace("{{CONTROL_MAILBOX_BLOCK}}", control_line)
}

fn load_workflow_system_prompt_template(base_directory: Option<&PathBuf>) -> String {
    let Some(path) = workflow_system_prompt_template_path(base_directory) else {
        return default_workflow_system_prompt_template().to_string();
    };
    if !path.exists() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, default_workflow_system_prompt_template());
    }
    std::fs::read_to_string(&path)
        .unwrap_or_else(|_| default_workflow_system_prompt_template().to_string())
}

fn workflow_system_prompt_template_path(base_directory: Option<&PathBuf>) -> Option<PathBuf> {
    let base_directory = base_directory?;
    Some(
        base_directory
            .join(".arroba")
            .join("system-prompts")
            .join("workflow-turn.md"),
    )
}

fn render_workflow_node_system_prompt(
    base_directory: Option<&PathBuf>,
    context: &Option<WorkflowNodeTurnPromptContext>,
) -> String {
    let fragments = workflow_node_prompt_fragments(base_directory, context);
    if fragments.is_empty() {
        return String::new();
    }
    fragments.concat()
}

fn workflow_node_prompt_context(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    node_id: &str,
) -> Option<WorkflowNodeTurnPromptContext> {
    let workflow_run = app
        .sessions()
        .resolve_workflow_run_ref(session_id, workflow_run_id)
        .ok()?;
    let workflow = app
        .sessions()
        .resolve_workflow_ref(session_id, workflow_run.workflow_id())
        .ok()?;
    let node = workflow.node(node_id)?;
    let turn_index = workflow_run
        .node_runs()
        .iter()
        .filter(|node_run| node_run.node_id() == node_id)
        .count() as u32;
    Some(WorkflowNodeTurnPromptContext {
        turn_index,
        max_turns: node.max_turns(),
        can_complete_workflow_run: node.can_complete_workflow_run(),
        can_emit_intermediate_output: node.can_emit_intermediate_run_output(),
    })
}

fn workflow_node_prompt_fragments(
    base_directory: Option<&PathBuf>,
    context: &Option<WorkflowNodeTurnPromptContext>,
) -> Vec<String> {
    let mut fragments = Vec::new();
    if let Some(context) = context {
        fragments.push(workflow_node_turn_index_block(context));
        if context.can_emit_intermediate_output {
            if let Some(fragment) =
                load_workflow_run_intermediate_output_prompt_template(base_directory)
            {
                fragments.push(fragment);
            }
        }
        if context.can_complete_workflow_run {
            if let Some(fragment) = load_workflow_run_completion_prompt_template(base_directory) {
                fragments.push(fragment);
            }
        }
        if let Some(fragment) = workflow_last_turn_notice_block(context) {
            fragments.push(fragment);
        }
    }
    fragments
}

fn workflow_node_turn_index_block(context: &WorkflowNodeTurnPromptContext) -> String {
    let mut block = format!(
        "System node-level prompt:\nThis is turn {} for this node in the current workflow run.\n",
        context.turn_index
    );
    if let Some(max_turns) = context.max_turns {
        block.push_str(&format!("- node max turns: {max_turns}\n"));
    }
    block.push('\n');
    block
}

fn load_workflow_run_completion_prompt_template(
    base_directory: Option<&PathBuf>,
) -> Option<String> {
    let path = workflow_run_completion_prompt_template_path(base_directory)?;
    if !path.exists() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, default_workflow_run_completion_prompt_template());
    }
    Some(
        std::fs::read_to_string(&path)
            .unwrap_or_else(|_| default_workflow_run_completion_prompt_template().to_string()),
    )
}

fn workflow_run_completion_prompt_template_path(
    base_directory: Option<&PathBuf>,
) -> Option<PathBuf> {
    let base_directory = base_directory?;
    Some(
        base_directory
            .join(".arroba")
            .join("system-prompts")
            .join("workflow-run-completion.md"),
    )
}

fn load_workflow_run_intermediate_output_prompt_template(
    base_directory: Option<&PathBuf>,
) -> Option<String> {
    let path = workflow_run_intermediate_output_prompt_template_path(base_directory)?;
    if !path.exists() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(
            &path,
            default_workflow_run_intermediate_output_prompt_template(),
        );
    }
    Some(
        std::fs::read_to_string(&path).unwrap_or_else(|_| {
            default_workflow_run_intermediate_output_prompt_template().to_string()
        }),
    )
}

fn workflow_run_intermediate_output_prompt_template_path(
    base_directory: Option<&PathBuf>,
) -> Option<PathBuf> {
    let base_directory = base_directory?;
    Some(
        base_directory
            .join(".arroba")
            .join("system-prompts")
            .join("workflow-run-intermediate-output.md"),
    )
}

fn workflow_last_turn_notice_block(context: &WorkflowNodeTurnPromptContext) -> Option<String> {
    let max_turns = context.max_turns?;
    if context.turn_index != max_turns {
        return None;
    }
    Some(format!(
        "System node-level prompt:\nThis is the last allowed turn for this node in the current workflow run.\n- node turn index: {turn_index}\n- node max turns: {max_turns}\nIf you consider that the workflow is complete and the run should stop, or will stop by design at this node, generate final workflow run output in this turn. In that case, normal node-to-node output is not necessary and does not need `validate_workflow_output`. Instead, call the Arroba runtime MCP tool `validate_and_submit_workflow_run_output` and do not finalize the turn until it returns `valid: true` with no warning.\n\n",
        turn_index = context.turn_index
    ))
}

fn workflow_outgoing_edge_contracts_block(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    node_id: &str,
) -> String {
    let Some(workflow_run) = app
        .sessions()
        .resolve_workflow_run_ref(session_id, workflow_run_id)
        .ok()
    else {
        return String::new();
    };
    let Some(workflow) = app
        .sessions()
        .resolve_workflow_ref(session_id, workflow_run.workflow_id())
        .ok()
    else {
        return String::new();
    };

    let lines = workflow
        .edges()
        .iter()
        .filter(|edge| edge.from_node_id() == node_id)
        .map(|edge| {
            let mut line = format!("- edge {} -> {}", edge.id(), edge.to_node_id());
            if let Some(schema_ref) = edge.output_schema_ref() {
                line.push_str(&format!(", output_schema_ref: {schema_ref}"));
            }
            if let Some(validation_policy) = edge.validation_policy() {
                let validation_policy = match validation_policy {
                    WorkflowOutputValidationPolicy::Warn => "warn",
                    WorkflowOutputValidationPolicy::Halt => "halt",
                };
                line.push_str(&format!(", validation_policy: {validation_policy}"));
            }
            line
        })
        .collect::<Vec<String>>();

    if lines.is_empty() {
        return String::new();
    }

    format!(
        "Outgoing edge contracts:\n{}\nAll schema refs needed for this turn are listed above. Do not search the workspace for workflow metadata unless the workflow-level prompt explicitly asks you to.\n\n",
        lines.join("\n")
    )
}

fn workflow_node_instruction_reference(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
    node_id: &str,
) -> Option<String> {
    let workflow_run = app
        .sessions()
        .resolve_workflow_run_ref(session_id, workflow_run_id)
        .ok()?;
    let workflow = app
        .sessions()
        .resolve_workflow_ref(session_id, workflow_run.workflow_id())
        .ok()?;
    let node = workflow.node(node_id);
    let root = workflow_runtime_artifact_root(
        app,
        session_id,
        workflow_run_id,
        workflow_node_run_id,
        "workflow-instructions",
    )?;
    let filename = format!("node-{node_id}.md");
    let path = root.join(filename);
    if !path.exists() || node.and_then(|node| node.instructions()).is_some() {
        if let Err(error) = std::fs::create_dir_all(&root) {
            tracing::debug!(
                ?error,
                "Failed to create workflow instruction directory at {:?}",
                root
            );
            return None;
        }
        let content = node
            .and_then(|node| node.instructions())
            .map(|value| value.to_string())
            .unwrap_or_else(|| {
                format!(
                    "# Workflow Node Instructions\n\nThis file is daemon-managed. Update node instructions through workflow configuration tooling.\n\nNode: {node_id}\n"
                )
            });
        if let Err(error) = std::fs::write(&path, content) {
            tracing::debug!(
                ?error,
                "Failed to write workflow instruction file at {:?}",
                path
            );
            return None;
        }
    }
    Some(path.to_string_lossy().to_string())
}

fn workflow_node_control_contents(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
    node_id: &str,
) -> Option<String> {
    let root = workflow_runtime_artifact_root(
        app,
        session_id,
        workflow_run_id,
        workflow_node_run_id,
        "workflow-control",
    )?;
    let path = root.join(format!("node-{node_id}.md"));
    std::fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn workflow_runtime_artifact_root(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
    category: &str,
) -> Option<std::path::PathBuf> {
    let base_directory =
        workflow_runtime_base_directory(app, session_id, workflow_run_id, workflow_node_run_id)?;
    Some(
        base_directory
            .join(".arroba")
            .join("workflow-runtime")
            .join(session_id)
            .join(workflow_run_id)
            .join(category),
    )
}

fn workflow_runtime_base_directory(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
) -> Option<std::path::PathBuf> {
    let session = app.sessions().get_session(session_id).ok()?;
    let workflow_run = session.workflow_run(workflow_run_id)?;
    let node_run = workflow_run
        .node_runs()
        .iter()
        .find(|candidate| candidate.id() == workflow_node_run_id)?;
    let base_directory = app
        .providers()
        .get_latest_run_for_agent(session_id, node_run.agent_id())
        .and_then(|run| run.working_directory().cloned())
        .or_else(|| {
            let worktree = std::path::PathBuf::from(session.worktree_id());
            if worktree.is_absolute() {
                Some(worktree)
            } else {
                std::env::current_dir().ok().map(|cwd| cwd.join(worktree))
            }
        })?;
    Some(base_directory)
}

fn serialize_handoff_payloads_json(handoff_messages: Option<&[WorkflowMessage]>) -> Option<String> {
    let handoff_payloads = handoff_messages
        .map(|messages| {
            messages
                .iter()
                .map(|message| {
                    serde_json::from_str::<serde_json::Value>(message.handoff_payload())
                        .unwrap_or_else(|_| {
                            serde_json::Value::String(message.handoff_payload().to_string())
                        })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if handoff_payloads.is_empty() {
        None
    } else {
        serde_json::to_string_pretty(&handoff_payloads).ok()
    }
}

fn workflow_turn_delivery_token(workflow_node_run_id: &str) -> String {
    format!("workflow-ack:{workflow_node_run_id}")
}

fn default_workflow_system_prompt_template() -> &'static str {
    "You are an agent participating in an Arroba workflow turn.\n\n{{NODE_INSTRUCTION_REFERENCE_BLOCK}}Your node-level instructions are in the referenced markdown file above. If you do not remember them exactly, read that file before continuing.\n\n{{WORKFLOW_HANDOFF_PAYLOADS_BLOCK}}{{OUTGOING_EDGE_CONTRACTS_BLOCK}}{{CONTROL_MAILBOX_BLOCK}}For the proper behavior of the workflow, you MUST acknowledge that you have successfully read the current input from the queue by calling the Arroba runtime MCP tool `ack_workflow_turn` exactly once with this JSON argument object:\n{\"delivery_token\":\"{{DELIVERY_TOKEN}}\"}\n\nIf an outgoing edge contract for this turn includes an `output_schema_ref`, you MUST validate your proposed `output.message` before finalizing by calling the Arroba runtime MCP tool `validate_workflow_output` with the delivery token above, that `output_schema_ref`, and your proposed `output.message` JSON. If no `output_schema_ref` is present for this turn, do not call `validate_workflow_output`.\n\nIf your node-level instructions require shared console output or inspection, you MUST use the Arroba runtime MCP tools `workflow_console_read`, `workflow_console_write`, and `workflow_console_clear` for that work.\n\nAt the end of this workflow turn, return exactly one fenced ```json block with this shape:\n{\"summary\":\"human-facing summary\",\"output\":{\"message\":\"explicit downstream output message\"}}\nDo not output any prose before or after that fenced block. Do not mention acknowledgments, tool calls, or workflow mechanics in the summary unless the task explicitly requires it. The downstream payload is only output.message plus any workflow-owned artifacts.\n\nIf a Control mailbox is present, resolve every listed issue before finalizing and do not repeat the invalid payload. When this turn includes an `output_schema_ref`, validation is a gate, not a suggestion. If `validate_workflow_output` returns `valid: false` or any warning, do not finalize the turn yet. Revise the proposed output, call `validate_workflow_output` again, and only finalize once the tool returns `valid: true` with no warning. A single failed validation call does not satisfy this turn's completion requirements."
}

fn default_workflow_run_completion_prompt_template() -> &'static str {
    "System node-level prompt:\nThis node is authorized to complete the workflow run.\nIf you consider that the workflow is complete and the run should stop, or will stop by design at this node, generate final workflow run output and submit it by calling the Arroba runtime MCP tool `validate_and_submit_workflow_run_output`.\nWhen you are generating final workflow run output, normal node-to-node output is not necessary and does not need `validate_workflow_output`.\nDo not finalize the turn until `validate_and_submit_workflow_run_output` returns `valid: true` with no warning.\n\n"
}

fn default_workflow_run_intermediate_output_prompt_template() -> &'static str {
    "System node-level prompt:\nThis node is authorized to emit intermediate workflow run outputs.\nIf you want to send an intermediate output to the endpoint without terminating the workflow run, call the Arroba runtime MCP tool `validate_and_submit_intermediate_workflow_run_output`.\nIntermediate workflow run output does not terminate the workflow run. You may still need to produce normal node-to-node output for downstream workflow edges in the same turn, and downstream output validation rules still apply.\nDo not finalize the turn until `validate_and_submit_intermediate_workflow_run_output` returns `valid: true` with no warning.\n\n"
}
