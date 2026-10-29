use super::*;

impl KernelRuntimeState {
    pub(crate) async fn dispatch_forwarded_workflow_runtime_tool_call<R>(
        &self,
        context: crate::execution_lease::RemoteWorkflowTurnContext,
        tool_name: String,
        arguments: serde_json::Value,
        release_response: impl FnOnce(
            crate::transport::runtime_tools::RuntimeToolResult,
        ) -> Result<R, DaemonError>,
    ) -> Result<R, DaemonError> {
        let canonical_tool_name = tool_name
            .strip_prefix("chariox_")
            .unwrap_or(&tool_name)
            .to_string();
        let is_final_submission = canonical_tool_name
            == crate::transport::runtime_tools::VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL;
        let operation = || {
            self.authorize_current_forwarded_binding()?;
            let owned = &self.owned;
            let home_session_id = context.home_session_id.clone();
            let home_agent_id = context.home_agent_id.clone();
            let session = owned.session_store.get_session(&home_session_id)?;
            let receipt = owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, &home_agent_id)
                .map(|prompt| -> Result<_, DaemonError> {
                    let agent = owned.agent_store.get_agent(&home_agent_id)?;
                    let binding = agent.remote_execution().cloned().ok_or_else(|| {
                        DaemonError::LocalTransport {
                            operation: "retain forwarded workflow settlement",
                            message: "workflow agent is not remote-backed".into(),
                        }
                    })?;
                    Ok((prompt.id().to_string(), binding))
                })
                .transpose()?;
            let context = owned.workflow_tool_context(
                context.home_session_id,
                context.workflow_run_id,
                context.workflow_node_run_id,
                Some(context.delivery_token),
            )?;
            let (result, dispatches) = owned
                .dispatch_workflow_runtime_tool_call_with_prompt_settlement(
                    tool_name, arguments, context, true,
                )?;
            let complete_prompt = forwarded_workflow_tool_result_should_complete_home_prompt(
                &canonical_tool_name,
                &result,
            );
            // Release under the original strict binding/prompt gate before our
            // own completion clears that binding. Keep the app mutation lane
            // through release and settlement, including publication workflows.
            let response = release_response(result)?;
            self.spawn_workflow_prompt_dispatches(dispatches);
            if complete_prompt {
                if let Some((prompt_id, binding)) = receipt {
                    let completion = owned.complete_remote_prompt_owner_for_receipt(
                        &home_session_id,
                        &home_agent_id,
                        binding
                            .active_worker_provider_run_id
                            .as_deref()
                            .unwrap_or("remote-provider-run-completed"),
                        None,
                        None,
                        Some((&prompt_id, &binding)),
                    )?;
                    if completion.completed.workflow_run_id().is_some() {
                        let dispatches = owned.workflow_complete_prompt(
                            &home_session_id,
                            &completion.completed,
                            Some("remote-provider-run-completed"),
                        )?;
                        self.spawn_workflow_prompt_dispatches(dispatches);
                    }
                }
            }
            Ok(response)
        };
        if is_final_submission {
            self.with_app_side_effect(|_| operation()).await
        } else {
            operation()
        }
    }
}

#[cfg(test)]
mod tests;
