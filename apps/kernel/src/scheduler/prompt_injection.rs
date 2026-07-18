use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::prompt_assembly::{
    assembled_prompt_component, bundled_metaagent_event_template,
    bundled_workflow_run_completion_template, bundled_workflow_run_intermediate_output_template,
    bundled_workflow_turn_template, prompt_component, unescape_prompt_component_delimiters,
    PromptManifest, PromptTemplate, PromptTemplateRegistry,
};
use crate::session::{
    WorkflowDefinition, WorkflowEdgeDefinition, WorkflowHandoffValidationPolicy, WorkflowMessage,
};
use std::path::PathBuf;

const ENDPOINT_PROMPT_TAG: &str = "endpoint-prompt";
const WORKFLOW_LEVEL_PROMPT_TAG: &str = "workflow-level-prompt";
const NODE_LEVEL_PROMPT_TAG: &str = "node-level-prompt";
const WORKFLOW_RUNTIME_INSTRUCTIONS_TAG: &str = "workflow-runtime-instructions";
const SYSTEM_NODE_LEVEL_PROMPT_TAG: &str = "system-node-level-prompt";
const WORKFLOW_HANDOFF_PAYLOADS_TAG: &str = "workflow-handoff-payloads";
const OUTGOING_EDGE_CONTRACTS_TAG: &str = "outgoing-edge-contracts";
const NODE_INSTRUCTION_REFERENCE_TAG: &str = "node-instruction-reference";
const CONTROL_MAILBOX_TAG: &str = "control-mailbox";

pub(crate) struct WorkflowPromptInjectionContext {
    pub workflow_ref: Option<String>,
    pub endpoint_prompt: String,
    pub workflow_prompt: String,
    pub node_id: Option<String>,
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
    pub wait_for_all_inputs: bool,
}

