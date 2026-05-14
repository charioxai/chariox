//! Outbound managed-I/O forwarding from a leased worker to the home kernel.

use super::super::*;

impl KernelRuntimeState {
    pub(in crate::runtime::state::tool_dispatch) async fn try_dispatch_remote_managed_io_runtime_tool_call(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<Option<crate::transport::runtime_tools::RuntimeToolResult>, DaemonError> {
        let workspace_context = self
            .managed_io_workspace_for_provider_run(provider_run)
            .await?;
        let remote_context = self
            .with_app_side_effect(|app| {
                crate::app::RemoteLeaseRuntime::new(app).leased_managed_io_context_for_provider_run(
                    provider_run.id(),
                    workspace_context.identity.clone(),
                )
            })
            .await;
        let Some(remote_context) = remote_context else {
            return Ok(None);
        };
        if !workspace_context.valid {
            return Ok(Some(managed_io_workspace_identity_rejected(
                &workspace_context,
            )));
        }
        let artifact_states = remote_managed_io_artifact_states_for_tool(
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
                        RelayPeerRequest::ForwardManagedIoRuntimeTool {
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
            RelayPeerResponse::ManagedIoRuntimeToolHandled {
                result,
                final_artifact_states,
            } => (result, final_artifact_states),
            other => {
                return Err(DaemonError::LocalTransport {
                    operation: "forward leased managed I/O runtime tool",
                    message: format!("unexpected forwarded managed I/O response: {other:?}"),
                });
            }
        };
        if result.ok && !final_states.is_empty() {
            if let Some(rejection) = apply_remote_managed_io_final_states(
                &workspace_context.root,
                &artifact_states,
                &final_states,
            )? {
                result = rejection;
            }
        }
        add_managed_io_workspace_payload(&mut result.payload, &workspace_context);
        Ok(Some(result))
    }
}
