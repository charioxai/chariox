//! Outbound workspace live sync forwarding from a leased worker to the home kernel.

use super::super::*;

impl KernelRuntimeState {
    pub(in crate::runtime::state::tool_dispatch) async fn try_dispatch_remote_workspace_live_sync_runtime_tool_call(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<Option<crate::transport::runtime_tools::RuntimeToolResult>, DaemonError> {
        let workspace_context = self
            .workspace_live_sync_workspace_for_provider_run(provider_run)
            .await?;
        let remote_context = self
            .with_app_side_effect(|app| {
                crate::app::RemoteLeaseRuntime::new(app)
                    .leased_workspace_live_sync_context_for_provider_run(
                        provider_run.id(),
                        workspace_context.identity.clone(),
                    )
            })
            .await;
        let Some(remote_context) = remote_context else {
            return Ok(None);
        };
        if !workspace_context.valid {
            return Ok(Some(workspace_live_sync_workspace_identity_rejected(
                &workspace_context,
            )));
        }
        let artifact_states = remote_workspace_live_sync_artifact_states_for_tool(
            &workspace_context.root,
            tool_name,
            &arguments,
        )?;
        let response = self
            .with_app_side_effect(|app| {
                app.block_on_relay_future(
                    crate::transport::relay_client::send_peer_request_via_temporary_connection(
                        app.config(),
                        ClientTarget {
                            daemon_id: Some(remote_context.home_kernel_id.clone()),
                            daemon_alias: None,
                        },
                        RelayPeerRequest::ForwardWorkspaceLiveSyncRuntimeTool {
                            context: remote_context.clone(),
                            tool_name: tool_name.to_string(),
                            arguments: arguments.clone(),
                            artifact_states: artifact_states.clone(),
                        },
                    ),
                )
            })
            .await?;
        let (mut result, final_states) = match response {
            RelayPeerResponse::WorkspaceLiveSyncRuntimeToolHandled {
                result,
                final_artifact_states,
            } => (result, final_artifact_states),
            other => {
                return Err(DaemonError::LocalTransport {
                    operation: "forward leased workspace live sync runtime tool",
                    message: format!(
                        "unexpected forwarded workspace live sync response: {other:?}"
                    ),
                });
            }
        };
        if result.ok && !final_states.is_empty() {
            if let Some(rejection) = apply_remote_workspace_live_sync_final_states(
                &workspace_context.root,
                &artifact_states,
                &final_states,
            )? {
                result = rejection;
            } else {
                let finalize_response = self
                    .with_app_side_effect(|app| {
                        app.block_on_relay_future(
                            crate::transport::relay_client::send_peer_request_via_temporary_connection(
                                app.config(),
                                ClientTarget {
                                    daemon_id: Some(remote_context.home_kernel_id.clone()),
                                    daemon_alias: None,
                                },
                                RelayPeerRequest::FinalizeWorkspaceLiveSyncRuntimeTool {
                                    context: remote_context.clone(),
                                    tool_name: tool_name.to_string(),
                                    arguments: arguments.clone(),
                                    initial_artifact_states: artifact_states.clone(),
                                    final_artifact_states: final_states.clone(),
                                },
                            ),
                        )
                    })
                    .await?;
                if !matches!(
                    finalize_response,
                    RelayPeerResponse::WorkspaceLiveSyncRuntimeToolFinalized
                ) {
                    return Err(DaemonError::LocalTransport {
                        operation: "finalize leased workspace live sync runtime tool",
                        message: format!(
                            "unexpected forwarded workspace live sync finalize response: {finalize_response:?}"
                        ),
                    });
                }
            }
        }
        add_workspace_live_sync_workspace_payload(&mut result.payload, &workspace_context);
        Ok(Some(result))
    }
}
