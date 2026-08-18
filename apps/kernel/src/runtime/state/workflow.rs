//! Core workflow prompt setup and validation helpers.
//!
//! This module contains shared workflow mechanics used by admin, dispatch, and tool-facing
//! workflow code: context construction, node validation, and prompt-start preparation.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn workflow_start_prompt(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<(), DaemonError> {
        let (Some(workflow_run_id), Some(workflow_node_run_id)) =
            (prompt.workflow_run_id(), prompt.workflow_node_run_id())
        else {
            return Ok(());
        };
        let workflow_run = self.session_store.write().start_workflow_node_run(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        )?;
        let recipients = self
            .attachment_store
            .list_session_attachment_ids(session_id);
        let active_provider_run_id = self
            .session_store
            .get_session(session_id)?
            .active_provider_run_id()
            .map(str::to_string);
        self.record_notice(
            session_id,
            active_provider_run_id.as_deref(),
            recipients,
            format!(
                "Workflow run `{}` started on agent `{}`.",
                workflow_run.id(),
                prompt.target_agent_id()
            ),
        );
        // The workflow node state is part of the restart contract.  The owned prompt
        // dispatcher calls this path for both ordinary and recovered prompts, so merely
        // updating the in-memory session/projection is insufficient: after a restart the
        // provider prompt can be active while its workflow node is still persisted as Ready.
        // Persist the Running transition before the provider can call workflow runtime tools.
        self.persist_workflow_runtime_session(session_id, "workflow_prompt_started")?;
        Ok(())
    }

    pub(super) fn workflow_ensure_provider_run(
        &self,
        session_id: &str,
        agent_id: &str,
        event_reply_enabled: bool,
    ) -> Result<String, DaemonError> {
        let existing_run = self.provider_store.get_run_for_agent(session_id, agent_id);
        if let Some(run) = existing_run.as_ref() {
            if run.workflow_tools_enabled()
                && run.workflow_event_reply_enabled() == event_reply_enabled
            {
                if run.state() == crate::provider::ProviderRunState::Parked {
                    let resumed = self.resume_provider_run_for_session(session_id, run.id())?;
                    self.session_store
                        .set_active_provider_run(session_id, Some(resumed.id().to_string()))?;
                    return Ok(resumed.id().to_string());
                }
                if run.state() != crate::provider::ProviderRunState::Ended {
                    self.session_store
                        .set_active_provider_run(session_id, Some(run.id().to_string()))?;
                    return Ok(run.id().to_string());
                }
            } else if matches!(
                run.state(),
                crate::provider::ProviderRunState::Starting
                    | crate::provider::ProviderRunState::Running
            ) && self.provider_run_has_active_prompt(session_id, &run)?
            {
                // An ordinary provider with an active prompt owns the session until that
                // prompt settles. The workflow prompt remains FIFO-queued and the normal
                // settlement path will replace this run before it is promoted.
                self.session_store
                    .set_active_provider_run(session_id, Some(run.id().to_string()))?;
                return Ok(run.id().to_string());
            }
        }
        let agent = self.agent_store.get_agent(agent_id)?;
        let provider = crate::provider::provider_id_for_launch(agent.provider());
        // An existing provider run can use a provider-specific variant on a
        // shared adapter, such as the test-only `slow-structured` dev stub.
        // The agent profile stores the provider id, but the adapter identity
        // belongs to the run that is being rotated. Reuse that adapter when
        // replacing an idle run so workflow admission does not try to resolve
        // a provider variant as a standalone adapter.
        let adapter_key = existing_run
            .as_ref()
            .filter(|run| run.state() != crate::provider::ProviderRunState::Ended)
            .map(|run| run.adapter_key())
            .unwrap_or_else(|| crate::provider::adapter_key_for_provider(provider));
        let mut request = crate::provider::LaunchProviderRequest::new(
            session_id,
            adapter_key,
            provider,
            agent.provider_account_profile(),
            agent.model().unwrap_or("default"),
        )
        .with_agent_id(agent.id().to_string())
        .with_owner_user_id(agent.owner_user_id().to_string())
        .with_variant(agent.effort().map(str::to_string));
        if let Some(working_directory) = existing_run
            .and_then(|run| run.working_directory().cloned())
            .or_else(|| agent.worktree_id().map(std::path::PathBuf::from))
        {
            request = request.with_working_directory(working_directory);
        }
        request = self.prepare_provider_launch_request(
            request,
            self.config_projection.snapshot().runtime_mcp_url(),
        )?;
        request = request.with_workflow_event_reply(event_reply_enabled);
        let started = self.start_provider_launch(request)?;
        // The owned workflow admission path is synchronous, so it cannot use the async app
        // launch helper. Enable the workflow tool surface before the detached launch is spawned;
        // otherwise an idle ordinary provider run can be reused and the workflow prompt is
        // dispatched without the workflow tools it needs.
        let run = self
            .provider_store
            .enable_workflow_tools(started.run.id())?;
        self.provider_run_projection.update(run.clone());
        Ok(run.id().to_string())
    }

    pub(super) fn workflow_event_reply_enabled_for_prompt(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<bool, DaemonError> {
        let Some(workflow_run_id) = prompt.workflow_run_id() else {
            return Ok(false);
        };
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_id)?;
        let Some(invocation) = workflow_run.publication_invocation() else {
            return Ok(false);
        };
        if invocation.transport != "event" {
            return Ok(false);
        }
        let Some(binding_id) = invocation.hook_id.as_deref() else {
            return Ok(false);
        };
        Ok(self
            .session_store
            .read()
            .get_session(session_id)?
            .workflow_event_binding(binding_id)
            .is_some_and(|binding| {
                matches!(binding.reply_mode.as_deref(), Some("thread" | "channel"))
            }))
    }

    pub(super) fn workflow_dispatch_claim_id(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> String {
        format!("workflow-node:{session_id}:{workflow_run_id}:{workflow_node_run_id}")
    }

    /// Promote a workflow prompt that was already admitted to the provider queue.
    ///
    /// Queue promotion can happen outside the normal workflow launch path (for
    /// example after a provider cancellation). Keep the workflow run transition
    /// coupled to that promotion so a Ready node cannot strand the session with
    /// a provider prompt that is already Dispatching.
    pub(super) fn workflow_mark_prompt_started(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<(), DaemonError> {
        let (Some(workflow_run_id), Some(workflow_node_run_id)) =
            (prompt.workflow_run_id(), prompt.workflow_node_run_id())
        else {
            return Ok(());
        };
        self.session_store.write().mark_workflow_turn_dispatched(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        )?;
        self.workflow_start_prompt(session_id, prompt)
    }

    pub(super) fn workflow_submit_prepared_prompt(
        &self,
        prepared: crate::app::KernelPreparedPromptSubmission,
        _workflow_run_id: &str,
        _workflow_node_run_id: &str,
    ) -> Result<WorkflowPromptDispatches, DaemonError> {
        let prepared = normalize_workflow_prepared_prompt(prepared);
        let mut dispatches = WorkflowPromptDispatches::default();
        let target_agent = self
            .agent_store
            .get_agent(prepared.prompt.target_agent_id())?;
        let workflow_provider_run_id = if target_agent.remote_execution().is_none() {
            let event_reply_enabled = self
                .workflow_event_reply_enabled_for_prompt(&prepared.session_id, &prepared.prompt)?;
            Some(self.workflow_ensure_provider_run(
                &prepared.session_id,
                prepared.prompt.target_agent_id(),
                event_reply_enabled,
            )?)
        } else {
            None
        };
        let mut submission = match self.submit_local_prepared_prompt_for_provider_run(
            &prepared,
            workflow_provider_run_id.as_deref(),
        )? {
            Some(submission) => submission,
            None => match self.submit_remote_prepared_prompt(&prepared)? {
                Some(submission) => submission,
                None => return Ok(dispatches),
            },
        };
        dispatches.mark_workflow_prompt_admitted();
        if let crate::session::PromptSubmissionOutcome::Started { prompt } = &submission.outcome {
            self.workflow_mark_prompt_started(&prepared.session_id, prompt)?;
        }
        if let Some(dispatch) = submission.dispatch.take() {
            dispatches.local.push(dispatch);
        }
        if let Some(mut dispatch) = submission.remote_dispatch.take() {
            let prompt = match &submission.outcome {
                crate::session::PromptSubmissionOutcome::Started { prompt }
                | crate::session::PromptSubmissionOutcome::Queued { prompt } => prompt,
            };
            dispatch.prompt = prompt.prompt().to_string();
            if dispatch.workflow_context.is_none() {
                dispatch.workflow_context = Some(self.remote_workflow_turn_context_for_prompt(
                    &prepared.session_id,
                    prompt.target_agent_id(),
                    prompt,
                )?);
            }
            dispatches.remote.push(dispatch);
        }
        if matches!(
            submission.outcome,
            crate::session::PromptSubmissionOutcome::Queued { .. }
        ) {
            if let Some(run) = workflow_provider_run_id
                .as_deref()
                .and_then(|provider_run_id| self.provider_store.get_run(provider_run_id).ok())
            {
                if run.state() == crate::provider::ProviderRunState::Starting {
                    dispatches.starting_provider_runs.push(run.id().to_string());
                }
            }
        }
        Ok(dispatches)
    }

    pub(super) fn remote_workflow_turn_context_for_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<crate::execution_lease::RemoteWorkflowTurnContext, DaemonError> {
        let workflow_run_id =
            prompt
                .workflow_run_id()
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "dispatch remote workflow prompt",
                    message: "remote workflow prompt is missing workflow run id".to_string(),
                })?;
        let workflow_node_run_id =
            prompt
                .workflow_node_run_id()
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "dispatch remote workflow prompt",
                    message: "remote workflow prompt is missing workflow node run id".to_string(),
                })?;
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_id)?;
        let delivery_token = workflow_run
            .node_runs()
            .iter()
            .find(|node_run| node_run.id() == workflow_node_run_id)
            .and_then(|node_run| node_run.turn_envelope())
            .map(|envelope| envelope.delivery_token().to_string())
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "dispatch remote workflow prompt",
                message: format!(
                    "workflow node run `{workflow_node_run_id}` has no prepared turn envelope"
                ),
            })?;
        let event_reply_enabled =
            self.workflow_event_reply_enabled_for_prompt(session_id, prompt)?;
        Ok(crate::execution_lease::RemoteWorkflowTurnContext {
            home_kernel_id: self.config_projection.snapshot().daemon_id,
            home_session_id: session_id.to_string(),
            home_agent_id: target_agent_id.to_string(),
            workflow_run_id: workflow_run.id().to_string(),
            workflow_node_run_id: workflow_node_run_id.to_string(),
            delivery_token,
            event_reply_enabled,
        })
    }

    pub(super) fn workflow_validate_agents(
        &self,
        session_id: &str,
        workflow: &crate::session::WorkflowDefinition,
    ) -> Result<(), DaemonError> {
        let agents = self.agent_store.get_session_agents(session_id);
        let agent_ids = agents
            .iter()
            .map(|agent| agent.id().to_string())
            .collect::<std::collections::BTreeSet<_>>();
        for node in workflow.nodes() {
            if !agent_ids.contains(node.agent_id()) {
                return Err(DaemonError::WorkflowNodeAgentMissing {
                    session_id: session_id.to_string(),
                    workflow_id: workflow.id().to_string(),
                    node_id: node.id().to_string(),
                    agent_id: node.agent_id().to_string(),
                });
            }
            let Some(agent) = agents.iter().find(|agent| agent.id() == node.agent_id()) else {
                continue;
            };
            let capabilities = self
                .provider_store
                .get_run_for_agent(session_id, node.agent_id())
                .unwrap_or_else(|| {
                    let adapter_key = crate::provider::adapter_key_for_provider(agent.provider());
                    crate::provider::RuntimeProviderRun::from_control_capability_inference(
                        format!("inferred-{session_id}-{}", node.agent_id()),
                        session_id.to_string(),
                        Some(node.agent_id().to_string()),
                        adapter_key.to_string(),
                    )
                });
            if !capabilities
                .supports_control_operation(crate::provider::ControlOperation::AckWorkflowTurn)
            {
                return Err(DaemonError::WorkflowNodeControlUnsupported {
                    session_id: session_id.to_string(),
                    workflow_id: workflow.id().to_string(),
                    node_id: node.id().to_string(),
                    agent_id: node.agent_id().to_string(),
                    operation: "ack_workflow_turn",
                });
            }
            let requires_validation = workflow.edges().iter().any(|edge| {
                edge.from_node_id() == node.id() && edge.handoff_schema_ref().is_some()
            });
            if requires_validation
                && !capabilities.supports_control_operation(
                    crate::provider::ControlOperation::ValidateWorkflowHandoff,
                )
            {
                return Err(DaemonError::WorkflowNodeControlUnsupported {
                    session_id: session_id.to_string(),
                    workflow_id: workflow.id().to_string(),
                    node_id: node.id().to_string(),
                    agent_id: node.agent_id().to_string(),
                    operation: "validate_workflow_handoff",
                });
            }
        }
        Ok(())
    }
}

