//! Workflow handoff validation and submission runtime-tool handlers.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn workflow_validate_handoff_tool_result(
        &self,
        arguments: &serde_json::Value,
        context: &crate::transport::runtime_tools::WorkflowRuntimeToolContext,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let args = serde_json::from_value::<
            crate::transport::runtime_tools::ValidateWorkflowHandoffArgs,
        >(arguments.clone())
        .map_err(|error| DaemonError::LocalTransport {
            operation: "runtime_tool_validate_workflow_handoff",
            message: format!("invalid tool arguments: {error}"),
        })?;
        if !context.allowed_handoff_schema_refs.is_empty()
            && !context
                .allowed_handoff_schema_refs
                .iter()
                .any(|schema_ref| schema_ref == &args.handoff_schema_ref)
        {
            return Err(DaemonError::LocalTransport {
                operation: "runtime_tool_validate_workflow_handoff",
                message: format!(
                    "schema ref `{}` is not allowed for workflow node run `{}`",
                    args.handoff_schema_ref, context.workflow_node_run_id
                ),
            });
        }
        let warning = self
            .validate_workflow_runtime_schema_ref(
                context,
                &args.handoff_schema_ref,
                &args.handoff_json,
            )
            .err();
        Ok(crate::transport::runtime_tools::RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "valid": warning.is_none(),
                "warning": warning,
                "next_action": if warning.is_none() {
                    "Validation passed. Now finish this same workflow turn by emitting exactly one final fenced json block and then stop."
                } else {
                    "Validation failed or warned. Revise the output and call validate_workflow_handoff again before finalizing."
                },
            }),
        })
    }

    pub(super) fn workflow_submit_output_tool_result(
        &self,
        arguments: &serde_json::Value,
        context: &crate::transport::runtime_tools::WorkflowRuntimeToolContext,
        is_final: bool,
    ) -> Result<
        (
            crate::transport::runtime_tools::RuntimeToolResult,
            WorkflowPromptDispatches,
        ),
        DaemonError,
    > {
        if is_final && !context.can_complete_workflow_run {
            return Err(DaemonError::LocalTransport {
                operation: "runtime_tool_validate_and_submit_workflow_run_output",
                message: "current workflow node run is not allowed to complete the workflow run"
                    .to_string(),
            });
        }
        if !is_final && !context.can_emit_intermediate_workflow_run_output {
            return Err(DaemonError::LocalTransport {
                operation: "runtime_tool_validate_and_submit_intermediate_workflow_run_output",
                message:
                    "current workflow node run is not allowed to emit intermediate workflow run output"
                        .to_string(),
            });
        }
        let args = serde_json::from_value::<
            crate::transport::runtime_tools::ValidateAndSubmitWorkflowRunOutputArgs,
        >(arguments.clone())
        .map_err(|error| DaemonError::LocalTransport {
            operation: if is_final {
                "runtime_tool_validate_and_submit_workflow_run_output"
            } else {
                "runtime_tool_validate_and_submit_intermediate_workflow_run_output"
            },
            message: format!("invalid tool arguments: {error}"),
        })?;
        let workflow_run_id = self
            .session_store
            .read()
            .resolve_workflow_run_ref(&context.session_id, &context.workflow_run_ref)?
            .id()
            .to_string();
        let schema_ref = if is_final {
            context.workflow_run_output_schema_ref.as_deref()
        } else {
            context.workflow_intermediate_output_schema_ref.as_deref()
        };
        let warning = schema_ref.and_then(|schema_ref| {
            self.validate_workflow_runtime_schema_ref(
                context,
                schema_ref,
                &args.workflow_output_json,
            )
            .err()
        });
        let workflow_output_json = args.workflow_output_json.clone();
        let output = crate::session::WorkflowOutputPayload::new(
            workflow_output_json.clone(),
            Vec::<crate::session::WorkflowArtifactRef>::new(),
        );
        let workflow_run = if is_final {
            self.session_store
                .write()
                .submit_workflow_run_final_output(
                    &context.session_id,
                    &workflow_run_id,
                    &context.workflow_node_run_id,
                    output,
                    warning.is_none(),
                    warning.clone(),
                )?
        } else {
            self.session_store
                .write()
                .submit_workflow_run_intermediate_output(
                    &context.session_id,
                    &workflow_run_id,
                    &context.workflow_node_run_id,
                    output,
                    warning.is_none(),
                    warning.clone(),
                )?
        };
        let mut dispatches = WorkflowPromptDispatches::default();
        let event_kind = if is_final {
            "workflow.output.final"
        } else {
            "workflow.output.intermediate"
        };
        let output_preview = workflow_output_json.chars().take(500).collect::<String>();
        let source_agent_id = workflow_run
            .node_runs()
            .iter()
            .find(|node_run| node_run.id() == context.workflow_node_run_id)
            .map(|node_run| node_run.agent_id().to_string());
        let source_attachment_id =
            crate::scheduler::runtime::workflow_prompt_source_attachment_id(&workflow_run_id);
        dispatches.extend(self.metaagent_workflow_event_prompt_dispatches(
            &context.session_id,
            event_kind,
            source_agent_id.as_deref(),
            &source_attachment_id,
            if is_final {
                format!("Workflow run `{workflow_run_id}` submitted final output")
            } else {
                format!("Workflow run `{workflow_run_id}` submitted intermediate output")
            },
            if output_preview.trim().is_empty() {
                format!("Workflow run `{workflow_run_id}` submitted an empty output.")
            } else {
                format!(
                    "Workflow run `{workflow_run_id}` submitted output: {}",
                    output_preview.trim()
                )
            },
            serde_json::json!({
                "workflow_run_id": workflow_run.id(),
                "workflow_node_run_id": context.workflow_node_run_id,
                "kind": event_kind,
                "valid": warning.is_none(),
                "warning": warning.clone(),
                "output": workflow_output_json,
            }),
        ));
        if is_final && warning.is_none() && workflow_run.publication_invocation().is_some() {
            let max_turns = self.workflow_max_turns(&context.session_id);
            let update = self.session_store.write().complete_workflow_node_run(
                &context.session_id,
                &workflow_run_id,
                &context.workflow_node_run_id,
                None,
                max_turns,
            )?;
            dispatches.extend(self.workflow_prepare_dispatches(
                &context.session_id,
                &workflow_run_id,
                &update.dispatches,
            ));
            if !self.workflow_publication_prompt_waits_for_provider_completion(
                &context.session_id,
                &workflow_run_id,
                &context.workflow_node_run_id,
            )? {
                dispatches.extend(self.workflow_settle_completed_publication_prompt(
                    &context.session_id,
                    &workflow_run_id,
                    &context.workflow_node_run_id,
                )?);
            }
        }
        if !is_final && warning.is_none() {
            self.session_store
                .write()
                .record_workflow_intermediate_output_event(
                    &context.session_id,
                    &workflow_run_id,
                    &context.workflow_node_run_id,
                )?;
        }
        let output_valid = warning.is_none();
        Ok((
            crate::transport::runtime_tools::RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({
                    "submitted": true,
                    "valid": output_valid,
                    "warning": warning,
                    "workflow_run_id": workflow_run.id(),
                    "workflow_node_run_id": context.workflow_node_run_id,
                    "next_action": workflow_output_submission_next_action(
                        is_final,
                        output_valid,
                    ),
                }),
            },
            dispatches,
        ))
    }

    fn validate_workflow_runtime_schema_ref(
        &self,
        context: &crate::transport::runtime_tools::WorkflowRuntimeToolContext,
        schema_ref: &str,
        output_json: &str,
    ) -> Result<(), String> {
        let workflow_schema = {
            let store = self.session_store.read();
            store
                .resolve_workflow_run_ref(&context.session_id, &context.workflow_run_ref)
                .ok()
                .and_then(|workflow_run| {
                    store
                        .resolve_workflow_ref(&context.session_id, workflow_run.workflow_id())
                        .ok()
                })
                .and_then(|workflow| {
                    workflow
                        .schema(schema_ref)
                        .map(|schema| schema.schema().clone())
                })
        };
        if let Some(schema) = workflow_schema {
            crate::transport::runtime_tools::validate_json_output_schema(
                schema_ref,
                &schema,
                output_json,
            )
        } else {
            crate::transport::runtime_tools::validate_workflow_handoff_schema(
                schema_ref,
                output_json,
            )
        }
    }

    fn workflow_publication_prompt_waits_for_provider_completion(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> Result<bool, DaemonError> {
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_id)?;
        let Some(node_run) = workflow_run
            .node_runs()
            .iter()
            .find(|node_run| node_run.id() == workflow_node_run_id)
        else {
            return Ok(false);
        };
        let Some(run) = self
            .provider_store
            .get_run_for_agent(session_id, node_run.agent_id())
        else {
            return Ok(false);
        };
        if !crate::provider::provider_run_waits_for_workflow_publication_completion(&run) {
            return Ok(false);
        }
        let agent_id = node_run.agent_id().to_string();
        let session = self.session_store.get_session(session_id)?;
        let active_prompt_matches = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
            .is_some_and(|prompt| {
                prompt.workflow_run_id() == Some(workflow_run_id)
                    && prompt.workflow_node_run_id() == Some(workflow_node_run_id)
            });
        if !active_prompt_matches {
            return Ok(false);
        }
        Ok(true)
    }

    fn workflow_settle_completed_publication_prompt(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> Result<WorkflowPromptDispatches, DaemonError> {
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_id)?;
        if workflow_run.publication_invocation().is_none()
            || !matches!(
                workflow_run.status(),
                crate::session::WorkflowRunStatus::Completed
                    | crate::session::WorkflowRunStatus::Failed
                    | crate::session::WorkflowRunStatus::Stopped
            )
        {
            return Ok(WorkflowPromptDispatches::default());
        }
        let Some(node_run) = workflow_run
            .node_runs()
            .iter()
            .find(|node_run| node_run.id() == workflow_node_run_id)
        else {
            return Ok(WorkflowPromptDispatches::default());
        };
        let agent_id = node_run.agent_id().to_string();
        let session = self.session_store.get_session(session_id)?;
        let Some(active_prompt) = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
        else {
            return Ok(WorkflowPromptDispatches::default());
        };
        if active_prompt.workflow_run_id() != Some(workflow_run_id)
            || active_prompt.workflow_node_run_id() != Some(workflow_node_run_id)
        {
            return Ok(WorkflowPromptDispatches::default());
        }
        let provider_run_id = self
            .provider_store
            .get_run_for_agent(session_id, &agent_id)
            .map(|run| run.id().to_string());
        let next_queued_prompt = provider_run_id
            .as_deref()
            .and_then(|provider_run_id| self.provider_store.get_run(provider_run_id).ok())
            .filter(|run| run.state() == crate::provider::ProviderRunState::Running)
            .and_then(|_| {
                self.prompt_state_owner
                    .peek_next_queued_prompt(&session, &agent_id)
            });
        let completion = if let Some(next_queued_prompt) = next_queued_prompt.as_ref() {
            self.complete_local_prompt_with_queued_advance(
                session_id,
                &agent_id,
                provider_run_id.as_deref(),
                next_queued_prompt,
            )?
        } else {
            self.complete_local_prompt_without_advance(
                session_id,
                &agent_id,
                provider_run_id.as_deref(),
            )?
        };
        let mut dispatches = WorkflowPromptDispatches::default();
        if let Some(mut completion) = completion {
            if let Some(dispatch) = completion.dispatch.take() {
                dispatches.local.push(dispatch);
            }
            if completion.released_claim {
                dispatches.extend(self.workflow_retry_blocked_claims());
            }
        }
        dispatches.extend(self.workflow_maybe_start_next_queued_prompt(session_id));
        Ok(dispatches)
    }
}

fn workflow_output_submission_next_action(is_final: bool, valid: bool) -> &'static str {
    match (is_final, valid) {
        (true, true) => {
            "Final workflow run output was submitted and validated. Finish this same workflow turn now."
        }
        (true, false) => {
            "Final workflow run output failed validation. Revise it and call validate_and_submit_workflow_run_output again. Do not finish this turn until the tool returns valid: true with no warning."
        }
        (false, _) => {
            "Intermediate workflow run output was submitted as a user-visible event. Continue this same workflow turn. You may submit more intermediate outputs if useful. Before stopping, either emit the required final fenced JSON handoff or submit final workflow run output if authorized."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::workflow_output_submission_next_action;

    #[test]
    fn invalid_final_output_submission_requires_revision() {
        let action = workflow_output_submission_next_action(true, false);
        assert!(action.contains("failed validation"));
        assert!(action.contains("call validate_and_submit_workflow_run_output again"));
        assert!(action.contains("Do not finish"));
    }

    #[test]
    fn valid_final_output_submission_allows_turn_completion() {
        let action = workflow_output_submission_next_action(true, true);
        assert!(action.contains("submitted and validated"));
        assert!(action.contains("Finish this same workflow turn"));
    }
}
