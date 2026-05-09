//! Workflow scheduling and node-dispatch state transitions.
//!
//! This module advances queued/running workflow nodes, applies retry policy, records completion,
//! and prepares provider prompts for executable workflow nodes.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn workflow_schedule_entry_node(
        &self,
        session_id: &str,
        workflow_run: &crate::session::WorkflowRun,
    ) -> Result<WorkflowPromptDispatches, DaemonError> {
        let endpoint_prompt = workflow_run
            .invocation_prompt()
            .map(str::trim)
            .unwrap_or("");
        let node_run = workflow_run.node_runs().first().ok_or_else(|| {
            DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_run.workflow_id().to_string(),
                reference: workflow_run.id().to_string(),
                message: "workflow run has no entry node run",
            }
        })?;
        let prompt_text = self.workflow_turn_prompt_text(
            session_id,
            workflow_run.id(),
            node_run.id(),
            node_run.node_id(),
            endpoint_prompt,
            None,
            None,
        )?;
        let _ = self.session_store.write().prepare_workflow_turn(
            session_id,
            workflow_run.id(),
            node_run.id(),
            format!("workflow-ack:{}", node_run.id()),
            prompt_text.clone(),
            None,
            None,
        )?;
        let claim_id = self.workflow_dispatch_claim_id(session_id, node_run.agent_id())?;
        match self.acquire_workflow_node_workspace_claim(
            session_id,
            &claim_id,
            node_run.agent_id(),
            workflow_run.id(),
            node_run.id(),
        ) {
            Ok(()) => {}
            Err(error @ DaemonError::WorkspaceClaimConflict { .. }) => {
                let _ = self
                    .session_store
                    .write()
                    .block_workflow_node_on_workspace_claim(
                        session_id,
                        workflow_run.id(),
                        node_run.id(),
                    );
                self.record_notice(
                    session_id,
                    None,
                    self.attachment_store
                        .list_session_attachment_ids(session_id),
                    format!(
                        "Workflow run `{}` blocked node `{}` on a workspace claim: {error}",
                        workflow_run.id(),
                        node_run.node_id()
                    ),
                );
                let _ = self.session_snapshot(session_id)?;
                return Ok(WorkflowPromptDispatches::default());
            }
            Err(error) => return Err(error),
        }
        let _ = self
            .session_store
            .write()
            .ready_workflow_node_after_workspace_claim(
                session_id,
                workflow_run.id(),
                node_run.id(),
            );
        let prompt = crate::session::PromptQueueItem::new(
            self.session_store.reserve_prompt_id(),
            crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
            node_run.agent_id(),
            prompt_text,
            crate::session::PromptStatus::Queued,
        )
        .with_workflow_context(workflow_run.id(), node_run.id());
        self.workflow_submit_prepared_prompt(
            crate::app::KernelPreparedPromptSubmission {
                session_id: session_id.to_string(),
                prompt,
                force_queue: false,
            },
            workflow_run.id(),
            node_run.id(),
        )
    }

    pub(super) fn workflow_invoke_queued_launch(
        &self,
        session_id: &str,
        queued_launch: crate::session::QueuedWorkflowLaunch,
    ) -> Result<
        (
            crate::app::workflow_runtime::WorkflowLaunchOutcome,
            WorkflowPromptDispatches,
        ),
        DaemonError,
    > {
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(session_id, queued_launch.workflow_id())?;
        let endpoint = self.session_store.read().resolve_workflow_endpoint_ref(
            session_id,
            queued_launch.workflow_id(),
            queued_launch.endpoint_id(),
        )?;
        self.workflow_validate_agents(session_id, &workflow)?;
        self.workflow_flush_agent_context_if_needed(session_id, &workflow)?;
        let workflow_run = self.session_store.write().invoke_workflow_endpoint(
            session_id,
            workflow.id(),
            endpoint.id(),
            queued_launch.invocation_prompt().map(str::to_string),
        )?;
        let dispatches = self.workflow_schedule_entry_node(session_id, &workflow_run)?;
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run.id())?;
        if let Some(watchdog_id) = queued_launch.watchdog_id() {
            let _ = self.session_store.write().mark_workflow_watchdog_invoked(
                session_id,
                watchdog_id,
                workflow_run.id(),
            );
        }
        Ok((
            crate::app::workflow_runtime::WorkflowLaunchOutcome::Started {
                workflow_run,
                workflow,
                endpoint,
            },
            dispatches,
        ))
    }

    pub(super) fn workflow_invoke_endpoint_with_admission(
        &self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        prompt: Option<String>,
    ) -> Result<
        (
            crate::app::workflow_runtime::WorkflowLaunchOutcome,
            WorkflowPromptDispatches,
        ),
        DaemonError,
    > {
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(session_id, workflow_ref)?;
        let endpoint = self.session_store.read().resolve_workflow_endpoint_ref(
            session_id,
            workflow_ref,
            endpoint_ref,
        )?;
        self.workflow_validate_agents(session_id, &workflow)?;
        let admission = self.session_store.write().admit_manual_workflow_launch(
            session_id,
            workflow.id(),
            endpoint.id(),
            prompt.clone(),
        )?;
        match admission {
            crate::session::WorkflowLaunchAdmission::StartNow => {
                self.workflow_flush_agent_context_if_needed(session_id, &workflow)?;
                let workflow_run = self.session_store.write().invoke_workflow_endpoint(
                    session_id,
                    workflow.id(),
                    endpoint.id(),
                    prompt,
                )?;
                let dispatches = self.workflow_schedule_entry_node(session_id, &workflow_run)?;
                let workflow_run = self
                    .session_store
                    .read()
                    .resolve_workflow_run_ref(session_id, workflow_run.id())?;
                Ok((
                    crate::app::workflow_runtime::WorkflowLaunchOutcome::Started {
                        workflow_run,
                        workflow,
                        endpoint,
                    },
                    dispatches,
                ))
            }
            crate::session::WorkflowLaunchAdmission::Queued(queued_launch) => Ok((
                crate::app::workflow_runtime::WorkflowLaunchOutcome::Queued {
                    queued_launch,
                    workflow,
                    endpoint,
                },
                WorkflowPromptDispatches::default(),
            )),
        }
    }

    pub(super) fn workflow_cancel_prompt(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<(), DaemonError> {
        let (Some(workflow_run_id), Some(workflow_node_run_id)) =
            (prompt.workflow_run_id(), prompt.workflow_node_run_id())
        else {
            return Ok(());
        };
        let workflow_run = self.session_store.write().stop_workflow_node_run(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        )?;
        let _ = self.release_workflow_node_workspace_claim(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        );
        self.workflow_record_failure(
            session_id,
            workflow_run_id,
            &crate::session::WorkflowFailureEvent::new(
                crate::session::WorkflowFailureKind::RunStopped,
                workflow_node_run_id,
                Vec::new(),
                "workflow node run was stopped before validated completion",
            ),
        );
        self.record_notice(
            session_id,
            None,
            self.attachment_store
                .list_session_attachment_ids(session_id),
            format!("Workflow run `{}` was stopped.", workflow_run.id()),
        );
        self.workflow_maybe_start_next_queued_launch(session_id);
        let _ = self.session_snapshot(session_id)?;
        Ok(())
    }

    pub(super) fn workflow_fail_provider_prompt(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
        provider_run_id: Option<&str>,
        message: &str,
    ) -> Result<(), DaemonError> {
        let (Some(workflow_run_id), Some(workflow_node_run_id)) =
            (prompt.workflow_run_id(), prompt.workflow_node_run_id())
        else {
            return Ok(());
        };
        self.workflow_record_failure(
            session_id,
            workflow_run_id,
            &crate::session::WorkflowFailureEvent::new(
                crate::session::WorkflowFailureKind::ProviderFailure,
                workflow_node_run_id,
                Vec::new(),
                message,
            ),
        );
        let workflow_run = self.session_store.write().fail_workflow_node_run(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        )?;
        let _ = self.release_workflow_node_workspace_claim(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        );
        self.record_notice(
            session_id,
            provider_run_id,
            self.attachment_store
                .list_session_attachment_ids(session_id),
            format!(
                "Workflow run `{}` failed after provider turn failure: {}",
                workflow_run.id(),
                message
            ),
        );
        self.workflow_maybe_start_next_queued_launch(session_id);
        let _ = self.session_snapshot(session_id)?;
        Ok(())
    }

    fn provider_run_terminal_diagnostic(&self, provider_run_id: &str) -> Option<String> {
        self.provider_store
            .get_run(provider_run_id)
            .ok()
            .and_then(|run| run.terminal_diagnostic().map(str::to_string))
            .filter(|message| !message.trim().is_empty())
    }

    #[allow(dead_code)]
    pub(super) fn workflow_complete_prompt(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
        provider_run_id: Option<&str>,
    ) -> Result<WorkflowPromptDispatches, DaemonError> {
        let (Some(workflow_run_id), Some(workflow_node_run_id)) =
            (prompt.workflow_run_id(), prompt.workflow_node_run_id())
        else {
            return Ok(WorkflowPromptDispatches::default());
        };
        let completion_snapshot = self.workflow_completion_snapshot(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
            provider_run_id,
        );
        let has_valid_pending_final_output = self.workflow_node_run_has_valid_pending_final_output(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        );
        if completion_snapshot.is_none() && !has_valid_pending_final_output {
            let message = "provider completed workflow turn without a validated workflow output";
            let provider_diagnostic =
                provider_run_id.and_then(|run_id| self.provider_run_terminal_diagnostic(run_id));
            let (failure_kind, failure_message, notice_message) = if let Some(diagnostic) =
                provider_diagnostic
            {
                (
                    crate::session::WorkflowFailureKind::ProviderFailure,
                    diagnostic.clone(),
                    format!(
                        "Workflow run `{workflow_run_id}` failed after provider turn failure: {diagnostic}"
                    ),
                )
            } else {
                (
                    crate::session::WorkflowFailureKind::MissingStructuredOutput,
                    message.to_string(),
                    format!("Workflow run `{workflow_run_id}` failed: {message}."),
                )
            };
            self.workflow_record_failure(
                session_id,
                workflow_run_id,
                &crate::session::WorkflowFailureEvent::new(
                    failure_kind,
                    workflow_node_run_id,
                    Vec::new(),
                    failure_message,
                ),
            );
            self.session_store.write().fail_workflow_node_run(
                session_id,
                workflow_run_id,
                workflow_node_run_id,
            )?;
            let _ = self.release_workflow_node_workspace_claim(
                session_id,
                workflow_run_id,
                workflow_node_run_id,
            );
            self.record_notice(
                session_id,
                provider_run_id,
                self.attachment_store
                    .list_session_attachment_ids(session_id),
                notice_message,
            );
            self.workflow_maybe_start_next_queued_launch(session_id);
            let _ = self.session_snapshot(session_id)?;
            return Ok(WorkflowPromptDispatches::default());
        }
        let max_turns = self.workflow_max_turns(session_id);
        let completion_result = self.session_store.write().complete_workflow_node_run(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
            completion_snapshot.clone(),
            max_turns,
        );
        let update = match completion_result {
            Ok(update) => update,
            Err(crate::error::DaemonError::WorkflowOutputValidationFailed {
                edge_id,
                message,
                ..
            }) => {
                self.workflow_record_failure(
                    session_id,
                    workflow_run_id,
                    &crate::session::WorkflowFailureEvent::new(
                        crate::session::WorkflowFailureKind::OutputValidationFailed,
                        workflow_node_run_id,
                        vec![edge_id.clone()],
                        message.clone(),
                    ),
                );
                self.session_store.write().stop_workflow_node_run(
                    session_id,
                    workflow_run_id,
                    workflow_node_run_id,
                )?;
                let _ = self.release_workflow_node_workspace_claim(
                    session_id,
                    workflow_run_id,
                    workflow_node_run_id,
                );
                self.record_notice(
                    session_id,
                    None,
                    self.attachment_store.list_session_attachment_ids(session_id),
                    format!(
                        "Workflow run `{workflow_run_id}` stopped after validation failed on edge `{edge_id}`: {message}"
                    ),
                );
                self.workflow_maybe_start_next_queued_launch(session_id);
                let _ = self.session_snapshot(session_id)?;
                return Ok(WorkflowPromptDispatches::default());
            }
            Err(error) => return Err(error),
        };
        for warning in &update.validation_warnings {
            let failure = crate::session::WorkflowFailureEvent::new(
                crate::session::classify_workflow_failure_kind(
                    &completion_snapshot,
                    &warning.message,
                ),
                workflow_node_run_id,
                vec![warning.edge_id.clone()],
                warning.message.clone(),
            );
            self.workflow_record_failure(session_id, workflow_run_id, &failure);
            self.record_notice(
                session_id,
                None,
                self.attachment_store
                    .list_session_attachment_ids(session_id),
                format!(
                    "Workflow output validation warning on edge `{}`: {}",
                    warning.edge_id, warning.message
                ),
            );
        }
        if update.workflow_run.status() == crate::session::WorkflowRunStatus::Stopped
            && update.workflow_run.final_output().is_none()
            && update.workflow_run.failure_events().iter().all(|event| {
                event.kind() != crate::session::WorkflowFailureKind::NodeTurnBudgetExhausted
            })
        {
            self.workflow_record_failure(
                session_id,
                workflow_run_id,
                &crate::session::WorkflowFailureEvent::new(
                    crate::session::WorkflowFailureKind::NodeTurnBudgetExhausted,
                    workflow_node_run_id,
                    Vec::new(),
                    "workflow run stopped after a node exhausted its turn budget",
                ),
            );
        }
        if update.workflow_run.final_output_valid() == Some(false) {
            self.workflow_record_failure(
                session_id,
                workflow_run_id,
                &crate::session::WorkflowFailureEvent::new(
                    crate::session::WorkflowFailureKind::WorkflowRunOutputValidationFailed,
                    workflow_node_run_id,
                    Vec::new(),
                    update
                        .workflow_run
                        .final_output_warning()
                        .unwrap_or("workflow run output validation failed"),
                ),
            );
        }
        if update.validation_warnings.is_empty() {
            let _ = self
                .session_store
                .write()
                .mark_workflow_turn_validated_completed(
                    session_id,
                    workflow_run_id,
                    workflow_node_run_id,
                )?;
        }
        let claim_provider_run_id = provider_run_id.map(str::to_string).or_else(|| {
            self.provider_store
                .get_run_for_agent(session_id, prompt.target_agent_id())
                .map(|run| run.id().to_string())
        });
        let released_claim = claim_provider_run_id
            .as_deref()
            .map(|provider_run_id| self.clear_prompt_activity(provider_run_id))
            .unwrap_or(false);
        let released_workflow_claim = self.release_workflow_node_workspace_claim(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        );
        let mut dispatches =
            self.workflow_prepare_dispatches(session_id, workflow_run_id, &update.dispatches);
        if released_claim || released_workflow_claim {
            dispatches.extend(self.workflow_retry_blocked_claims());
        }
        let state_suffix = match update.workflow_run.status() {
            crate::session::WorkflowRunStatus::Waiting => "waiting for downstream handoffs",
            crate::session::WorkflowRunStatus::Completing => "is completing",
            crate::session::WorkflowRunStatus::Completed => "completed",
            crate::session::WorkflowRunStatus::Stopped => "stopped",
            _ => "updated",
        };
        self.record_notice(
            session_id,
            None,
            self.attachment_store
                .list_session_attachment_ids(session_id),
            format!(
                "Workflow run `{}` {state_suffix}.",
                update.workflow_run.id()
            ),
        );
        if matches!(
            update.workflow_run.status(),
            crate::session::WorkflowRunStatus::Completed
                | crate::session::WorkflowRunStatus::Failed
                | crate::session::WorkflowRunStatus::Stopped
        ) {
            self.workflow_maybe_start_next_queued_launch(session_id);
        }
        let _ = self.session_snapshot(session_id)?;
        Ok(dispatches)
    }

    #[allow(dead_code)]
    pub(super) fn workflow_completion_snapshot(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
        provider_run_id: Option<&str>,
    ) -> Option<crate::session::WorkflowCompletionSnapshot> {
        let provider_run_id = provider_run_id
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        let session = self.session_store.get_session(session_id).ok()?;
        let history = match self.history_store.load(&session) {
            Ok(history) => history,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.workflow",
                    "failed to load session history for workflow completion snapshot",
                    serde_json::json!({
                        "session_id": session_id,
                        "workflow_run_id": workflow_run_id,
                        "workflow_node_run_id": workflow_node_run_id,
                        "provider_run_id": provider_run_id,
                        "error": error.to_string(),
                    }),
                );
                return None;
            }
        };
        self.history_projection
            .update_entries(session_id, history.clone());
        crate::scheduler::runtime::build_workflow_completion_snapshot_from_history(
            &session,
            history,
            session_id,
            workflow_run_id,
            workflow_node_run_id,
            provider_run_id,
        )
    }

    pub(super) fn workflow_prompt_has_completion_output(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
        provider_run_id: &str,
    ) -> bool {
        if self.workflow_node_run_has_valid_pending_final_output(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        ) {
            return true;
        }
        self.workflow_completion_snapshot(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
            Some(provider_run_id),
        )
        .and_then(|snapshot| snapshot.output().cloned())
        .is_some()
    }

    fn workflow_node_run_has_valid_pending_final_output(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> bool {
        self.session_store
            .get_session(session_id)
            .ok()
            .and_then(|session| {
                session
                    .workflow_run(workflow_run_id)
                    .and_then(|workflow_run| {
                        workflow_run
                            .node_runs()
                            .iter()
                            .find(|node_run| node_run.id() == workflow_node_run_id)
                    })
                    .map(|node_run| node_run.has_valid_pending_final_output())
            })
            .unwrap_or(false)
    }

    #[allow(dead_code)]
    pub(super) fn workflow_max_turns(&self, session_id: &str) -> Option<usize> {
        self.session_store
            .get_session(session_id)
            .ok()
            .and_then(|session| {
                session
                    .config_state()
                    .values()
                    .get("workflow.max_turns")
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .filter(|value| *value > 0)
            })
            .or(Some(
                crate::session::DEFAULT_WORKFLOW_RUN_MAX_TURNS_SAFETY_LIMIT,
            ))
    }

    pub(super) fn workflow_record_failure(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        failure: &crate::session::WorkflowFailureEvent,
    ) {
        let _ = self.session_store.write().record_workflow_failure_event(
            session_id,
            workflow_run_id,
            failure.clone(),
        );
    }

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
                },
            ),
        )
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

    pub(super) fn workflow_maybe_start_next_queued_launch(&self, session_id: &str) {
        let queued_launch = match self
            .session_store
            .write()
            .dequeue_next_workflow_launch(session_id)
        {
            Ok(Some(queued_launch)) => queued_launch,
            Ok(None) => return,
            Err(error) => {
                self.record_notice(
                    session_id,
                    None,
                    self.attachment_store
                        .list_session_attachment_ids(session_id),
                    format!("Failed to start queued workflow launch: {error}"),
                );
                return;
            }
        };
        if let Some(watchdog_id) = queued_launch.watchdog_id() {
            let _ = self
                .session_store
                .write()
                .mark_workflow_watchdog_pending_started(session_id, watchdog_id);
        }
        match self.workflow_invoke_queued_launch(session_id, queued_launch.clone()) {
            Ok((
                crate::app::workflow_runtime::WorkflowLaunchOutcome::Started {
                    workflow_run,
                    workflow,
                    endpoint,
                },
                _dispatches,
            )) => {
                self.record_notice(
                    session_id,
                    None,
                    self.attachment_store
                        .list_session_attachment_ids(session_id),
                    format!(
                        "Started queued workflow run `{}` for workflow `{}` endpoint `{}`.",
                        workflow_run.id(),
                        workflow.id(),
                        endpoint.id()
                    ),
                );
            }
            Ok((crate::app::workflow_runtime::WorkflowLaunchOutcome::Queued { .. }, _)) => {}
            Err(error) => {
                if let Some(watchdog_id) = queued_launch.watchdog_id() {
                    let _ = self.session_store.write().mark_workflow_watchdog_failed(
                        session_id,
                        watchdog_id,
                        error.to_string(),
                    );
                }
                self.record_notice(
                    session_id,
                    None,
                    self.attachment_store
                        .list_session_attachment_ids(session_id),
                    format!(
                        "Queued workflow launch `{}` failed: {error}",
                        queued_launch.id()
                    ),
                );
            }
        }
    }

    pub(super) fn workflow_resume_run(
        &self,
        session_id: &str,
        workflow_run_ref: &str,
    ) -> Result<(crate::session::WorkflowRun, WorkflowPromptDispatches), DaemonError> {
        let workflow_run = self
            .session_store
            .write()
            .resume_workflow_run(session_id, workflow_run_ref)?;
        let resumable = workflow_run
            .node_runs()
            .iter()
            .filter_map(|node_run| {
                let prompt = node_run.turn_envelope()?.rendered_prompt()?.to_string();
                Some((
                    node_run.id().to_string(),
                    node_run.agent_id().to_string(),
                    prompt,
                ))
            })
            .collect::<Vec<_>>();
        let mut dispatches = WorkflowPromptDispatches::default();
        for (workflow_node_run_id, agent_id, prompt_text) in resumable {
            let prompt = crate::session::PromptQueueItem::new(
                self.session_store.reserve_prompt_id(),
                crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
                agent_id,
                prompt_text,
                crate::session::PromptStatus::Queued,
            )
            .with_workflow_context(workflow_run.id(), &workflow_node_run_id);
            match self.workflow_submit_prepared_prompt(
                crate::app::KernelPreparedPromptSubmission {
                    session_id: session_id.to_string(),
                    prompt,
                    force_queue: false,
                },
                workflow_run.id(),
                &workflow_node_run_id,
            ) {
                Ok(prepared) => dispatches.extend(prepared),
                Err(error) => {
                    self.record_notice(
                        session_id,
                        None,
                        self.attachment_store
                            .list_session_attachment_ids(session_id),
                        format!(
                            "Workflow run `{}` could not resume node prompt: {}",
                            workflow_run.id(),
                            error
                        ),
                    );
                }
            }
        }
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run.id())?;
        Ok((workflow_run, dispatches))
    }
}