fn normalize_workflow_prepared_prompt(
    prepared: crate::app::KernelPreparedPromptSubmission,
) -> crate::app::KernelPreparedPromptSubmission {
    let (visible_prompt, extracted_hidden_context) =
        crate::prompt_transcript::split_workflow_prompt_for_hidden_context(
            prepared.prompt.prompt().to_string(),
        );
    let visible_prompt = crate::prompt_transcript::workflow_visible_prompt_text(&visible_prompt);
    if extracted_hidden_context.is_empty() {
        return crate::app::KernelPreparedPromptSubmission {
            prompt: prepared.prompt.with_prompt_text(visible_prompt),
            ..prepared
        };
    }
    let hidden_system_context = join_workflow_prompt_context(
        prepared.prompt.hidden_system_context(),
        &extracted_hidden_context,
    );
    crate::app::KernelPreparedPromptSubmission {
        prompt: prepared
            .prompt
            .with_prompt_text(visible_prompt)
            .with_hidden_system_context(hidden_system_context),
        ..prepared
    }
}

fn join_workflow_prompt_context(first: &str, second: &str) -> String {
    match (first.trim(), second.trim()) {
        ("", "") => String::new(),
        (first, "") => first.to_string(),
        ("", second) => second.to_string(),
        (first, second) => format!("{first}\n\n{second}"),
    }
}

#[cfg(test)]
mod workflow_prepared_prompt_tests {
    use super::normalize_workflow_prepared_prompt;
    use crate::session::{PromptQueueItem, PromptStatus};

    #[test]
    fn workflow_admission_separates_visible_prompt_from_private_context() {
        let prepared = crate::app::KernelPreparedPromptSubmission {
            session_id: "session-1".to_string(),
            prompt: PromptQueueItem::new(
                "pending-1",
                "workflow-run:run-1",
                "agent-1",
                "<endpoint-prompt>\nvisible\n</endpoint-prompt>\n\n\
                 <node-level-prompt>\nhidden\n</node-level-prompt>",
                PromptStatus::Queued,
            )
            .with_workflow_context("run-1", "node-run-1"),
            force_queue: false,
            refresh_projection: true,
        };

        let normalized = normalize_workflow_prepared_prompt(prepared);

        assert_eq!(normalized.prompt.prompt().trim(), "visible");
        assert_eq!(
            normalized.prompt.hidden_system_context(),
            "<node-level-prompt>\nhidden\n</node-level-prompt>"
        );
    }
}
