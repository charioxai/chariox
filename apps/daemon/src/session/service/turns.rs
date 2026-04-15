use super::*;

impl SessionService {
    pub fn cancel_workflow_run(
        &mut self,
        session_id: &str,
        workflow_run_ref: &str,
    ) -> Result<WorkflowRun, DaemonError> {
        let workflow_run_id = self
            .resolve_workflow_run_ref(session_id, workflow_run_ref)?
            .id()
            .to_string();
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let cancelled = {
            let workflow_run = session.workflow_run_mut(&workflow_run_id).ok_or_else(|| {
                DaemonError::WorkflowRunNotFound {
                    session_id: session_id.to_string(),
                    workflow_run_id: workflow_run_id.clone(),
                }
            })?;
            if matches!(
                workflow_run.status(),
                WorkflowRunStatus::Completed
                    | WorkflowRunStatus::Failed
                    | WorkflowRunStatus::Stopped
            ) {
                return Err(DaemonError::InvalidWorkflowRunState {
                    workflow_run_id: workflow_run_id.clone(),
                    status: workflow_run.status(),
                    operation: "cancel workflow run",
                });
            }
            workflow_run.set_status(WorkflowRunStatus::Stopped);
            workflow_run.clear_active_node_run();
            for node_run in workflow_run.node_runs_mut() {
                if !matches!(
                    node_run.status(),
                    WorkflowNodeRunStatus::Completed
                        | WorkflowNodeRunStatus::Failed
                        | WorkflowNodeRunStatus::Stopped
                ) {
                    node_run.set_status(WorkflowNodeRunStatus::Stopped);
                    if let Some(envelope) = node_run.turn_envelope_mut() {
                        envelope.mark_cancelled();
                    }
                }
            }
            workflow_run.clone()
        };
        session.remove_queued_prompts_by_workflow_run(&workflow_run_id);
        Ok(cancelled)
    }

