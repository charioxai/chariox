use super::*;

impl SessionService {
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
        let mut pending_outputs =
            Self::take_pending_workflow_turn_outputs(node_run.turn_envelope_mut());
        if pending_outputs.final_output.is_none() {
            pending_outputs.final_output =
                Self::fallback_terminal_completion_as_final_output(&context, completion.as_ref());
        }
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

    pub(super) fn load_workflow_completion_context(
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
    ) -> Result<(Vec<WorkflowMessage>, Vec<WorkflowHandoffValidationWarning>), DaemonError> {
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
        let mut emitted_messages = Vec::new();
        for edge in context
            .workflow
            .edges()
            .iter()
            .filter(|edge| edge.from_node_id() == context.source_node_run.node_id())
        {
            let Some(edge_completion) = Self::workflow_edge_completion(edge, completion.as_ref())
            else {
                continue;
            };
            let warning = validate_workflow_edge_handoff(
                session_id,
                &context.workflow,
                edge,
                &edge_completion,
            )?;
            if let Some(message) = warning.as_ref() {
                validation_warnings.push(WorkflowHandoffValidationWarning {
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
                Some(edge.id().to_string()),
                context.source_node_run.id().to_string(),
                context.source_node_run.node_id().to_string(),
                Some(context.source_node_run.iteration_index()),
                context.source_node_run.agent_id().to_string(),
                target_node.id().to_string(),
                context.workflow_run.invocation_prompt().map(str::to_string),
                edge_completion.clone(),
                edge.handoff_schema_ref().map(str::to_string),
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
                serde_json::to_string(&payload).map_err(|error| DaemonError::LocalTransport {
                    operation: "serialize workflow handoff payload",
                    message: error.to_string(),
                })?,
            );
            let mut message = message;
            message.set_edge_id(edge.id().to_string());
            message.set_source_node_iteration_index(context.source_node_run.iteration_index());
            emitted_messages.push(message);
        }
        Ok((emitted_messages, validation_warnings))
    }

    fn workflow_edge_completion(
        edge: &WorkflowEdgeDefinition,
        completion: Option<&WorkflowCompletionSnapshot>,
    ) -> Option<Option<WorkflowCompletionSnapshot>> {
        let Some(output) = completion.and_then(|snapshot| snapshot.output()) else {
            return Some(completion.cloned());
        };
        let Ok(value) = serde_json::from_str::<Value>(output.message()) else {
            return Some(completion.cloned());
        };
        let Some(handoffs) = value
            .get("workflow_handoffs")
            .and_then(Value::as_array)
            .filter(|handoffs| !handoffs.is_empty())
        else {
            return Some(completion.cloned());
        };
        let handoff = handoffs.iter().find(|handoff| {
            handoff
                .get("edge_id")
                .and_then(Value::as_str)
                .is_some_and(|edge_id| edge_id == edge.id())
                || handoff
                    .get("to_node_id")
                    .and_then(Value::as_str)
                    .is_some_and(|node_id| node_id == edge.to_node_id())
        })?;
        let message_value = handoff
            .get("output")
            .and_then(|output| output.get("message"))
            .or_else(|| handoff.get("message"))
            .cloned()
            .or_else(|| Self::workflow_handoff_inline_message(handoff))
            .unwrap_or(Value::Null);
        if message_value.is_null() {
            return None;
        }
        let message = match message_value {
            Value::String(message) => message,
            other => other.to_string(),
        };
        let artifacts = completion
            .and_then(|snapshot| snapshot.output())
            .map(|output| output.artifacts().to_vec())
            .unwrap_or_default();
        let summary = handoff
            .get("summary")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| completion.map(|snapshot| snapshot.summary().to_string()))
            .unwrap_or_else(|| "routed workflow output".to_string());
        Some(Some(WorkflowCompletionSnapshot::new(
            summary,
            Some(WorkflowOutputPayload::new(message, artifacts)),
        )))
    }

    fn workflow_handoff_inline_message(handoff: &Value) -> Option<Value> {
        let object = handoff.as_object()?;
        let mut message = serde_json::Map::new();
        for (key, value) in object {
            if matches!(
                key.as_str(),
                "edge_id" | "to_node_id" | "summary" | "output" | "message"
            ) {
                continue;
            }
            message.insert(key.clone(), value.clone());
        }
        if message.is_empty() {
            None
        } else {
            Some(Value::Object(message))
        }
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

    fn fallback_terminal_completion_as_final_output(
        context: &WorkflowCompletionContext,
        completion: Option<&WorkflowCompletionSnapshot>,
    ) -> Option<WorkflowRunOutputSubmission> {
        let source_node = context.workflow.node(context.source_node_run.node_id())?;
        if !source_node.can_complete_workflow_run() {
            return None;
        }
        if context
            .workflow
            .edges()
            .iter()
            .any(|edge| edge.from_node_id() == source_node.id())
        {
            return None;
        }
        let output = completion.and_then(|snapshot| snapshot.output())?.clone();
        let warning = context
            .workflow
            .run_output_schema_ref()
            .and_then(|schema_ref| {
                validate_workflow_output_schema_ref(&context.workflow, schema_ref, output.message())
                    .err()
            });
        Some(WorkflowRunOutputSubmission::new(
            output,
            warning.is_none(),
            warning,
        ))
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
}

fn validate_workflow_output_schema_ref(
    workflow: &WorkflowDefinition,
    schema_ref: &str,
    output_json: &str,
) -> Result<(), String> {
    if let Some(schema) = workflow.schema(schema_ref) {
        return crate::transport::runtime_tools::validate_json_output_schema(
            schema_ref,
            schema.schema(),
            output_json,
        );
    }
    crate::transport::runtime_tools::validate_workflow_handoff_schema(schema_ref, output_json)
}
