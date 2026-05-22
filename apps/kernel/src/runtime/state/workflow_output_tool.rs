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
        let warning = crate::transport::runtime_tools::validate_workflow_handoff_schema(
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
            crate::transport::runtime_tools::validate_workflow_handoff_schema(
                schema_ref,
                &args.workflow_output_json,
            )
            .err()
        });
        let output = crate::session::WorkflowOutputPayload::new(
            args.workflow_output_json,
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
        if !is_final && warning.is_none() {
            let update = self
                .session_store
                .write()
                .release_workflow_intermediate_output_downstream(
                    &context.session_id,
                    &workflow_run_id,
                    &context.workflow_node_run_id,
                )?;
            for warning in &update.validation_warnings {
                self.workflow_record_failure(
                    &context.session_id,
                    &workflow_run_id,
                    &crate::session::WorkflowFailureEvent::new(
                        crate::session::WorkflowFailureKind::OutputValidationFailed,
                        &context.workflow_node_run_id,
                        vec![warning.edge_id.clone()],
                        warning.message.clone(),
                    ),
                );
                self.record_notice(
                    &context.session_id,
                    None,
                    self.attachment_store
                        .list_session_attachment_ids(&context.session_id),
                    format!(
                        "Workflow handoff validation warning on edge `{}`: {}",
                        warning.edge_id, warning.message
                    ),
                );
            }
            dispatches.extend(self.workflow_prepare_dispatches(
                &context.session_id,
                &workflow_run_id,
                &update.dispatches,
            ));
            let _ = self.session_snapshot(&context.session_id);
        }
        Ok((
            crate::transport::runtime_tools::RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({
                    "submitted": true,
                    "valid": warning.is_none(),
                    "warning": warning,
                    "workflow_run_id": workflow_run.id(),
                    "workflow_node_run_id": context.workflow_node_run_id,
                    "next_action": if is_final {
                        "Final workflow run output was submitted. If it is valid with no warning, finish this same workflow turn now."
                    } else {
                        "Intermediate workflow run output was submitted. Continue this same workflow turn and emit the required final fenced json block before stopping."
                    },
                }),
            },
            dispatches,
        ))
    }
}