    pub fn start_workflow_node_run(
        &mut self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> Result<WorkflowRun, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let workflow_run = session.workflow_run_mut(workflow_run_id).ok_or_else(|| {
            DaemonError::WorkflowRunNotFound {
                session_id: session_id.to_string(),
                workflow_run_id: workflow_run_id.to_string(),
            }
        })?;
        let workflow_id = workflow_run.workflow_id().to_string();
        let node_run = workflow_run
            .node_runs_mut()
            .iter_mut()
            .find(|node_run| node_run.id() == workflow_node_run_id)
            .ok_or_else(|| DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                reference: workflow_node_run_id.to_string(),
                message: "workflow node run was not found",
            })?;
        node_run.set_status(WorkflowNodeRunStatus::Running);
        workflow_run.set_active_node_run(workflow_node_run_id.to_string());
        workflow_run.set_status(WorkflowRunStatus::Running);
        Ok(workflow_run.clone())
    }

    pub fn prepare_workflow_turn(
        &mut self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
        delivery_token: String,
        rendered_prompt: String,
        mailbox_content: Option<String>,
        handoff_payloads_json: Option<String>,
    ) -> Result<WorkflowRun, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let workflow_run = session.workflow_run_mut(workflow_run_id).ok_or_else(|| {
            DaemonError::WorkflowRunNotFound {
                session_id: session_id.to_string(),
                workflow_run_id: workflow_run_id.to_string(),
            }
        })?;
        let workflow_id = workflow_run.workflow_id().to_string();
        let node_run = workflow_run
            .node_runs_mut()
            .iter_mut()
            .find(|node_run| node_run.id() == workflow_node_run_id)
            .ok_or_else(|| DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                reference: workflow_node_run_id.to_string(),
                message: "workflow node run was not found",
            })?;
        node_run.set_turn_envelope(Some(WorkflowTurnEnvelope::new(
            delivery_token,
            rendered_prompt,
            mailbox_content,
            handoff_payloads_json,
        )));
        Ok(workflow_run.clone())
    }

    pub fn mark_workflow_turn_dispatched(
        &mut self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> Result<WorkflowRun, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let workflow_run = session.workflow_run_mut(workflow_run_id).ok_or_else(|| {
            DaemonError::WorkflowRunNotFound {
                session_id: session_id.to_string(),
                workflow_run_id: workflow_run_id.to_string(),
            }
        })?;
        let workflow_id = workflow_run.workflow_id().to_string();
        let node_run = workflow_run
            .node_runs_mut()
            .iter_mut()
            .find(|node_run| node_run.id() == workflow_node_run_id)
            .ok_or_else(|| DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                reference: workflow_node_run_id.to_string(),
                message: "workflow node run was not found",
            })?;
        let envelope = node_run.turn_envelope_mut().ok_or_else(|| {
            DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                reference: workflow_node_run_id.to_string(),
                message: "workflow turn envelope was not prepared",
            }
        })?;
        envelope.mark_dispatched();
        Ok(workflow_run.clone())
    }

    pub fn block_workflow_node_on_workspace_claim(
        &mut self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> Result<WorkflowRun, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let workflow_run = session.workflow_run_mut(workflow_run_id).ok_or_else(|| {
            DaemonError::WorkflowRunNotFound {
                session_id: session_id.to_string(),
                workflow_run_id: workflow_run_id.to_string(),
            }
        })?;
        let workflow_id = workflow_run.workflow_id().to_string();
        let node_run = workflow_run
            .node_runs_mut()
            .iter_mut()
            .find(|node_run| node_run.id() == workflow_node_run_id)
            .ok_or_else(|| DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id,
                reference: workflow_node_run_id.to_string(),
                message: "workflow node run was not found",
            })?;
        node_run.set_status(WorkflowNodeRunStatus::BlockedOnWorkspaceClaim);
        workflow_run.clear_active_node_run();
        workflow_run.set_status(WorkflowRunStatus::Waiting);
        Ok(workflow_run.clone())
    }

    pub fn ready_workflow_node_after_workspace_claim(
        &mut self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> Result<WorkflowRun, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let workflow_run = session.workflow_run_mut(workflow_run_id).ok_or_else(|| {
            DaemonError::WorkflowRunNotFound {
                session_id: session_id.to_string(),
                workflow_run_id: workflow_run_id.to_string(),
            }
        })?;
        let workflow_id = workflow_run.workflow_id().to_string();
        let node_run = workflow_run
            .node_runs_mut()
            .iter_mut()
            .find(|node_run| node_run.id() == workflow_node_run_id)
            .ok_or_else(|| DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id,
                reference: workflow_node_run_id.to_string(),
                message: "workflow node run was not found",
            })?;
        if node_run.status() == WorkflowNodeRunStatus::BlockedOnWorkspaceClaim {
            node_run.set_status(WorkflowNodeRunStatus::Ready);
        }
        Ok(workflow_run.clone())
    }

    pub fn ack_workflow_turn(
        &mut self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
        delivery_token: &str,
    ) -> Result<WorkflowRun, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let workflow_run = session.workflow_run_mut(workflow_run_id).ok_or_else(|| {
            DaemonError::WorkflowRunNotFound {
                session_id: session_id.to_string(),
                workflow_run_id: workflow_run_id.to_string(),
            }
        })?;
        let workflow_id = workflow_run.workflow_id().to_string();
        let node_run = workflow_run
            .node_runs_mut()
            .iter_mut()
            .find(|node_run| node_run.id() == workflow_node_run_id)
            .ok_or_else(|| DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                reference: workflow_node_run_id.to_string(),
                message: "workflow node run was not found",
            })?;
        let envelope = node_run.turn_envelope_mut().ok_or_else(|| {
            DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                reference: workflow_node_run_id.to_string(),
                message: "workflow turn envelope was not prepared",
            }
        })?;
        if envelope.delivery_token() != delivery_token {
            return Err(DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id,
                reference: workflow_node_run_id.to_string(),
                message: "workflow turn delivery token did not match",
            });
        }
        if matches!(
            envelope.state(),
            WorkflowTurnRuntimeState::Dispatched | WorkflowTurnRuntimeState::Acknowledged
        ) {
            envelope.mark_acknowledged();
        }
        Ok(workflow_run.clone())
    }

    pub fn mark_workflow_turn_validated_completed(
        &mut self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> Result<WorkflowRun, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let workflow_run = session.workflow_run_mut(workflow_run_id).ok_or_else(|| {
            DaemonError::WorkflowRunNotFound {
                session_id: session_id.to_string(),
                workflow_run_id: workflow_run_id.to_string(),
            }
        })?;
        let workflow_id = workflow_run.workflow_id().to_string();
        let node_run = workflow_run
            .node_runs_mut()
            .iter_mut()
            .find(|node_run| node_run.id() == workflow_node_run_id)
            .ok_or_else(|| DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                reference: workflow_node_run_id.to_string(),
                message: "workflow node run was not found",
            })?;
        let envelope = node_run.turn_envelope_mut().ok_or_else(|| {
            DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                reference: workflow_node_run_id.to_string(),
                message: "workflow turn envelope was not prepared",
            }
        })?;
        if envelope.state() != WorkflowTurnRuntimeState::Acknowledged {
            return Ok(workflow_run.clone());
        }
        envelope.mark_validated_completed();
        envelope.clear_transient_inputs();
        workflow_run.retain_messages(|message| {
            message.consumed_by_node_run_id() != Some(workflow_node_run_id)
        });
        Ok(workflow_run.clone())
    }

    pub fn record_workflow_runtime_tool_call(
        &mut self,
        session_id: &str,
        workflow_node_run_id: &str,
        event: WorkflowRuntimeToolCallEvent,
    ) -> Result<WorkflowNodeRun, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let node_run = session
            .workflow_node_run_mut(workflow_node_run_id)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "record_workflow_runtime_tool_call",
                message: format!(
                    "workflow node run `{workflow_node_run_id}` not found in session `{session_id}`"
                ),
            })?;
        let envelope = node_run
            .turn_envelope_mut()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "record_workflow_runtime_tool_call",
                message: format!(
                    "workflow node run `{workflow_node_run_id}` has no active turn envelope"
                ),
            })?;
        envelope.add_runtime_tool_call(event);
        Ok(node_run.clone())
    }

    pub fn submit_workflow_run_final_output(
        &mut self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
        output: WorkflowOutputPayload,
        valid: bool,
        warning: Option<String>,
    ) -> Result<WorkflowRun, DaemonError> {
        self.submit_workflow_run_output_submission(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
            WorkflowTurnSubmissionKind::Final,
            output,
            valid,
            warning,
        )
    }

    pub fn submit_workflow_run_intermediate_output(
        &mut self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
        output: WorkflowOutputPayload,
        valid: bool,
        warning: Option<String>,
    ) -> Result<WorkflowRun, DaemonError> {
        self.submit_workflow_run_output_submission(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
            WorkflowTurnSubmissionKind::Intermediate,
            output,
            valid,
            warning,
        )
    }

    fn submit_workflow_run_output_submission(
        &mut self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
        submission_kind: WorkflowTurnSubmissionKind,
        output: WorkflowOutputPayload,
        valid: bool,
        warning: Option<String>,
    ) -> Result<WorkflowRun, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let workflow_run = session.workflow_run_mut(workflow_run_id).ok_or_else(|| {
            DaemonError::WorkflowRunNotFound {
                session_id: session_id.to_string(),
                workflow_run_id: workflow_run_id.to_string(),
            }
        })?;
        let workflow_id = workflow_run.workflow_id().to_string();
        let node_run = workflow_run
            .node_run_mut(workflow_node_run_id)
            .ok_or_else(|| DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                reference: workflow_node_run_id.to_string(),
                message: "workflow node run was not found",
            })?;
        let envelope = node_run.turn_envelope_mut().ok_or_else(|| {
            DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                reference: workflow_node_run_id.to_string(),
                message: "workflow turn envelope was not prepared",
            }
        })?;
        envelope.set_pending_output_submission(
            submission_kind,
            Some(WorkflowRunOutputSubmission::new(output, valid, warning)),
        );
        Ok(workflow_run.clone())
    }

    pub fn stop_workflow_run_with_final_output(
        &mut self,
        session_id: &str,
        workflow_run_id: &str,
        final_output: Option<WorkflowOutputPayload>,
        final_output_valid: Option<bool>,
        final_output_warning: Option<String>,
        completed_by_node_run_id: Option<String>,
    ) -> Result<WorkflowRun, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let workflow_run = session.workflow_run_mut(workflow_run_id).ok_or_else(|| {
            DaemonError::WorkflowRunNotFound {
                session_id: session_id.to_string(),
                workflow_run_id: workflow_run_id.to_string(),
            }
        })?;
        workflow_run.set_final_output(
            final_output,
            final_output_valid,
            final_output_warning,
            completed_by_node_run_id,
        );
        workflow_run.clear_active_node_run();
        for node_run in workflow_run.node_runs_mut() {
            if !matches!(
                node_run.status(),
                WorkflowNodeRunStatus::Completed
                    | WorkflowNodeRunStatus::Failed
                    | WorkflowNodeRunStatus::Stopped
            ) {
                node_run.set_status(WorkflowNodeRunStatus::Stopped);
            }
        }
        workflow_run.retain_messages(|_| false);
        workflow_run.set_status(WorkflowRunStatus::Stopped);
        Ok(workflow_run.clone())
    }

    pub fn record_workflow_failure_event(
        &mut self,
        session_id: &str,
        workflow_run_id: &str,
        event: WorkflowFailureEvent,
    ) -> Result<WorkflowRun, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let workflow_run = session.workflow_run_mut(workflow_run_id).ok_or_else(|| {
            DaemonError::WorkflowRunNotFound {
                session_id: session_id.to_string(),
                workflow_run_id: workflow_run_id.to_string(),
            }
        })?;
        workflow_run.add_failure_event(event);
        Ok(workflow_run.clone())
    }

    pub fn read_workflow_console(
        &self,
        session_id: &str,
        workflow_id: &str,
    ) -> Result<WorkflowConsole, DaemonError> {
        let session = self.get_session(session_id)?;
        if session.workflow(workflow_id).is_none() {
            return Err(DaemonError::WorkflowNotFound {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.to_string(),
            });
        }
        Ok(session
            .workflow_console(workflow_id)
            .cloned()
            .unwrap_or_else(|| WorkflowConsole::new(workflow_id)))
    }

    pub fn append_workflow_console_entry(
        &mut self,
        session_id: &str,
        workflow_id: &str,
        source_node_run_id: Option<String>,
        source_agent_id: Option<String>,
        text: impl Into<String>,
    ) -> Result<WorkflowConsoleEntry, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        if session.workflow(workflow_id).is_none() {
            return Err(DaemonError::WorkflowNotFound {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.to_string(),
            });
        }
        let entry = WorkflowConsoleEntry::new(source_node_run_id, source_agent_id, text);
        Ok(session
            .ensure_workflow_console(workflow_id)
            .add_entry(entry))
    }

    pub fn clear_workflow_console(
        &mut self,
        session_id: &str,
        workflow_id: &str,
    ) -> Result<WorkflowConsole, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        if session.workflow(workflow_id).is_none() {
            return Err(DaemonError::WorkflowNotFound {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.to_string(),
            });
        }
        let console = session.ensure_workflow_console(workflow_id);
        console.clear();
        Ok(console.clone())
    }

    pub fn resume_workflow_run(
        &mut self,
        session_id: &str,
        workflow_run_ref: &str,
    ) -> Result<WorkflowRun, DaemonError> {
        let workflow_run_id = self
            .resolve_workflow_run_ref(session_id, workflow_run_ref)?
            .id()
            .to_string();
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let workflow_run = session.workflow_run_mut(&workflow_run_id).ok_or_else(|| {
            DaemonError::WorkflowRunNotFound {
                session_id: session_id.to_string(),
                workflow_run_id: workflow_run_id.clone(),
            }
        })?;
        let resumable_node_ids = workflow_run
            .node_runs()
            .iter()
            .filter(|node_run| {
                node_run.status() == WorkflowNodeRunStatus::Stopped
                    && node_run.completion().is_none()
                    && node_run
                        .turn_envelope()
                        .and_then(|envelope| envelope.rendered_prompt())
                        .is_some()
            })
            .map(|node_run| node_run.id().to_string())
            .collect::<Vec<_>>();
        if resumable_node_ids.is_empty() {
            return Err(DaemonError::InvalidWorkflowRunState {
                workflow_run_id,
                status: workflow_run.status(),
                operation: "resume workflow run",
            });
        }
        workflow_run.resume();
        workflow_run.clear_active_node_run();
        for node_run in workflow_run.node_runs_mut() {
            if resumable_node_ids.iter().any(|id| id == node_run.id()) {
                node_run.resume();
            }
        }
        Ok(workflow_run.clone())
    }

    pub fn complete_workflow_node_run(
        &mut self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
        completion: Option<WorkflowCompletionSnapshot>,
        max_turns: Option<usize>,
    ) -> Result<WorkflowCompletionUpdate, DaemonError> {
        let context = self.load_workflow_completion_context(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        )?;
        let (emitted_messages, validation_warnings) =
            self.build_workflow_completion_messages(session_id, &context, completion.as_ref())?;

        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let workflow_run = session.workflow_run_mut(workflow_run_id).ok_or_else(|| {
            DaemonError::WorkflowRunNotFound {
                session_id: session_id.to_string(),
                workflow_run_id: workflow_run_id.to_string(),
            }
        })?;
        let workflow_id_for_error = workflow_run.workflow_id().to_string();
        let node_run = workflow_run
            .node_runs_mut()
            .iter_mut()
            .find(|node_run| node_run.id() == workflow_node_run_id)
            .ok_or_else(|| DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_id_for_error,
                reference: workflow_node_run_id.to_string(),
                message: "workflow node run was not found",
            })?;
        let completed_node_id = node_run.node_id().to_string();
        let has_pending_final_output = node_run
            .turn_envelope()
            .and_then(|envelope| {
                envelope.pending_output_submission(WorkflowTurnSubmissionKind::Final)
            })
            .is_some();
        let should_clear_resolved_failures = validation_warnings.is_empty()
            && (completion
                .as_ref()
                .is_some_and(|snapshot| snapshot.output().is_some())
                || has_pending_final_output);
        let pending_outputs =
            Self::take_pending_workflow_turn_outputs(node_run.turn_envelope_mut());
        Self::apply_workflow_node_completion(node_run, completion);
        workflow_run.clear_active_node_run();
        Self::commit_pending_workflow_turn_outputs(
            workflow_run,
            workflow_node_run_id,
            pending_outputs,
        );
        if should_clear_resolved_failures {
            let resolved_source_node_run_ids = workflow_run
                .node_runs()
                .iter()
                .filter(|node_run| node_run.node_id() == completed_node_id)
                .map(|node_run| node_run.id().to_string())
                .collect::<std::collections::BTreeSet<_>>();
            workflow_run.retain_failure_events(|event| {
                let resolved_failure = matches!(
                    event.kind(),
                    WorkflowFailureKind::MissingStructuredOutput
                        | WorkflowFailureKind::OutputValidationFailed
                ) && resolved_source_node_run_ids
                    .contains(event.source_node_run_id());
                !resolved_failure
            });
        }
        for message in emitted_messages {
            workflow_run.add_message(message);
        }
        if workflow_run.completed_by_node_run_id() == Some(workflow_node_run_id) {
            workflow_run.retain_messages(|_| false);
            Self::stop_other_workflow_node_runs(workflow_run, workflow_node_run_id);
            workflow_run.set_status(WorkflowRunStatus::Completed);
            return Ok(WorkflowCompletionUpdate {
                workflow_run: workflow_run.clone(),
                dispatches: Vec::new(),
                validation_warnings,
            });
        }
        let workflow_id = workflow_run.workflow_id().to_string();
        let dispatches = collect_ready_workflow_dispatches(
            &mut self.next_workflow_node_run_number,
            session_id,
            &workflow_id,
            &context.workflow,
            workflow_run,
        )?;
        let node_turn_budget_exhausted = context
            .workflow
            .node(context.source_node_run.node_id())
            .and_then(|node| node.max_turns())
            .is_some_and(|limit| {
                let completed_turns = workflow_run
                    .node_runs()
                    .iter()
                    .filter(|node_run| node_run.node_id() == context.source_node_run.node_id())
                    .filter(|node_run| node_run.completion().is_some())
                    .count() as u32;
                completed_turns >= limit
            });
        let max_turns_reached = max_turns
            .filter(|limit| *limit > 0)
            .is_some_and(|limit| workflow_run.node_runs().len() >= limit);
        let has_unconsumed_messages = workflow_run
            .messages()
            .iter()
            .any(|message| message.consumed_by_node_run_id().is_none());
        let has_pending_node_runs = workflow_run.node_runs().iter().any(|node_run| {
            !matches!(
                node_run.status(),
                WorkflowNodeRunStatus::Completed
                    | WorkflowNodeRunStatus::Failed
                    | WorkflowNodeRunStatus::Stopped
            )
        });
        if node_turn_budget_exhausted {
            workflow_run.retain_messages(|_| false);
            Self::stop_other_workflow_node_runs(workflow_run, workflow_node_run_id);
            workflow_run.set_final_output(None, None, None, None);
            workflow_run.set_status(WorkflowRunStatus::Stopped);
            return Ok(WorkflowCompletionUpdate {
                workflow_run: workflow_run.clone(),
                dispatches: Vec::new(),
                validation_warnings,
            });
        }
        if max_turns_reached {
            workflow_run.set_status(WorkflowRunStatus::Stopped);
            return Ok(WorkflowCompletionUpdate {
                workflow_run: workflow_run.clone(),
                dispatches: Vec::new(),
                validation_warnings,
            });
        }
        workflow_run.set_status(if has_unconsumed_messages || has_pending_node_runs {
            WorkflowRunStatus::Waiting
        } else {
            WorkflowRunStatus::Completed
        });
        Ok(WorkflowCompletionUpdate {
            workflow_run: workflow_run.clone(),
            dispatches,
            validation_warnings,
        })
    }

    fn load_workflow_completion_context(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> Result<WorkflowCompletionContext, DaemonError> {
        let session = self
            .store
            .get(session_id)
            .ok_or_else(|| DaemonError::SessionNotFound {
                session_id: session_id.to_string(),
            })?;
        let workflow_run = session
            .workflow_run(workflow_run_id)
            .ok_or_else(|| DaemonError::WorkflowRunNotFound {
                session_id: session_id.to_string(),
                workflow_run_id: workflow_run_id.to_string(),
            })?
            .clone();
        let source_node_run = workflow_run
            .node_runs()
            .iter()
            .find(|node_run| node_run.id() == workflow_node_run_id)
            .cloned()
            .ok_or_else(|| DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_run.workflow_id().to_string(),
                reference: workflow_node_run_id.to_string(),
                message: "workflow node run was not found",
            })?;
        let workflow = session
            .workflow(workflow_run.workflow_id())
            .ok_or_else(|| DaemonError::WorkflowNotFound {
                session_id: session_id.to_string(),
                workflow_id: workflow_run.workflow_id().to_string(),
            })?
            .clone();
        Ok(WorkflowCompletionContext {
            workflow_run,
            source_node_run,
            workflow,
        })
    }

    fn build_workflow_completion_messages(
        &mut self,
        session_id: &str,
        context: &WorkflowCompletionContext,
        completion: Option<&WorkflowCompletionSnapshot>,
    ) -> Result<(Vec<WorkflowMessage>, Vec<WorkflowOutputValidationWarning>), DaemonError> {
        if context.workflow_run.completed_by_node_run_id() == Some(context.source_node_run.id()) {
            return Ok((Vec::new(), Vec::new()));
        }
        if context
            .source_node_run
            .turn_envelope()
            .and_then(|envelope| {
                envelope.pending_output_submission(WorkflowTurnSubmissionKind::Final)
            })
            .is_some()
        {
            return Ok((Vec::new(), Vec::new()));
        }
        let mut validation_warnings = Vec::new();
        let completion = completion.cloned();
        let emitted_messages = context
            .workflow
            .edges()
            .iter()
            .filter(|edge| edge.from_node_id() == context.source_node_run.node_id())
            .map(|edge| {
                let warning = validate_workflow_edge_output(
                    session_id,
                    &context.workflow,
                    edge,
                    &completion,
                )?;
                if let Some(message) = warning.as_ref() {
                    validation_warnings.push(WorkflowOutputValidationWarning {
                        edge_id: edge.id().to_string(),
                        message: message.clone(),
                    });
                }
                let target_node = context.workflow.node(edge.to_node_id()).ok_or_else(|| {
                    DaemonError::InvalidWorkflowGraphReference {
                        session_id: session_id.to_string(),
                        workflow_id: context.workflow.id().to_string(),
                        reference: edge.to_node_id().to_string(),
                        message: "target node does not exist",
                    }
                })?;
                let payload = WorkflowHandoffPayload::new(
                    context.workflow_run.id().to_string(),
                    context.workflow.id().to_string(),
                    context.source_node_run.id().to_string(),
                    context.source_node_run.node_id().to_string(),
                    context.source_node_run.agent_id().to_string(),
                    target_node.id().to_string(),
                    context.workflow_run.invocation_prompt().map(str::to_string),
                    completion.clone(),
                    edge.output_schema_ref().map(str::to_string),
                    warning.clone(),
                );
                let message = WorkflowMessage::new(
                    self.next_workflow_message_id(),
                    Some(context.source_node_run.id().to_string()),
                    target_node.id().to_string(),
                    "handoff",
                    format!(
                        "handoff from `{}` to `{}`",
                        context.source_node_run.node_id(),
                        target_node.id()
                    ),
                    serde_json::to_string(&payload).map_err(|error| {
                        DaemonError::LocalTransport {
                            operation: "serialize workflow handoff payload",
                            message: error.to_string(),
                        }
                    })?,
                );
                Ok(message)
            })
            .collect::<Result<Vec<_>, DaemonError>>()?;
        Ok((emitted_messages, validation_warnings))
    }

    fn take_pending_workflow_turn_outputs(
        envelope: Option<&mut WorkflowTurnEnvelope>,
    ) -> PendingWorkflowTurnOutputs {
        let intermediate = envelope.as_ref().and_then(|envelope| {
            envelope
                .pending_output_submission(WorkflowTurnSubmissionKind::Intermediate)
                .cloned()
        });
        let final_output = envelope.as_ref().and_then(|envelope| {
            envelope
                .pending_output_submission(WorkflowTurnSubmissionKind::Final)
                .cloned()
        });
        if let Some(envelope) = envelope {
            envelope.set_pending_output_submission(WorkflowTurnSubmissionKind::Intermediate, None);
            envelope.set_pending_output_submission(WorkflowTurnSubmissionKind::Final, None);
        }
        PendingWorkflowTurnOutputs {
            intermediate,
            final_output,
        }
    }

    fn apply_workflow_node_completion(
        node_run: &mut WorkflowNodeRun,
        completion: Option<WorkflowCompletionSnapshot>,
    ) {
        node_run.set_status(WorkflowNodeRunStatus::Completed);
        node_run.set_summary(Some(
            completion
                .as_ref()
                .map(|value| value.summary().to_string())
                .unwrap_or_else(|| "completed".to_string()),
        ));
        node_run.set_completion(completion);
    }

    fn commit_pending_workflow_turn_outputs(
        workflow_run: &mut WorkflowRun,
        workflow_node_run_id: &str,
        pending_outputs: PendingWorkflowTurnOutputs,
    ) {
        if let Some(submission) = pending_outputs.intermediate {
            workflow_run.add_intermediate_output(WorkflowIntermediateOutput::new(
                format!(
                    "workflow-intermediate-output-{}-{}",
                    workflow_node_run_id,
                    unix_epoch_ms()
                ),
                workflow_node_run_id.to_string(),
                submission.output().clone(),
                submission.valid(),
                submission.warning().map(str::to_string),
            ));
        }
        if let Some(submission) = pending_outputs.final_output {
            workflow_run.set_final_output(
                Some(submission.output().clone()),
                Some(submission.valid()),
                submission.warning().map(str::to_string),
                Some(workflow_node_run_id.to_string()),
            );
            workflow_run.set_status(WorkflowRunStatus::Completing);
        }
    }

    fn stop_other_workflow_node_runs(workflow_run: &mut WorkflowRun, workflow_node_run_id: &str) {
        for other_node_run in workflow_run.node_runs_mut() {
            if other_node_run.id() != workflow_node_run_id
                && !matches!(
                    other_node_run.status(),
                    WorkflowNodeRunStatus::Completed
                        | WorkflowNodeRunStatus::Failed
                        | WorkflowNodeRunStatus::Stopped
                )
            {
                other_node_run.set_status(WorkflowNodeRunStatus::Stopped);
            }
        }
    }

    pub fn stop_workflow_node_run(
        &mut self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> Result<WorkflowRun, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let workflow_run = session.workflow_run_mut(workflow_run_id).ok_or_else(|| {
            DaemonError::WorkflowRunNotFound {
                session_id: session_id.to_string(),
                workflow_run_id: workflow_run_id.to_string(),
            }
        })?;
        let workflow_id = workflow_run.workflow_id().to_string();
        let node_run = workflow_run
            .node_runs_mut()
            .iter_mut()
            .find(|node_run| node_run.id() == workflow_node_run_id)
            .ok_or_else(|| DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id,
                reference: workflow_node_run_id.to_string(),
                message: "workflow node run was not found",
            })?;
        node_run.set_status(WorkflowNodeRunStatus::Stopped);
        if let Some(envelope) = node_run.turn_envelope_mut() {
            envelope.mark_cancelled();
        }
        workflow_run.clear_active_node_run();
        workflow_run.set_status(WorkflowRunStatus::Stopped);
        Ok(workflow_run.clone())
    }
}
