//! Workflow scheduling and node-dispatch state transitions.
//!
//! This module advances queued/running workflow nodes, applies retry policy, records completion,
//! and prepares provider prompts for executable workflow nodes.

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
                let mut line = format!("- edge {} -> {}", edge.id(), edge.to_node_id());
                if let Some(schema_ref) = edge.output_schema_ref() {
                    line.push_str(&format!(", output_schema_ref: {schema_ref}"));
                }
                if let Some(validation_policy) = edge.validation_policy() {
                    line.push_str(&format!(", validation_policy: {validation_policy:?}"));
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

    #[allow(dead_code)]
    pub(super) fn workflow_prepare_dispatches(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        dispatches: &[crate::session::WorkflowDispatch],
    ) -> WorkflowPromptDispatches {
        let mut prepared = WorkflowPromptDispatches::default();
        for dispatch in dispatches {
            if !self.workflow_dispatch_has_all_inputs(session_id, workflow_run_id, &dispatch) {
                continue;
            }
            self.record_notice(
                session_id,
                None,
                self.attachment_store
                    .list_session_attachment_ids(session_id),
                format!(
                    "Workflow run `{workflow_run_id}` routed {} upstream message(s) to node `{}`.",
                    dispatch.messages.len(),
                    dispatch.node_run.node_id()
                ),
            );
            let handoff_payloads_json =
                serde_json::to_string(&dispatch.messages).unwrap_or_else(|_| "[]".to_string());
            let control_mailbox = self.workflow_control_mailbox_text(
                session_id,
                workflow_run_id,
                dispatch.node_run.id(),
            );
            let prompt_text = match self.workflow_turn_prompt_text(
                session_id,
                workflow_run_id,
                dispatch.node_run.id(),
                dispatch.node_run.node_id(),
                "",
                Some(&handoff_payloads_json),
                control_mailbox.as_deref(),
            ) {
                Ok(prompt_text) => prompt_text,
                Err(error) => {
                    self.record_notice(
                        session_id,
                        None,
                        self.attachment_store.list_session_attachment_ids(session_id),
                        format!(
                            "Workflow run `{workflow_run_id}` could not prepare downstream node `{}`: {}",
                            dispatch.node_run.node_id(),
                            error
                        ),
                    );
                    continue;
                }
            };
            let _ = self.session_store.write().prepare_workflow_turn(
                session_id,
                workflow_run_id,
                dispatch.node_run.id(),
                format!("workflow-ack:{}", dispatch.node_run.id()),
                prompt_text.clone(),
                control_mailbox,
                Some(handoff_payloads_json),
            );
            let claim_id = match self
                .workflow_dispatch_claim_id(session_id, dispatch.node_run.agent_id())
            {
                Ok(claim_id) => claim_id,
                Err(error) => {
                    self.record_notice(
                            session_id,
                            None,
                            self.attachment_store.list_session_attachment_ids(session_id),
                            format!(
                                "Workflow run `{workflow_run_id}` could not schedule downstream node `{}`: {}",
                                dispatch.node_run.node_id(),
                                error
                            ),
                        );
                    continue;
                }
            };
            match self.acquire_workflow_node_workspace_claim(
                session_id,
                &claim_id,
                dispatch.node_run.agent_id(),
                workflow_run_id,
                dispatch.node_run.id(),
            ) {
                Ok(()) => {
                    let _ = self
                        .session_store
                        .write()
                        .ready_workflow_node_after_workspace_claim(
                            session_id,
                            workflow_run_id,
                            dispatch.node_run.id(),
                        );
                }
                Err(error @ DaemonError::WorkspaceClaimConflict { .. }) => {
                    let _ = self
                        .session_store
                        .write()
                        .block_workflow_node_on_workspace_claim(
                            session_id,
                            workflow_run_id,
                            dispatch.node_run.id(),
                        );
                    self.record_notice(
                        session_id,
                        None,
                        self.attachment_store.list_session_attachment_ids(session_id),
                        format!(
                            "Workflow run `{workflow_run_id}` blocked node `{}` on a workspace claim: {error}",
                            dispatch.node_run.node_id()
                        ),
                    );
                    continue;
                }
                Err(error) => {
                    self.record_notice(
                        session_id,
                        None,
                        self.attachment_store.list_session_attachment_ids(session_id),
                        format!(
                            "Workflow run `{workflow_run_id}` could not schedule downstream node `{}`: {}",
                            dispatch.node_run.node_id(),
                            error
                        ),
                    );
                    continue;
                }
            }
            let prompt = crate::session::PromptQueueItem::new(
                self.session_store.reserve_prompt_id(),
                crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run_id),
                dispatch.node_run.agent_id(),
                prompt_text,
                crate::session::PromptStatus::Queued,
            )
            .with_workflow_context(workflow_run_id, dispatch.node_run.id());
            match self.workflow_submit_prepared_prompt(
                crate::app::KernelPreparedPromptSubmission {
                    session_id: session_id.to_string(),
                    prompt,
                    force_queue: false,
                },
                workflow_run_id,
                dispatch.node_run.id(),
            ) {
                Ok(dispatches) => prepared.extend(dispatches),
                Err(error) => {
                    self.record_notice(
                        session_id,
                        None,
                        self.attachment_store.list_session_attachment_ids(session_id),
                        format!(
                            "Workflow run `{workflow_run_id}` could not schedule downstream node `{}`: {}",
                            dispatch.node_run.node_id(),
                            error
                        ),
                    );
                }
            }
        }
        prepared
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
                    endpoint_prompt: endpoint_prompt.to_string(),
                    workflow_prompt: workflow_run
                        .invocation_prompt()
                        .map(str::to_string)
                        .unwrap_or_default(),
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
            let content = node_instructions
                .map(str::to_string)
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

    pub(super) fn workflow_retry_blocked_claims(&self) -> WorkflowPromptDispatches {
        let mut blocked = Vec::new();
        for session in self.session_store.read().list_sessions() {
            for workflow_run in session.workflow_runs() {
                for node_run in workflow_run.node_runs() {
                    if node_run.status()
                        != crate::session::WorkflowNodeRunStatus::BlockedOnWorkspaceClaim
                    {
                        continue;
                    }
                    let Some(prompt) = node_run
                        .turn_envelope()
                        .and_then(|envelope| envelope.rendered_prompt())
                        .map(str::to_string)
                    else {
                        continue;
                    };
                    blocked.push((
                        session.id().to_string(),
                        workflow_run.id().to_string(),
                        node_run.id().to_string(),
                        node_run.agent_id().to_string(),
                        node_run.node_id().to_string(),
                        prompt,
                    ));
                }
            }
        }
        let mut dispatches = WorkflowPromptDispatches::default();
        for (session_id, workflow_run_id, workflow_node_run_id, agent_id, node_id, prompt_text) in
            blocked
        {
            let claim_id = match self.workflow_dispatch_claim_id(&session_id, &agent_id) {
                Ok(claim_id) => claim_id,
                Err(error) => {
                    self.record_notice(
                        &session_id,
                        None,
                        self.attachment_store.list_session_attachment_ids(&session_id),
                        format!(
                            "Workflow run `{workflow_run_id}` could not retry blocked node `{node_id}`: {error}"
                        ),
                    );
                    continue;
                }
            };
            match self.acquire_workflow_node_workspace_claim(
                &session_id,
                &claim_id,
                &agent_id,
                &workflow_run_id,
                &workflow_node_run_id,
            ) {
                Ok(()) => {
                    let _ = self
                        .session_store
                        .write()
                        .ready_workflow_node_after_workspace_claim(
                            &session_id,
                            &workflow_run_id,
                            &workflow_node_run_id,
                        );
                }
                Err(DaemonError::WorkspaceClaimConflict { .. }) => continue,
                Err(error) => {
                    self.record_notice(
                        &session_id,
                        None,
                        self.attachment_store.list_session_attachment_ids(&session_id),
                        format!(
                            "Workflow run `{workflow_run_id}` could not retry blocked node `{node_id}`: {error}"
                        ),
                    );
                    continue;
                }
            }
            let prompt = crate::session::PromptQueueItem::new(
                self.session_store.reserve_prompt_id(),
                crate::scheduler::runtime::workflow_prompt_source_attachment_id(&workflow_run_id),
                agent_id,
                prompt_text,
                crate::session::PromptStatus::Queued,
            )
            .with_workflow_context(&workflow_run_id, &workflow_node_run_id);
            match self.workflow_submit_prepared_prompt(
                crate::app::KernelPreparedPromptSubmission {
                    session_id: session_id.clone(),
                    prompt,
                    force_queue: false,
                },
                &workflow_run_id,
                &workflow_node_run_id,
            ) {
                Ok(prepared) => dispatches.extend(prepared),
                Err(error) => {
                    self.record_notice(
                        &session_id,
                        None,
                        self.attachment_store.list_session_attachment_ids(&session_id),
                        format!(
                            "Workflow run `{workflow_run_id}` could not retry blocked node `{node_id}`: {error}"
                        ),
                    );
                }
            }
        }
        dispatches
    }

    pub(super) fn workflow_dispatch_has_all_inputs(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        dispatch: &crate::session::WorkflowDispatch,
    ) -> bool {
        let workflow_id = match self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_id)
        {
            Ok(run) => run.workflow_id().to_string(),
            Err(_) => return true,
        };
        let workflow = match self
            .session_store
            .read()
            .resolve_workflow_ref(session_id, &workflow_id)
        {
            Ok(workflow) => workflow,
            Err(_) => return true,
        };
        let expected = workflow
            .edges()
            .iter()
            .filter(|edge| edge.to_node_id() == dispatch.node_run.node_id())
            .map(|edge| edge.from_node_id().to_string())
            .collect::<std::collections::BTreeSet<_>>();
        if expected.len() <= 1 {
            return true;
        }
        let run = match self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_id)
        {
            Ok(run) => run,
            Err(_) => return true,
        };
        let run_node_by_id = run
            .node_runs()
            .iter()
            .map(|node_run| (node_run.id().to_string(), node_run.node_id().to_string()))
            .collect::<std::collections::BTreeMap<_, _>>();
        let delivered = dispatch
            .messages
            .iter()
            .filter_map(|message| message.source_node_run_id())
            .filter_map(|node_run_id| run_node_by_id.get(node_run_id).cloned())
            .collect::<std::collections::BTreeSet<_>>();
        expected.is_subset(&delivered)
    }
}
