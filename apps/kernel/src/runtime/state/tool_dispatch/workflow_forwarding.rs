use super::*;

impl KernelRuntimeState {
    pub(crate) async fn dispatch_forwarded_workflow_runtime_tool_call(
        &self,
        context: crate::execution_lease::RemoteWorkflowTurnContext,
        tool_name: String,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        {
            let owned = &self.owned;
            let home_session_id = context.home_session_id.clone();
            let home_agent_id = context.home_agent_id.clone();
            let canonical_tool_name = tool_name
                .strip_prefix("chariox_")
                .unwrap_or(&tool_name)
                .to_string();
            let context = owned.workflow_tool_context(
                context.home_session_id,
                context.workflow_run_id,
                context.workflow_node_run_id,
                Some(context.delivery_token),
            )?;
            let (result, dispatches) =
                owned.dispatch_workflow_runtime_tool_call(tool_name, arguments, context)?;
            self.spawn_workflow_prompt_dispatches(dispatches);
            if forwarded_workflow_tool_result_should_complete_home_prompt(
                &canonical_tool_name,
                &result,
            ) {
                if let Some(active_prompt) = owned.prompt_state_owner.active_prompt_for_agent(
                    &owned.session_store.get_session(&home_session_id)?,
                    &home_agent_id,
                ) {
                    let completion = owned.complete_remote_prompt_owner(
                        &home_session_id,
                        &home_agent_id,
                        "remote-provider-run-completed",
                        None,
                    )?;
                    if active_prompt.workflow_run_id().is_some() {
                        let dispatches = owned.workflow_complete_prompt(
                            &home_session_id,
                            &completion.completed,
                            Some("remote-provider-run-completed"),
                        )?;
                        self.spawn_workflow_prompt_dispatches(dispatches);
                    }
                }
            }
            Ok(result)
        }
    }

    pub(crate) async fn dispatch_forwarded_workflow_provider_failure(
        &self,
        context: crate::execution_lease::RemoteWorkflowTurnContext,
        message: String,
    ) -> Result<(), DaemonError> {
        let owned = &self.owned;
        if context.home_kernel_id != owned.config_projection.snapshot().daemon_id {
            return Err(DaemonError::LocalTransport {
                operation: "forward workflow provider failure",
                message: "failure targets a different home kernel".to_string(),
            });
        }
        let session = owned.session_store.get_session(&context.home_session_id)?;
        let Some(active_prompt) = owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &context.home_agent_id)
        else {
            return Ok(());
        };
        // A worker can deliver a failure after the home has already settled
        // that turn. Acknowledge it without touching a newer active prompt.
        if active_prompt.workflow_run_id() != Some(context.workflow_run_id.as_str())
            || active_prompt.workflow_node_run_id() != Some(context.workflow_node_run_id.as_str())
        {
            return Ok(());
        }
        let (run_id, node_run_id) = owned.resolve_owned_authenticated_workflow_turn(
            &context.home_session_id,
            std::slice::from_ref(&context.home_agent_id),
            Some(&context.delivery_token),
        )?;
        if run_id != context.workflow_run_id || node_run_id != context.workflow_node_run_id {
            return Err(DaemonError::LocalTransport {
                operation: "forward workflow provider failure",
                message: "failure does not match the authenticated workflow turn".to_string(),
            });
        }
        let dispatches = owned.workflow_fail_provider_prompt(
            &context.home_session_id,
            &active_prompt,
            Some("remote-provider-run-failed"),
            &message,
        )?;
        let _ = owned.complete_remote_prompt_owner(
            &context.home_session_id,
            &context.home_agent_id,
            "remote-provider-run-failed",
            None,
        );
        self.spawn_workflow_prompt_dispatches(dispatches);
        Ok(())
    }
}
