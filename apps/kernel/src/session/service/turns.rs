use super::*;

impl SessionService {
    pub fn pause_workflow_run(
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
        let paused = {
            let workflow_run = session.workflow_run_mut(&workflow_run_id).ok_or_else(|| {
                DaemonError::WorkflowRunNotFound {
                    session_id: session_id.to_string(),
                    workflow_run_id: workflow_run_id.clone(),
                }
            })?;
            if matches!(
                workflow_run.status(),
                WorkflowRunStatus::Paused
                    | WorkflowRunStatus::Completed
                    | WorkflowRunStatus::Failed
                    | WorkflowRunStatus::Stopped
            ) {
                return Err(DaemonError::InvalidWorkflowRunState {
                    workflow_run_id: workflow_run_id.clone(),
                    status: workflow_run.status(),
                    operation: "pause workflow run",
                });
            }
            workflow_run.set_status(WorkflowRunStatus::Paused);
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
        Ok(paused)
    }

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
        let _ = session.release_workflow_runtime_instance_for_run(&workflow_run_id);
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
        if matches!(
            node_run.status(),
            WorkflowNodeRunStatus::Completed
                | WorkflowNodeRunStatus::Failed
                | WorkflowNodeRunStatus::Stopped
        ) {
            return Ok(workflow_run.clone());
        }
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

    pub fn record_workflow_intermediate_output_event(
        &mut self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> Result<WorkflowRun, DaemonError> {
        let context = self.load_workflow_completion_context(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        )?;
        let Some(submission) = context
            .source_node_run
            .turn_envelope()
            .and_then(|envelope| {
                envelope.pending_output_submission(WorkflowTurnSubmissionKind::Intermediate)
            })
            .cloned()
        else {
            return Ok(context.workflow_run);
        };
        if !submission.valid() || submission.warning().is_some() {
            return Ok(context.workflow_run);
        }

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
                workflow_id: workflow_id_for_error.clone(),
                reference: workflow_node_run_id.to_string(),
                message: "workflow node run was not found",
            })?;
        let envelope = node_run.turn_envelope_mut().ok_or_else(|| {
            DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_id_for_error,
                reference: workflow_node_run_id.to_string(),
                message: "workflow turn envelope was not prepared",
            }
        })?;
        envelope.set_pending_output_submission(WorkflowTurnSubmissionKind::Intermediate, None);
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
        Ok(workflow_run.clone())
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
        workflow_run.discard_unconsumed_messages();
        workflow_run.set_status(WorkflowRunStatus::Stopped);
        let stopped = workflow_run.clone();
        let _ = session.release_workflow_runtime_instance_for_run(workflow_run_id);
        Ok(stopped)
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
        let stopped = workflow_run.clone();
        let _ = session.release_workflow_runtime_instance_for_run(workflow_run_id);
        Ok(stopped)
    }

    pub fn fail_workflow_node_run(
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
        node_run.set_status(WorkflowNodeRunStatus::Failed);
        if let Some(envelope) = node_run.turn_envelope_mut() {
            envelope.mark_failed();
        }
        workflow_run.clear_active_node_run();
        workflow_run.set_status(WorkflowRunStatus::Failed);
        let failed = workflow_run.clone();
        let _ = session.release_workflow_runtime_instance_for_run(workflow_run_id);
        Ok(failed)
    }

    pub fn fail_workflow_run(
        &mut self,
        session_id: &str,
        workflow_run_id: &str,
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
        for node_run in workflow_run.node_runs_mut() {
            if !matches!(
                node_run.status(),
                WorkflowNodeRunStatus::Completed
                    | WorkflowNodeRunStatus::Failed
                    | WorkflowNodeRunStatus::Stopped
            ) {
                node_run.set_status(WorkflowNodeRunStatus::Failed);
                if let Some(envelope) = node_run.turn_envelope_mut() {
                    envelope.mark_failed();
                }
            }
        }
        workflow_run.clear_active_node_run();
        workflow_run.set_status(WorkflowRunStatus::Failed);
        let failed = workflow_run.clone();
        let _ = session.release_workflow_runtime_instance_for_run(workflow_run_id);
        Ok(failed)
    }
}
