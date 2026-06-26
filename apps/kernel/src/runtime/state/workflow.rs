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
        let _ = self.session_snapshot(session_id)?;
        Ok(())
    }

    pub(super) fn workflow_ensure_provider_run(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<String, DaemonError> {
        if let Some(run) = self.provider_store.get_run_for_agent(session_id, agent_id) {
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
        }
        let agent = self.agent_store.get_agent(agent_id)?;
        let provider = match agent.provider() {
            "default" => "opencode",
            value => value,
        };
        let adapter_key = crate::provider::adapter_key_for_provider(provider);
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
        if let Some(worktree_id) = agent.worktree_id() {
            request = request.with_working_directory(std::path::PathBuf::from(worktree_id));
        }
        request = self.prepare_provider_launch_request(
            request,
            self.config_projection.snapshot().runtime_mcp_url(),
        )?;
        let started = self.start_provider_launch(request)?;
        let run = started.run;
        self.provider_run_projection.update(run.clone());
        Ok(run.id().to_string())
    }

    pub(super) fn workflow_dispatch_claim_id(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<String, DaemonError> {
        let agent = self.agent_store.get_agent(agent_id)?;
        if let Some(remote_execution) = agent.remote_execution() {
            return Ok(format!(
                "remote-workflow:{}:{}",
                remote_execution.worker_kernel_id, remote_execution.leased_agent_id
            ));
        }
        self.workflow_ensure_provider_run(session_id, agent_id)
    }

    pub(super) fn workflow_submit_prepared_prompt(
        &self,
        prepared: crate::app::KernelPreparedPromptSubmission,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> Result<WorkflowPromptDispatches, DaemonError> {
        let mut dispatches = WorkflowPromptDispatches::default();
        let mut submission = match self.submit_local_prepared_prompt(&prepared)? {
            Some(submission) => submission,
            None => match self.submit_remote_prepared_prompt(&prepared)? {
                Some(submission) => submission,
                None => return Ok(dispatches),
            },
        };
        if let crate::session::PromptSubmissionOutcome::Started { prompt } = &submission.outcome {
            let _ = self.session_store.write().mark_workflow_turn_dispatched(
                &prepared.session_id,
                workflow_run_id,
                workflow_node_run_id,
            );
            let _ = self.workflow_start_prompt(&prepared.session_id, prompt);
        }
        if let Some(dispatch) = submission.dispatch.take() {
            dispatches.local.push(dispatch);
        }
        if let Some(mut dispatch) = submission.remote_dispatch.take() {
            if dispatch.workflow_context.is_none() {
                let prompt = match &submission.outcome {
                    crate::session::PromptSubmissionOutcome::Started { prompt }
                    | crate::session::PromptSubmissionOutcome::Queued { prompt } => prompt,
                };
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
            if let Some(run) = self
                .provider_store
                .get_run_for_agent(&prepared.session_id, prepared.prompt.target_agent_id())
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
        Ok(crate::execution_lease::RemoteWorkflowTurnContext {
            home_kernel_id: self.config_projection.snapshot().daemon_id,
            home_session_id: session_id.to_string(),
            home_agent_id: target_agent_id.to_string(),
            workflow_run_id: workflow_run.id().to_string(),
            workflow_node_run_id: workflow_node_run_id.to_string(),
            delivery_token,
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
