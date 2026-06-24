//! Workflow turn prompt rendering and runtime prompt-file context.
//!
//! This module owns prompt text construction for workflow node turns. Scheduling and retry state
//! transitions stay in `workflow_dispatch`.

use super::*;

impl KernelRuntimeOwnedState {
    #[allow(dead_code)]
    pub(super) fn workflow_control_mailbox_text(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        _workflow_node_run_id: &str,
    ) -> Option<String> {
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_id)
            .ok()?;
        let lines = workflow_run
            .failure_events()
            .iter()
            .map(|failure| format!("- {:?}: {}", failure.kind(), failure.message()))
            .collect::<Vec<_>>();
        (!lines.is_empty()).then(|| lines.join("\n"))
    }

    #[allow(dead_code)]
    pub(super) fn workflow_outgoing_edge_contracts_text(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        node_id: &str,
    ) -> String {
        let workflow_id = match self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_id)
        {
            Ok(run) => run.workflow_id().to_string(),
            Err(_) => return String::new(),
        };
        let Ok(workflow) = self
            .session_store
            .read()
            .resolve_workflow_ref(session_id, &workflow_id)
        else {
            return String::new();
        };
        let lines = workflow
            .edges()
            .iter()
            .filter(|edge| edge.from_node_id() == node_id)
            .map(|edge| {
                let target_label = workflow
                    .node(edge.to_node_id())
                    .map(|node| node.public_label())
                    .filter(|label| !label.trim().is_empty());
                let mut line = match target_label {
                    Some(label) => {
                        format!("- edge {} -> {} ({label})", edge.id(), edge.to_node_id())
                    }
                    None => format!("- edge {} -> {}", edge.id(), edge.to_node_id()),
                };
                if let Some(schema_ref) = edge.handoff_schema_ref() {
                    line.push_str(&format!(", handoff_schema_ref: {schema_ref}"));
                }
                if let Some(validation_policy) = edge.validation_policy() {
                    let validation_policy = match validation_policy {
                        crate::session::WorkflowHandoffValidationPolicy::Warn => "warn",
                        crate::session::WorkflowHandoffValidationPolicy::Halt => "halt",
                    };
                    line.push_str(&format!(", validation_policy: {validation_policy}"));
                }
                line
            })
            .collect::<Vec<_>>();
        if lines.is_empty() {
            String::new()
        } else {
            format!("Outgoing edge contracts:\n{}\n\n", lines.join("\n"))
        }
    }

    pub(super) fn workflow_turn_prompt_text(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
        node_id: &str,
        endpoint_prompt: &str,
        handoff_payloads_json: Option<&str>,
        control_mailbox: Option<&str>,
    ) -> Result<String, DaemonError> {
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_id)?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(session_id, workflow_run.workflow_id())?;
        let node = workflow.node(node_id);
        let base_directory =
            self.workflow_runtime_base_directory(session_id, workflow_run_id, workflow_node_run_id);
        let instruction_ref = self.workflow_node_instruction_reference(
            base_directory.as_ref(),
            workflow_run_id,
            node_id,
            node.and_then(|node| node.instructions()),
        );
        let turn_index = workflow_run
            .node_runs()
            .iter()
            .filter(|node_run| node_run.node_id() == node_id)
            .count() as u32;
        Ok(
            crate::scheduler::prompt_injection::build_workflow_turn_prompt(
                crate::scheduler::prompt_injection::WorkflowPromptInjectionContext {
                    workflow_ref: Some(workflow.id().to_string()),
                    endpoint_prompt: endpoint_prompt.to_string(),
                    workflow_prompt: workflow_run
                        .invocation_prompt()
                        .map(str::to_string)
                        .unwrap_or_default(),
                    node_id: Some(node_id.to_string()),
                    node_instructions: node
                        .and_then(|node| node.instructions())
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .unwrap_or("No node-specific instructions were configured.")
                        .to_string(),
                    instruction_ref,
                    handoff_payloads_json: handoff_payloads_json.map(str::to_string),
                    outgoing_edge_contracts: self.workflow_outgoing_edge_contracts_text(
                        session_id,
                        workflow_run_id,
                        node_id,
                    ),
                    control_mailbox: control_mailbox.map(str::to_string),
                    delivery_token: format!("workflow-ack:{workflow_node_run_id}"),
                    node_turn: node.map(|node| {
                        crate::scheduler::prompt_injection::WorkflowNodeTurnPromptContext {
                            turn_index,
                            max_turns: node.max_turns(),
                            can_complete_workflow_run: node.can_complete_workflow_run(),
                            can_emit_intermediate_output: node.can_emit_intermediate_run_output(),
                            wait_for_all_inputs: node.wait_for_all_inputs(),
                        }
                    }),
                    base_directory,
                    hide_in_native_tui: self.workflow_node_uses_native_tui(
                        session_id,
                        workflow_run_id,
                        workflow_node_run_id,
                    ),
                },
            ),
        )
    }

    fn workflow_node_uses_native_tui(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> bool {
        self.session_store
            .get_session(session_id)
            .ok()
            .and_then(|session| session.workflow_run(workflow_run_id).cloned())
            .and_then(|workflow_run| {
                workflow_run
                    .node_runs()
                    .iter()
                    .find(|candidate| candidate.id() == workflow_node_run_id)
                    .map(|node_run| node_run.agent_id().to_string())
            })
            .and_then(|agent_id| {
                self.provider_store
                    .get_latest_run_for_agent(session_id, &agent_id)
            })
            .is_some_and(|run| !run.client_interface().is_arroba())
    }

    pub(super) fn workflow_runtime_base_directory(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> Option<PathBuf> {
        let session = self.session_store.get_session(session_id).ok()?;
        let workflow_run = session.workflow_run(workflow_run_id)?;
        let node_run = workflow_run
            .node_runs()
            .iter()
            .find(|candidate| candidate.id() == workflow_node_run_id)?;
        self.provider_store
            .get_latest_run_for_agent(session_id, node_run.agent_id())
            .and_then(|run| run.working_directory().cloned())
            .or_else(|| {
                let worktree = PathBuf::from(session.worktree_id());
                if worktree.is_absolute() {
                    Some(worktree)
                } else {
                    std::env::current_dir().ok().map(|cwd| cwd.join(worktree))
                }
            })
    }

    pub(super) fn workflow_node_instruction_reference(
        &self,
        base_directory: Option<&PathBuf>,
        workflow_run_id: &str,
        node_id: &str,
        node_instructions: Option<&str>,
    ) -> Option<String> {
        let root = base_directory?
            .join(".arroba")
            .join("workflow-runtime")
            .join("kernel")
            .join(workflow_run_id)
            .join("workflow-instructions");
        let path = root.join(format!("node-{node_id}.md"));
        if !path.exists() || node_instructions.is_some() {
            if let Err(error) = std::fs::create_dir_all(&root) {
                tracing::debug!(
                    ?error,
                    "Failed to create workflow instruction directory at {:?}",
                    root
                );
                return None;
            }
            let content = node_instructions.map(str::to_string).unwrap_or_else(|| {
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
}
