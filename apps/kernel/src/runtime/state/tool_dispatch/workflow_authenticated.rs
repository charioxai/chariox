use super::*;

impl KernelRuntimeState {
    pub(super) async fn dispatch_authenticated_workflow_runtime_tool_call(
        &self,
        provider_runs: &[crate::provider::RuntimeProviderRun],
        canonical_tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let owned = &self.owned;
        let provider_run_ids = provider_runs
            .iter()
            .map(|run| run.id().to_string())
            .collect::<Vec<_>>();
        let leased_workflow_context = self
            .with_app_side_effect(|app| {
                let runtime = crate::app::RemoteLeaseRuntime::new(app);
                provider_run_ids.iter().find_map(|provider_run_id| {
                    runtime.leased_workflow_turn_context_for_provider_run(provider_run_id)
                })
            })
            .await;
        if let Some(context) = leased_workflow_context {
            let response = self
                .with_app_side_effect(|app| {
                    app.block_on_relay_future(
                        crate::transport::relay_client::send_peer_request_via_temporary_connection(
                            app.config(),
                            ClientTarget {
                                daemon_id: Some(context.home_kernel_id.clone()),
                                daemon_alias: None,
                            },
                            RelayPeerRequest::ForwardWorkflowRuntimeTool {
                                context: context.clone(),
                                tool_name: canonical_tool_name.to_string(),
                                arguments: arguments.clone(),
                            },
                        ),
                    )
                })
                .await?;
            return match response {
                RelayPeerResponse::WorkflowRuntimeToolHandled { result } => {
                    if leased_workflow_tool_result_should_complete_turn(
                        canonical_tool_name,
                        &result,
                    ) {
                        self.with_app_side_effect(|app| {
                            let mut runtime = crate::app::RemoteLeaseRuntime::new(app);
                            for provider_run_id in &provider_run_ids {
                                if runtime
                                    .leased_workflow_turn_context_for_provider_run(provider_run_id)
                                    .is_some()
                                {
                                    let _ = runtime
                                        .complete_leased_workflow_prompt_for_provider_run(
                                            provider_run_id,
                                        )?;
                                    break;
                                }
                            }
                            Ok(())
                        })
                        .await?;
                    }
                    Ok(result)
                }
                other => Err(DaemonError::LocalTransport {
                    operation: "forward leased workflow runtime tool",
                    message: format!("unexpected forwarded workflow tool response: {other:?}"),
                }),
            };
        }
        let requested_delivery_token = match canonical_tool_name {
            crate::transport::runtime_tools::ACK_WORKFLOW_TURN_TOOL => {
                serde_json::from_value::<crate::transport::runtime_tools::AckWorkflowTurnArgs>(
                    arguments.clone(),
                )
                .ok()
                .map(|args| args.delivery_token)
            }
            crate::transport::runtime_tools::VALIDATE_WORKFLOW_OUTPUT_TOOL => {
                serde_json::from_value::<crate::transport::runtime_tools::ValidateWorkflowOutputArgs>(
                    arguments.clone(),
                )
                .ok()
                .and_then(|args| args.delivery_token)
            }
            crate::transport::runtime_tools::VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL
            | crate::transport::runtime_tools::VALIDATE_AND_SUBMIT_INTERMEDIATE_WORKFLOW_RUN_OUTPUT_TOOL => {
                serde_json::from_value::<
                    crate::transport::runtime_tools::ValidateAndSubmitWorkflowRunOutputArgs,
                >(arguments.clone())
                .ok()
                .and_then(|args| args.delivery_token)
            }
            _ => None,
        };
        let session_id = provider_runs[0].session_id().to_string();
        let candidate_agent_ids = provider_runs
            .iter()
            .filter_map(|run| run.agent_instance_id().map(str::to_string))
            .collect::<Vec<_>>();
        let (workflow_run_ref, workflow_node_run_id) = owned
            .resolve_owned_authenticated_workflow_turn(
                &session_id,
                &candidate_agent_ids,
                requested_delivery_token.as_deref(),
            )?;
        let context = owned.workflow_tool_context(
            session_id,
            workflow_run_ref,
            workflow_node_run_id,
            None,
        )?;
        let (result, dispatches) = owned.dispatch_workflow_runtime_tool_call(
            canonical_tool_name.to_string(),
            arguments,
            context,
        )?;
        self.spawn_workflow_prompt_dispatches(dispatches);
        Ok(result)
    }
}
