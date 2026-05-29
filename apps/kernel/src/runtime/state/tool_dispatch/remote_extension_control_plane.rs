use arroba_relay::protocol::ClientTarget;

use crate::error::DaemonError;
use crate::runtime::state::KernelRuntimeState;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};

use super::*;

impl KernelRuntimeState {
    pub(super) async fn try_dispatch_remote_capability_runtime_tool_call(
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
        let response = self
            .with_app_side_effect(|app| {
                app.block_on_relay_future(
                    crate::transport::relay_client::send_peer_request_via_temporary_connection(
                        app.config(),
                        ClientTarget {
                            daemon_id: Some(remote_context.home_kernel_id.clone()),
                            daemon_alias: None,
                        },
                        RelayPeerRequest::ForwardCapabilityRuntimeTool {
                            context: remote_context.clone(),
                            tool_name: tool_name.to_string(),
                            arguments: arguments.clone(),
                        },
                    ),
                )
            })
            .await?;
        let (result, skill_package, remote_extension_manifest) = match response {
            RelayPeerResponse::CapabilityRuntimeToolHandled {
                result,
                skill_package,
                remote_extension_manifest,
            } => (result, skill_package, remote_extension_manifest),
            other => {
                return Err(DaemonError::LocalTransport {
                    operation: "forward leased capability runtime tool",
                    message: format!("unexpected forwarded capability response: {other:?}"),
                });
            }
        };
        if !remote_extension_manifest.is_empty() {
            let updated = self
                .owned
                .provider_store
                .update_run_remote_extension_manifest(
                    provider_run.id(),
                    remote_extension_manifest,
                )?;
            self.owned.provider_run_projection.update(updated);
        }
        let result = self.apply_remote_skill_package_response(
            &workspace_context.root,
            &remote_context.home_kernel_id,
            result,
            skill_package,
        )?;
        Ok(Some(result))
    }

    pub(crate) async fn dispatch_forwarded_capability_runtime_tool_call(
        &self,
        context: crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
        tool_name: String,
        arguments: serde_json::Value,
    ) -> Result<
        (
            crate::transport::runtime_tools::RuntimeToolResult,
            Option<crate::skill::ArrobaSkillPackage>,
            crate::extension::RemoteExtensionManifest,
        ),
        DaemonError,
    > {
        let (result, package) = self
            .dispatch_capability_runtime_tool_call_for_agent(
                &context.home_session_id,
                &context.home_agent_id,
                &tool_name,
                arguments,
                true,
            )
            .await?;
        let agent = self.owned.agent_store.get_agent(&context.home_agent_id)?;
        let manifest = self.remote_extension_manifest_for_agent(&agent)?;
        Ok((result, package, manifest))
    }

    pub(super) async fn dispatch_capability_runtime_tool_call(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let Some(agent_id) = provider_run.agent_instance_id().map(str::to_string) else {
            return Ok(crate::transport::runtime_tools::RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "error": "capability tools require an agent-scoped provider run"
                }),
            });
        };
        let session_id = provider_run.session_id().to_string();
        let (result, _) = self
            .dispatch_capability_runtime_tool_call_for_agent(
                &session_id,
                &agent_id,
                tool_name,
                arguments,
                false,
            )
            .await?;
        Ok(result)
    }

    async fn dispatch_capability_runtime_tool_call_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        tool_name: &str,
        arguments: serde_json::Value,
        include_skill_package: bool,
    ) -> Result<
        (
            crate::transport::runtime_tools::RuntimeToolResult,
            Option<crate::skill::ArrobaSkillPackage>,
        ),
        DaemonError,
    > {
        let session = self.owned.session_store.get_session(session_id)?;
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        match tool_name {
            crate::transport::runtime_tools::LIST_EXTENSIONS_TOOL => {
                self.handle_list_extensions_runtime_tool(&session, &agent, arguments)
            }
            crate::transport::runtime_tools::REQUEST_EXTENSION_TOOL => {
                self.handle_request_extension_runtime_tool(
                    &session,
                    &agent,
                    session_id,
                    arguments,
                    include_skill_package,
                )
                .await
            }
            _ => Err(DaemonError::LocalTransport {
                operation: "dispatch_capability_runtime_tool_call",
                message: format!("unknown capability runtime tool `{tool_name}`"),
            }),
        }
    }
}