pub(crate) struct MetaagentEventPromptContext {
    pub event_id: String,
    pub event_kind: String,
    pub source: String,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkflowTurnPromptAssembly {
    pub(crate) visible_user_prompt: String,
    pub(crate) hidden_system_context: String,
    pub(crate) manifest: PromptManifest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetaagentEventPromptAssembly {
    pub(crate) visible_user_prompt: String,
    pub(crate) manifest: PromptManifest,
}

pub(crate) fn render_metaagent_event_prompt_assembly(
    context: MetaagentEventPromptContext,
) -> MetaagentEventPromptAssembly {
    let mut manifest = PromptManifest::current();
    let template = load_prompt_registry_template(
        "runtime/metaagent-event",
        bundled_metaagent_event_template(),
    );
    manifest.push_body(template.id.clone(), &template.body);
    let visible_user_prompt = template
        .body
        .replace("{{EVENT_ID}}", context.event_id.trim())
        .replace("{{EVENT_KIND}}", context.event_kind.trim())
        .replace("{{SOURCE}}", context.source.trim())
        .replace("{{TITLE}}", context.title.trim())
        .replace("{{BODY}}", context.body.trim());
    let visible_user_prompt = prompt_component("metaagent-event", &visible_user_prompt);
    MetaagentEventPromptAssembly {
        visible_user_prompt,
        manifest,
    }
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
    render_workflow_turn_prompt_assembly(
        app,
        session_id,
        workflow_run_id,
        workflow_node_run_id,
        node_id,
        endpoint_prompt,
        handoff_payloads_json,
        control_mailbox,
    )
    .map(|assembly| workflow_assembly_legacy_prompt(&assembly, false))
}

pub(crate) fn render_workflow_turn_prompt_assembly(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
    node_id: &str,
    endpoint_prompt: &str,
    handoff_payloads_json: Option<&str>,
    control_mailbox: Option<&str>,
) -> Result<WorkflowTurnPromptAssembly, DaemonError> {
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
    let workflow_run = app
        .sessions()
        .resolve_workflow_run_ref(session_id, workflow_run_id)
        .ok();
    let workflow = workflow_run.as_ref().and_then(|run| {
        app.sessions()
            .resolve_workflow_ref(session_id, run.workflow_id())
            .ok()
    });
    Ok(build_workflow_turn_prompt_assembly(
        WorkflowPromptInjectionContext {
            workflow_ref: workflow_run
                .as_ref()
                .map(|run| run.workflow_id().to_string()),
            endpoint_prompt: endpoint_prompt.to_string(),
            workflow_prompt: workflow
                .as_ref()
                .and_then(|workflow| workflow.prompt().map(str::to_string))
                .unwrap_or_default(),
            node_id: Some(node_id.to_string()),
            node_instructions: workflow_node_instructions(
                app,
                session_id,
                workflow_run_id,
                node_id,
            ),
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
        },
    ))
}

pub(crate) fn build_workflow_turn_prompt(context: WorkflowPromptInjectionContext) -> String {
    let hide_in_native_tui = context.hide_in_native_tui;
    let assembly = build_workflow_turn_prompt_assembly(context);
    workflow_assembly_legacy_prompt(&assembly, hide_in_native_tui)
}

pub(crate) fn build_workflow_turn_prompt_assembly(
    context: WorkflowPromptInjectionContext,
) -> WorkflowTurnPromptAssembly {
    let mut manifest = PromptManifest::current();
    let reference_line = context
        .instruction_ref
        .as_deref()
        .map(|path| prompt_component(NODE_INSTRUCTION_REFERENCE_TAG, path))
        .map(|component| format!("{component}\n\n"))
        .unwrap_or_default();
    let control_line = context
        .control_mailbox
        .as_deref()
        .map(|content| {
            prompt_component(
                CONTROL_MAILBOX_TAG,
                &format!(
                    "{content}\nTreat the control mailbox as authoritative runtime feedback for this node. Fix every listed issue in this turn before you finalize the workflow output."
                ),
            )
        })
        .map(|component| format!("{component}\n\n"))
        .unwrap_or_default();
    let handoff_payload_prompt = if context
        .handoff_payloads_json
        .as_deref()
        .is_none_or(|payloads| payloads.trim().is_empty() || payloads.trim() == "[]")
    {
        String::new()
    } else {
        prompt_component(
            WORKFLOW_HANDOFF_PAYLOADS_TAG,
            context.handoff_payloads_json.as_deref().unwrap_or("[]"),
        )
    };
    let outgoing_edge_contracts_block = if context.outgoing_edge_contracts.trim().is_empty() {
        String::new()
    } else {
        format!(
            "{}\n\n",
            prompt_component(
                OUTGOING_EDGE_CONTRACTS_TAG,
                &format!(
                    "{}\nAll schema refs needed for this turn are listed above. Do not search the workspace for workflow metadata unless the workflow-level prompt explicitly asks you to.",
                    strip_legacy_prompt_heading(
                        &context.outgoing_edge_contracts,
                        "Outgoing edge contracts:",
                    )
                ),
            )
        )
    };
    let visible_user_prompt = [
        (!context.endpoint_prompt.trim().is_empty())
            .then(|| prompt_component(ENDPOINT_PROMPT_TAG, &context.endpoint_prompt)),
        (!handoff_payload_prompt.is_empty()).then_some(handoff_payload_prompt),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("\n\n");
    let visible_user_prompt = if visible_user_prompt.is_empty() {
        visible_user_prompt
    } else {
        format!("{visible_user_prompt}\n\n")
    };
    let system_prompt = assembled_prompt_component(
        WORKFLOW_RUNTIME_INSTRUCTIONS_TAG,
        &render_workflow_system_prompt(
            context.base_directory.as_ref(),
            &mut manifest,
            &context.delivery_token,
            "",
            &outgoing_edge_contracts_block,
            &reference_line,
            &control_line,
        ),
    );
    let system_node_prompt = render_workflow_node_system_prompt(
        context.base_directory.as_ref(),
        &context.node_turn,
        &mut manifest,
    );
    if let Some(workflow_ref) = context.workflow_ref.as_deref() {
        manifest.push_body(
            format!("workflow/{workflow_ref}/prompt"),
            &context.workflow_prompt,
        );
    }
    if let Some(node_id) = context.node_id.as_deref() {
        manifest.push_body(
            format!("workflow-node/{node_id}/instructions"),
            &context.node_instructions,
        );
    }
    let mut instruction_sections = Vec::new();
    if !context.workflow_prompt.trim().is_empty() {
        instruction_sections.push(prompt_component(
            WORKFLOW_LEVEL_PROMPT_TAG,
            &context.workflow_prompt,
        ));
    }
    if !context.node_instructions.trim().is_empty() {
        instruction_sections.push(prompt_component(
            NODE_LEVEL_PROMPT_TAG,
            &context.node_instructions,
        ));
    }
    instruction_sections.push(system_prompt);
    instruction_sections.push(system_node_prompt);
    let workflow_instructions = instruction_sections.join("\n\n");
    WorkflowTurnPromptAssembly {
        visible_user_prompt,
        hidden_system_context: workflow_instructions,
        manifest,
    }
}

fn workflow_assembly_legacy_prompt(
    assembly: &WorkflowTurnPromptAssembly,
    hide_in_native_tui: bool,
) -> String {
    if hide_in_native_tui {
        format!(
            "{}{}",
            assembly.visible_user_prompt,
            crate::provider::native_tui_hidden_instructions_block(&assembly.hidden_system_context)
        )
    } else {
        format!(
            "{}{}",
            assembly.visible_user_prompt, assembly.hidden_system_context
        )
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
    manifest: &mut PromptManifest,
    delivery_token: &str,
    payload_block: &str,
    edge_contract_block: &str,
    reference_line: &str,
    control_line: &str,
) -> String {
    let template = load_workflow_system_prompt_template(base_directory);
    manifest.push_body(template.id.clone(), &template.body);
    template
        .body
        .replace("{{DELIVERY_TOKEN}}", delivery_token)
        .replace("{{WORKFLOW_HANDOFF_PAYLOADS_BLOCK}}", payload_block)
        .replace("{{OUTGOING_EDGE_CONTRACTS_BLOCK}}", edge_contract_block)
        .replace("{{NODE_INSTRUCTION_REFERENCE_BLOCK}}", reference_line)
        .replace("{{CONTROL_MAILBOX_BLOCK}}", control_line)
}

fn load_workflow_system_prompt_template(base_directory: Option<&PathBuf>) -> PromptTemplate {
    let _ = base_directory;
    load_prompt_registry_template("workflow/turn", bundled_workflow_turn_template())
}

fn render_workflow_node_system_prompt(
    base_directory: Option<&PathBuf>,
    context: &Option<WorkflowNodeTurnPromptContext>,
    manifest: &mut PromptManifest,
) -> String {
    let fragments = workflow_node_prompt_fragments(base_directory, context, manifest);
    if fragments.is_empty() {
        return String::new();
    }
    prompt_component(SYSTEM_NODE_LEVEL_PROMPT_TAG, &fragments.concat())
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
        wait_for_all_inputs: node.wait_for_all_inputs(),
    })
}

fn workflow_node_prompt_fragments(
    base_directory: Option<&PathBuf>,
    context: &Option<WorkflowNodeTurnPromptContext>,
    manifest: &mut PromptManifest,
) -> Vec<String> {
    let mut fragments = Vec::new();
    if let Some(context) = context {
        fragments.push(workflow_node_turn_index_block(context));
        if context.can_emit_intermediate_output {
            if let Some(fragment) =
                load_workflow_run_intermediate_output_prompt_template(base_directory)
            {
                manifest.push_body(fragment.id.clone(), &fragment.body);
                fragments.push(strip_legacy_prompt_heading(
                    &fragment.body,
                    "System node-level prompt:",
                ));
            }
        }
        if context.can_complete_workflow_run {
            if let Some(fragment) = load_workflow_run_completion_prompt_template(base_directory) {
                manifest.push_body(fragment.id.clone(), &fragment.body);
                fragments.push(strip_legacy_prompt_heading(
                    &fragment.body,
                    "System node-level prompt:",
                ));
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
        "This is turn {} for this node in the current workflow run.\n",
        context.turn_index
    );
    if let Some(max_turns) = context.max_turns {
        block.push_str(&format!("- node max turns: {max_turns}\n"));
    }
    if context.wait_for_all_inputs {
        block.push_str("- this node starts only after every incoming edge has an input for the same source iteration\n");
    }
    block.push('\n');
    block
}

fn load_workflow_run_completion_prompt_template(
    base_directory: Option<&PathBuf>,
) -> Option<PromptTemplate> {
    let _ = base_directory;
    Some(load_prompt_registry_template(
        "workflow/run-completion",
        bundled_workflow_run_completion_template(),
    ))
}

fn load_workflow_run_intermediate_output_prompt_template(
    base_directory: Option<&PathBuf>,
) -> Option<PromptTemplate> {
    let _ = base_directory;
    Some(load_prompt_registry_template(
        "workflow/run-intermediate-output",
        bundled_workflow_run_intermediate_output_template(),
    ))
}

fn load_prompt_registry_template(
    template_id: &str,
    bundled_default: &'static str,
) -> PromptTemplate {
    let registry = PromptTemplateRegistry::from_env();
    if let Err(error) = registry.materialize_bundled_defaults() {
        tracing::debug!(
            ?error,
            template_id,
            "Failed to materialize prompt registry defaults"
        );
        return PromptTemplate {
            id: template_id.to_string(),
            body: bundled_default.trim().to_string(),
        };
    }
    registry.read_required(template_id).unwrap_or_else(|error| {
        tracing::debug!(
            ?error,
            template_id,
            "Failed to read prompt registry template"
        );
        PromptTemplate {
            id: template_id.to_string(),
            body: bundled_default.trim().to_string(),
        }
    })
}

fn workflow_last_turn_notice_block(context: &WorkflowNodeTurnPromptContext) -> Option<String> {
    let max_turns = context.max_turns?;
    if context.turn_index != max_turns {
        return None;
    }
    Some(format!(
        "This is the last allowed turn for this node in the current workflow run.\n- node turn index: {turn_index}\n- node max turns: {max_turns}\nIf you consider that the workflow is complete and the run should stop, or will stop by design at this node, generate final workflow run output in this turn. In that case, normal node-to-node output is not necessary and does not need `validate_workflow_handoff`. Instead, call the Arroba runtime MCP tool `validate_and_submit_workflow_run_output` and do not finalize the turn until it returns `valid: true` with no warning.\n\n",
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
        .map(|edge| workflow_outgoing_edge_contract_line(&workflow, edge))
        .collect::<Vec<String>>();

    if lines.is_empty() {
        return String::new();
    }

    lines.join("\n")
}

fn strip_legacy_prompt_heading(body: &str, heading: &str) -> String {
    body.trim()
        .strip_prefix(heading)
        .unwrap_or(body.trim())
        .trim_start()
        .to_string()
}

pub(crate) fn workflow_handoff_payloads_from_prompt(prompt: &str) -> Option<String> {
    let open = format!("<{WORKFLOW_HANDOFF_PAYLOADS_TAG}>");
    let close = format!("</{WORKFLOW_HANDOFF_PAYLOADS_TAG}>");
    prompt
        .split_once(&open)
        .and_then(|(_, rest)| rest.split_once(&close).map(|(body, _)| body))
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "[]")
        .map(unescape_prompt_component_delimiters)
        .or_else(|| {
            prompt
                .split("Workflow handoff payloads (JSON array):\n")
                .nth(1)
                .and_then(|rest| rest.split("\n\n").next())
                .map(str::trim)
                .filter(|value| !value.is_empty() && *value != "[]")
                .map(str::to_string)
        })
}

fn workflow_outgoing_edge_contract_line(
    workflow: &WorkflowDefinition,
    edge: &WorkflowEdgeDefinition,
) -> String {
    let target_label = workflow
        .node(edge.to_node_id())
        .map(|node| node.public_label())
        .filter(|label| !label.trim().is_empty());
    let target_instructions = workflow
        .node(edge.to_node_id())
        .and_then(|node| node.instructions())
        .map(compact_workflow_edge_contract_instructions)
        .filter(|instructions| !instructions.is_empty());
    let mut line = match target_label {
        Some(label) => format!("- edge {} -> {} ({label})", edge.id(), edge.to_node_id()),
        None => format!("- edge {} -> {}", edge.id(), edge.to_node_id()),
    };
    if let Some(instructions) = target_instructions {
        let escaped = serde_json::to_string(&instructions).unwrap_or_else(|_| "\"\"".to_string());
        line.push_str(&format!(", target_instructions: {escaped}"));
    }
    if let Some(schema_ref) = edge.handoff_schema_ref() {
        line.push_str(&format!(", handoff_schema_ref: {schema_ref}"));
    }
    if let Some(validation_policy) = edge.validation_policy() {
        let validation_policy = match validation_policy {
            WorkflowHandoffValidationPolicy::Warn => "warn",
            WorkflowHandoffValidationPolicy::Halt => "halt",
        };
        line.push_str(&format!(", validation_policy: {validation_policy}"));
    }
    line
}

fn compact_workflow_edge_contract_instructions(instructions: &str) -> String {
    const MAX_CHARS: usize = 240;
    let compact = instructions
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if compact.chars().count() <= MAX_CHARS {
        return compact;
    }
    let truncated = compact.chars().take(MAX_CHARS).collect::<String>();
    format!("{truncated}...")
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

#[cfg(test)]
mod tests;
