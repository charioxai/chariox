use super::*;

const DEFAULT_WORKFLOW_RUN_OUTPUT_MAX_ATTEMPTS: u32 = 3;
const MISSING_WORKFLOW_OUTPUT_MESSAGE: &str =
    "provider completed workflow turn without a validated workflow output";

impl SessionService {
    pub fn complete_workflow_node_run(
        &mut self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
        completion: Option<WorkflowCompletionSnapshot>,
        max_turns: Option<usize>,
    ) -> Result<WorkflowCompletionUpdate, DaemonError> {
        self.complete_workflow_node_run_with_missing_output_policy(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
            completion,
            max_turns,
            false,
        )
    }

    pub(crate) fn complete_workflow_node_run_after_provider_turn(
        &mut self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
        completion: Option<WorkflowCompletionSnapshot>,
        max_turns: Option<usize>,
    ) -> Result<WorkflowCompletionUpdate, DaemonError> {
        self.complete_workflow_node_run_with_missing_output_policy(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
            completion,
            max_turns,
            true,
        )
    }

    fn complete_workflow_node_run_with_missing_output_policy(
        &mut self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
        completion: Option<WorkflowCompletionSnapshot>,
        max_turns: Option<usize>,
        retry_missing_output: bool,
    ) -> Result<WorkflowCompletionUpdate, DaemonError> {
        let context = self.load_workflow_completion_context(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        )?;
        let (emitted_messages, validation_warnings, handoff_validation_error) = match self
            .build_workflow_completion_messages(session_id, &context, completion.as_ref())
        {
            Ok((messages, warnings)) => (messages, warnings, None),
            Err(DaemonError::WorkflowHandoffValidationFailed {
                edge_id, message, ..
            }) => (Vec::new(), Vec::new(), Some((edge_id, message))),
            Err(error) => return Err(error),
        };
        let candidate_final_output = context
            .source_node_run
            .turn_envelope()
            .and_then(|envelope| {
                envelope
                    .pending_output_submission(WorkflowTurnSubmissionKind::Final)
                    .cloned()
            })
            .or_else(|| {
                Self::fallback_terminal_completion_as_final_output(&context, completion.as_ref())
            });
        let missing_output_failure =
            (retry_missing_output && completion.is_none() && candidate_final_output.is_none())
                .then(|| Self::workflow_missing_output_failure(&context, max_turns));
        let run_output_validation_failure = candidate_final_output
            .as_ref()
            .filter(|submission| !submission.valid())
            .map(|submission| {
                Self::workflow_run_output_validation_failure(&context, submission, max_turns)
            });
        let handoff_validation_failure = handoff_validation_error.map(|(edge_id, message)| {
            Self::workflow_handoff_validation_failure(&context, edge_id, message, max_turns)
        });
        let retry_prompt = handoff_validation_failure
            .as_ref()
            .filter(|failure| failure.retry_scheduled)
            .map(|failure| Self::workflow_handoff_correction_prompt(&context, failure))
            .or_else(|| {
                missing_output_failure
                    .as_ref()
                    .filter(|failure| failure.retry_scheduled)
                    .map(|failure| {
                        Self::workflow_missing_output_correction_prompt(&context, failure)
                    })
            })
            .or_else(|| {
                run_output_validation_failure
                    .as_ref()
                    .filter(|failure| failure.retry_scheduled)
                    .map(|failure| Self::workflow_run_output_correction_prompt(&context, failure))
            });
        let retry_dispatch =
            retry_prompt.map(|prompt| self.workflow_correction_dispatch(&context, prompt));
        let has_corrective_failure = handoff_validation_failure.is_some()
            || missing_output_failure.is_some()
            || run_output_validation_failure.is_some();

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
            pending_outputs.final_output = candidate_final_output;
        }
        Self::apply_workflow_node_completion(node_run, completion);
        if has_corrective_failure {
            node_run.set_status(WorkflowNodeRunStatus::Failed);
            if let Some(envelope) = node_run.turn_envelope_mut() {
                envelope.mark_failed();
            }
        }
        workflow_run.clear_active_node_run();
        if has_corrective_failure {
            pending_outputs.final_output = None;
            Self::commit_pending_workflow_turn_outputs(
                workflow_run,
                workflow_node_run_id,
                pending_outputs,
            );
            for message in emitted_messages {
                workflow_run.add_message(message);
            }
            let dispatches = retry_dispatch
                .map(|dispatch| {
                    let node_run = workflow_run.add_node_run(dispatch.node_run);
                    workflow_run.set_status(WorkflowRunStatus::Waiting);
                    vec![WorkflowDispatch {
                        node_run,
                        messages: dispatch.messages,
                        endpoint_prompt: dispatch.endpoint_prompt,
                    }]
                })
                .unwrap_or_else(|| {
                    workflow_run.set_status(WorkflowRunStatus::Failed);
                    Vec::new()
                });
            return Ok(WorkflowCompletionUpdate {
                workflow_run: workflow_run.clone(),
                dispatches,
                validation_warnings,
                handoff_validation_failure,
                missing_output_failure,
                run_output_validation_failure,
            });
        }
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
                        | WorkflowFailureKind::WorkflowRunOutputValidationFailed
                ) && resolved_source_node_run_ids
                    .contains(event.source_node_run_id());
                !resolved_failure
            });
        }
        for message in emitted_messages {
            workflow_run.add_message(message);
        }
        if workflow_run.completed_by_node_run_id() == Some(workflow_node_run_id) {
            workflow_run.discard_unconsumed_messages();
            Self::stop_other_workflow_node_runs(workflow_run, workflow_node_run_id);
            workflow_run.set_status(WorkflowRunStatus::Completed);
            return Ok(WorkflowCompletionUpdate {
                workflow_run: workflow_run.clone(),
                dispatches: Vec::new(),
                validation_warnings,
                handoff_validation_failure: None,
                missing_output_failure: None,
                run_output_validation_failure: None,
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
            workflow_run.discard_unconsumed_messages();
            Self::stop_other_workflow_node_runs(workflow_run, workflow_node_run_id);
            workflow_run.set_final_output(None, None, None, None);
            workflow_run.set_status(WorkflowRunStatus::Stopped);
            return Ok(WorkflowCompletionUpdate {
                workflow_run: workflow_run.clone(),
                dispatches: Vec::new(),
                validation_warnings,
                handoff_validation_failure: None,
                missing_output_failure: None,
                run_output_validation_failure: None,
            });
        }
        if max_turns_reached {
            workflow_run.set_status(WorkflowRunStatus::Stopped);
            return Ok(WorkflowCompletionUpdate {
                workflow_run: workflow_run.clone(),
                dispatches: Vec::new(),
                validation_warnings,
                handoff_validation_failure: None,
                missing_output_failure: None,
                run_output_validation_failure: None,
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
            handoff_validation_failure: None,
            missing_output_failure: None,
            run_output_validation_failure: None,
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
        let outgoing_edges = context
            .workflow
            .edges()
            .iter()
            .filter(|edge| edge.from_node_id() == context.source_node_run.node_id())
            .collect::<Vec<_>>();
        Self::validate_workflow_handoff_selectors(
            session_id,
            &context.workflow,
            &outgoing_edges,
            completion.as_ref(),
        )?;
        for edge in outgoing_edges {
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

    fn validate_workflow_handoff_selectors(
        session_id: &str,
        workflow: &WorkflowDefinition,
        outgoing_edges: &[&WorkflowEdgeDefinition],
        completion: Option<&WorkflowCompletionSnapshot>,
    ) -> Result<(), DaemonError> {
        let Some(output) = completion.and_then(|snapshot| snapshot.output()) else {
            return Ok(());
        };
        let Ok(value) = serde_json::from_str::<Value>(output.message()) else {
            return Ok(());
        };
        let Some(handoffs) = value
            .get("workflow_handoffs")
            .and_then(Value::as_array)
            .filter(|handoffs| !handoffs.is_empty())
        else {
            return Ok(());
        };

        for handoff in handoffs {
            let edge_id = handoff.get("edge_id").and_then(Value::as_str);
            let to_node_id = handoff.get("to_node_id").and_then(Value::as_str);
            let matches_outgoing_edge = outgoing_edges.iter().any(|edge| {
                edge_id.is_some_and(|candidate| candidate == edge.id())
                    || to_node_id.is_some_and(|candidate| candidate == edge.to_node_id())
            });
            if matches_outgoing_edge {
                continue;
            }

            let selector = edge_id.or(to_node_id).unwrap_or("<missing selector>");
            return Err(DaemonError::WorkflowHandoffValidationFailed {
                session_id: session_id.to_string(),
                workflow_id: workflow.id().to_string(),
                edge_id: selector.to_string(),
                message: "selected handoff does not match any outgoing workflow edge".to_string(),
            });
        }
        Ok(())
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

    fn workflow_run_output_validation_failure(
        context: &WorkflowCompletionContext,
        submission: &WorkflowRunOutputSubmission,
        max_turns: Option<usize>,
    ) -> WorkflowRunOutputValidationFailure {
        let (attempt, max_attempts, retry_scheduled) =
            Self::workflow_correction_retry_budget(context, max_turns);
        WorkflowRunOutputValidationFailure {
            message: submission
                .warning()
                .unwrap_or("workflow run output validation failed")
                .to_string(),
            attempt,
            max_attempts,
            retry_scheduled,
        }
    }

    fn workflow_handoff_validation_failure(
        context: &WorkflowCompletionContext,
        edge_id: String,
        message: String,
        max_turns: Option<usize>,
    ) -> WorkflowHandoffValidationFailure {
        let (attempt, max_attempts, retry_scheduled) =
            Self::workflow_correction_retry_budget(context, max_turns);
        WorkflowHandoffValidationFailure {
            edge_id,
            message,
            attempt,
            max_attempts,
            retry_scheduled,
        }
    }

    fn workflow_missing_output_failure(
        context: &WorkflowCompletionContext,
        max_turns: Option<usize>,
    ) -> WorkflowMissingOutputFailure {
        let (attempt, max_attempts, retry_scheduled) =
            Self::workflow_correction_retry_budget(context, max_turns);
        WorkflowMissingOutputFailure {
            message: MISSING_WORKFLOW_OUTPUT_MESSAGE.to_string(),
            attempt,
            max_attempts,
            retry_scheduled,
        }
    }

    fn workflow_correction_retry_budget(
        context: &WorkflowCompletionContext,
        max_turns: Option<usize>,
    ) -> (u32, u32, bool) {
        let source_node_id = context.source_node_run.node_id();
        let source_node = context
            .workflow
            .node(source_node_id)
            .expect("workflow completion context must contain its source node");
        let max_attempts = source_node
            .max_turns()
            .unwrap_or(DEFAULT_WORKFLOW_RUN_OUTPUT_MAX_ATTEMPTS)
            .min(DEFAULT_WORKFLOW_RUN_OUTPUT_MAX_ATTEMPTS)
            .max(1);
        let prior_correction_failures = context
            .workflow_run
            .failure_events()
            .iter()
            .filter(|event| {
                matches!(
                    event.kind(),
                    WorkflowFailureKind::MissingStructuredOutput
                        | WorkflowFailureKind::OutputValidationFailed
                        | WorkflowFailureKind::WorkflowRunOutputValidationFailed
                )
            })
            .filter(|event| {
                context
                    .workflow_run
                    .node_runs()
                    .iter()
                    .find(|node_run| node_run.id() == event.source_node_run_id())
                    .is_some_and(|node_run| node_run.node_id() == source_node_id)
            })
            .count() as u32;
        let attempt = prior_correction_failures.saturating_add(1);
        let node_turns = context
            .workflow_run
            .node_runs()
            .iter()
            .filter(|node_run| node_run.node_id() == source_node_id)
            .count() as u32;
        let node_budget_allows_retry = source_node
            .max_turns()
            .is_none_or(|limit| node_turns < limit);
        let workflow_budget_allows_retry = max_turns
            .filter(|limit| *limit > 0)
            .is_none_or(|limit| context.workflow_run.node_runs().len() < limit);
        (
            attempt,
            max_attempts,
            attempt < max_attempts && node_budget_allows_retry && workflow_budget_allows_retry,
        )
    }

    fn workflow_correction_dispatch(
        &mut self,
        context: &WorkflowCompletionContext,
        prompt: String,
    ) -> WorkflowDispatch {
        self.next_workflow_node_run_number += 1;
        let node_run = WorkflowNodeRun::new(
            format!("workflow-node-run-{}", self.next_workflow_node_run_number),
            context.source_node_run.node_id().to_string(),
            context.source_node_run.agent_id().to_string(),
            context
                .workflow_run
                .node_runs()
                .iter()
                .filter(|node_run| node_run.node_id() == context.source_node_run.node_id())
                .map(WorkflowNodeRun::iteration_index)
                .max()
                .unwrap_or(0)
                + 1,
            WorkflowNodeRunStatus::Ready,
        );
        WorkflowDispatch {
            node_run,
            messages: Vec::new(),
            endpoint_prompt: Some(prompt),
        }
    }

    fn workflow_run_output_correction_prompt(
        context: &WorkflowCompletionContext,
        failure: &WorkflowRunOutputValidationFailure,
    ) -> String {
        let invocation_prompt = context
            .workflow_run
            .invocation_prompt()
            .map(str::trim)
            .unwrap_or("");
        format!(
            "{invocation_prompt}\n\n{}",
            crate::prompt_assembly::render_bundled_prompt(
                crate::prompt_assembly::bundled_workflow_run_output_correction_template(),
                &[
                    ("ATTEMPT", &failure.attempt.to_string()),
                    ("MAX_ATTEMPTS", &failure.max_attempts.to_string()),
                    ("ERROR", &failure.message),
                ],
            )
        )
        .trim()
        .to_string()
    }

    fn workflow_handoff_correction_prompt(
        context: &WorkflowCompletionContext,
        failure: &WorkflowHandoffValidationFailure,
    ) -> String {
        let invocation_prompt = context
            .workflow_run
            .invocation_prompt()
            .map(str::trim)
            .unwrap_or("");
        let completion_guidance = context
            .workflow
            .node(context.source_node_run.node_id())
            .filter(|node| node.can_complete_workflow_run())
            .map(|_| {
                " If the work is accepted and the workflow should finish, do not emit an outgoing handoff. Call `validate_and_submit_workflow_run_output` with output matching the final workflow schema, and do not finish until it returns `valid: true` with no warning."
            })
            .unwrap_or("");
        format!(
            "{invocation_prompt}\n\n{}",
            crate::prompt_assembly::render_bundled_prompt(
                crate::prompt_assembly::bundled_workflow_handoff_correction_template(),
                &[
                    ("EDGE_ID", &failure.edge_id),
                    ("ATTEMPT", &failure.attempt.to_string()),
                    ("MAX_ATTEMPTS", &failure.max_attempts.to_string()),
                    ("ERROR", &failure.message),
                    ("COMPLETION_GUIDANCE", completion_guidance),
                ],
            )
        )
        .trim()
        .to_string()
    }

    fn workflow_missing_output_correction_prompt(
        context: &WorkflowCompletionContext,
        failure: &WorkflowMissingOutputFailure,
    ) -> String {
        let invocation_prompt = context
            .workflow_run
            .invocation_prompt()
            .map(str::trim)
            .unwrap_or("");
        format!(
            "{invocation_prompt}\n\n{}",
            crate::prompt_assembly::render_bundled_prompt(
                crate::prompt_assembly::bundled_workflow_missing_output_correction_template(),
                &[
                    ("ATTEMPT", &failure.attempt.to_string()),
                    ("MAX_ATTEMPTS", &failure.max_attempts.to_string()),
                ],
            )
        )
        .trim()
        .to_string()
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
