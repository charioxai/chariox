use super::*;

impl KernelRuntimeState {
    pub(super) async fn dispatch_authenticated_workflow_runtime_tool_call(
        &self,
        provider_runs: &[crate::provider::RuntimeProviderRun],
        canonical_tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let owned = &self.owned;
        let provider_run_allows_event_reply = provider_runs
            .iter()
            .any(|run| run.workflow_event_reply_enabled());
        let provider_run_allows_event_context = provider_runs
            .iter()
            .any(|run| run.workflow_event_context_enabled());
        // Tool discovery is advisory. A provider can retain a tool name from
        // an earlier snapshot, and an event binding can be edited while that
        // provider turn is still running. Enforce the capability snapshot at
        // dispatch time before the live binding/reply handler is consulted.
        let event_action_requested = matches!(
            canonical_tool_name,
            crate::transport::runtime_tools::REPLY_TO_EVENT_TOOL
                | crate::transport::runtime_tools::EVENT_CONTEXT_TOOL
                | crate::transport::runtime_tools::EVENT_ACTION_TOOL
        );
        let leased_event_context = if event_action_requested {
            let provider_run_ids = provider_runs
                .iter()
                .map(|run| run.id().to_string())
                .collect::<Vec<_>>();
            self.with_app_side_effect(|app| {
                let runtime = crate::app::RemoteLeaseRuntime::new(app);
                provider_run_ids.iter().find_map(|provider_run_id| {
                    runtime.leased_workflow_turn_context_for_provider_run(provider_run_id)
                })
            })
            .await
        } else {
            None
        };
        if canonical_tool_name == crate::transport::runtime_tools::REPLY_TO_EVENT_TOOL
            && !event_reply_dispatch_snapshot_allows(
                provider_run_allows_event_reply,
                leased_event_context
                    .as_ref()
                    .map(|context| context.event_reply_enabled),
            )
        {
            return Err(DaemonError::LocalTransport {
                operation: "dispatch_authenticated_workflow_runtime_tool_call",
                message: "reply_to_event is not enabled for the active workflow provider run"
                    .to_string(),
            });
        }
        if canonical_tool_name == crate::transport::runtime_tools::EVENT_CONTEXT_TOOL
            && !event_reply_dispatch_snapshot_allows(
                provider_run_allows_event_context,
                leased_event_context
                    .as_ref()
                    .map(|context| context.event_context_enabled),
            )
        {
            return Err(DaemonError::LocalTransport {
                operation: "dispatch_authenticated_workflow_runtime_tool_call",
                message: "event_context is not enabled for the active workflow provider run"
                    .to_string(),
            });
        }
        if canonical_tool_name == crate::transport::runtime_tools::EVENT_ACTION_TOOL
            && !event_reply_dispatch_snapshot_allows(
                provider_run_allows_event_context,
                leased_event_context
                    .as_ref()
                    .map(|context| context.event_context_enabled),
            )
        {
            return Err(DaemonError::LocalTransport {
                operation: "dispatch_authenticated_workflow_runtime_tool_call",
                message: "event_action is not enabled for the active workflow provider run"
                    .to_string(),
            });
        }
        let requested_delivery_token = match canonical_tool_name {
            crate::transport::runtime_tools::ACK_WORKFLOW_TURN_TOOL => {
                serde_json::from_value::<crate::transport::runtime_tools::AckWorkflowTurnArgs>(
                    arguments.clone(),
                )
                .ok()
                .map(|args| args.delivery_token)
            }
            crate::transport::runtime_tools::VALIDATE_WORKFLOW_HANDOFF_TOOL => {
                serde_json::from_value::<crate::transport::runtime_tools::ValidateWorkflowHandoffArgs>(
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
            crate::transport::runtime_tools::READ_WORKFLOW_TURN_CONTEXT_TOOL => {
                serde_json::from_value::<
                    crate::transport::runtime_tools::ReadWorkflowTurnContextArgs,
                >(arguments.clone())
                .ok()
                .and_then(|args| args.delivery_token)
            }
            crate::transport::runtime_tools::AGENT_APP_ACTION_TOOL => serde_json::from_value::<
                crate::transport::runtime_tools::AgentAppActionArgs,
            >(arguments.clone())
            .ok()
            .and_then(|args| args.delivery_token),
            crate::transport::runtime_tools::REPLY_TO_EVENT_TOOL => serde_json::from_value::<
                crate::transport::runtime_tools::ReplyToEventArgs,
            >(arguments.clone())
            .ok()
            .and_then(|args| args.delivery_token),
            crate::transport::runtime_tools::EVENT_CONTEXT_TOOL => serde_json::from_value::<
                crate::transport::runtime_tools::EventContextArgs,
            >(arguments.clone())
            .ok()
            .and_then(|args| args.delivery_token),
            crate::transport::runtime_tools::EVENT_ACTION_TOOL => serde_json::from_value::<
                crate::transport::runtime_tools::EventActionArgs,
            >(arguments.clone())
            .ok()
            .and_then(|args| args.delivery_token),
            _ => None,
        };
        let session_id = provider_runs[0].session_id().to_string();
        let candidate_agent_ids = provider_runs
            .iter()
            .filter_map(|run| run.agent_instance_id().map(str::to_string))
            .collect::<Vec<_>>();
        match owned.resolve_owned_authenticated_workflow_turn(
            &session_id,
            &candidate_agent_ids,
            requested_delivery_token.as_deref(),
        ) {
            Ok((workflow_run_ref, workflow_node_run_id)) => {
                let context = owned.workflow_tool_context(
                    session_id,
                    workflow_run_ref,
                    workflow_node_run_id,
                    requested_delivery_token,
                )?;
                if event_action_requested {
                    crate::runtime::event_catalog_control::ensure_event_generator_management_target_for_workflow_run(
                        self,
                        &owned.config_projection,
                        &context.session_id,
                        &context.workflow_run_ref,
                    )
                    .await?;
                }
                let (result, dispatches) = owned.dispatch_workflow_runtime_tool_call(
                    canonical_tool_name.to_string(),
                    arguments,
                    context,
                )?;
                self.spawn_workflow_prompt_dispatches(dispatches);
                Ok(result)
            }
            Err(error)
                if canonical_tool_name
                    == crate::transport::runtime_tools::READ_WORKFLOW_TURN_CONTEXT_TOOL =>
            {
                Err(error)
            }
            Err(local_error) => {
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
                let Some(context) = leased_event_context.or(leased_workflow_context) else {
                    return Err(local_error);
                };
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
                match response {
                    RelayPeerResponse::WorkflowRuntimeToolHandled { result } => {
                        if leased_workflow_tool_result_should_complete_turn(
                            canonical_tool_name,
                            &result,
                        ) {
                            self.with_app_side_effect(|app| {
                                let mut runtime = crate::app::RemoteLeaseRuntime::new(app);
                                for provider_run_id in &provider_run_ids {
                                    if runtime
                                        .leased_workflow_turn_context_for_provider_run(
                                            provider_run_id,
                                        )
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
                }
            }
        }
    }
}

fn event_reply_dispatch_snapshot_allows(
    owned_provider_run_enabled: bool,
    leased_binding_enabled: Option<bool>,
) -> bool {
    owned_provider_run_enabled || leased_binding_enabled == Some(true)
}

#[cfg(test)]
mod tests {
    use super::event_reply_dispatch_snapshot_allows;

    #[test]
    fn event_action_dispatch_requires_the_provider_or_lease_snapshot() {
        assert!(event_reply_dispatch_snapshot_allows(true, None));
        assert!(event_reply_dispatch_snapshot_allows(false, Some(true)));
        assert!(!event_reply_dispatch_snapshot_allows(false, Some(false)));
        assert!(!event_reply_dispatch_snapshot_allows(false, None));
    }
}
