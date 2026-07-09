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
                .strip_prefix("arroba_")
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
                if let Some(active_prompt) =
                    owned.prompt_state_owner.active_prompt_for_agent_or_restore(
                        &owned.session_store.get_session(&home_session_id)?,
                        &home_agent_id,
                    )
                {
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
        let session = owned.session_store.get_session(&context.home_session_id)?;
        let Some(active_prompt) = owned
            .prompt_state_owner
            .active_prompt_for_agent_or_restore(&session, &context.home_agent_id)
        else {
            return Ok(());
        };
        owned.workflow_fail_provider_prompt(
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
        Ok(())
    }
}
