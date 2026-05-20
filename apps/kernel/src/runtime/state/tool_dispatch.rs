//! Runtime MCP tool dispatch.
//!
//! Provider tool calls enter here and are routed to managed-I/O handlers or other runtime-owned
//! tool surfaces with consistent authorization and JSON payload shaping.

use super::*;

mod capability;
mod capability_registry;
mod connector;
mod credential;
mod managed_io_access;
mod managed_io_local;
mod managed_io_permission;
mod managed_io_remote_dispatch;
mod remote_capability_sync;
mod script;
mod slice;
mod workflow_authenticated;
mod workflow_forwarding;

impl KernelRuntimeState {
    pub(crate) fn runtime_tool_specs_for_auth_token(
        &self,
        auth_token: &str,
    ) -> Vec<crate::transport::runtime_tools::RuntimeToolSpec> {
        let mut specs = crate::transport::runtime_tools::managed_io_runtime_tool_specs()
            .into_iter()
            .chain(crate::transport::runtime_tools::extension_runtime_tool_specs())
            .chain(self.script_runtime_tool_specs_for_auth_token(auth_token))
            .chain(self.connector_runtime_tool_specs_for_auth_token(auth_token))
            .chain(crate::transport::runtime_tools::credential_runtime_tool_specs())
            .chain(crate::transport::runtime_tools::workflow_runtime_tool_specs())
            .collect::<Vec<_>>();
        if self.runtime_auth_token_has_active_provider_run(auth_token)
            && self.slice_kernel_id().is_some()
        {
            specs.extend(crate::transport::runtime_tools::slice_runtime_tool_specs());
        }
        specs
    }

    pub(crate) async fn dispatch_authenticated_runtime_tool_call(
        &self,
        auth_token: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        {
            let owned = &self.owned;
            let canonical_tool_name =
                crate::transport::runtime_tools::canonical_managed_io_tool_name(tool_name)
                    .or_else(|| {
                        crate::transport::runtime_tools::canonical_extension_tool_name(tool_name)
                    })
                    .or_else(|| {
                        crate::transport::runtime_tools::canonical_credential_tool_name(tool_name)
                    })
                    .or_else(|| {
                        crate::transport::runtime_tools::canonical_slice_tool_name(tool_name)
                    })
                    .unwrap_or_else(|| tool_name.strip_prefix("arroba_").unwrap_or(tool_name));
            let provider_runs = owned
                .provider_store
                .get_runs_by_runtime_mcp_auth_token(auth_token);
            if provider_runs.is_empty() {
                return Err(DaemonError::LocalTransport {
                    operation: "dispatch_authenticated_runtime_tool_call",
                    message: "invalid runtime MCP auth token".to_string(),
                });
            }
            if matches!(
                canonical_tool_name,
                crate::transport::runtime_tools::READ_ARTIFACT_TOOL
                    | crate::transport::runtime_tools::EDIT_ARTIFACT_TOOL
                    | crate::transport::runtime_tools::APPLY_PATCH_TOOL
                    | crate::transport::runtime_tools::DELETE_ARTIFACT_TOOL
                    | crate::transport::runtime_tools::MOVE_ARTIFACT_TOOL
                    | crate::transport::runtime_tools::WRITE_ARTIFACT_TOOL
            ) {
                if let Some(result) = self
                    .try_dispatch_remote_managed_io_runtime_tool_call(
                        &provider_runs[0],
                        canonical_tool_name,
                        arguments.clone(),
                    )
                    .await?
                {
                    return Ok(result);
                }
                return self
                    .dispatch_managed_io_runtime_tool_call(
                        &provider_runs[0],
                        canonical_tool_name,
                        arguments,
                    )
                    .await;
            }
            if matches!(
                canonical_tool_name,
                crate::transport::runtime_tools::LIST_EXTENSIONS_TOOL
                    | crate::transport::runtime_tools::REQUEST_EXTENSION_TOOL
            ) {
                if let Some(result) = self
                    .try_dispatch_remote_capability_runtime_tool_call(
                        &provider_runs[0],
                        canonical_tool_name,
                        arguments.clone(),
                    )
                    .await?
                {
                    return Ok(result);
                }
                return self
                    .dispatch_capability_runtime_tool_call(
                        &provider_runs[0],
                        canonical_tool_name,
                        arguments,
                    )
                    .await;
            }
            if let Some(result) = self
                .try_dispatch_script_runtime_tool_call(
                    &provider_runs[0],
                    canonical_tool_name,
                    arguments.clone(),
                )
                .await?
            {
                return Ok(result);
            }
            if let Some(result) = self
                .try_dispatch_connector_runtime_tool_call(
                    &provider_runs[0],
                    canonical_tool_name,
                    arguments.clone(),
                )
                .await?
            {
                return Ok(result);
            }
            if matches!(
                canonical_tool_name,
                crate::transport::runtime_tools::LIST_CREDENTIAL_HANDLES_TOOL
                    | crate::transport::runtime_tools::HTTP_REQUEST_WITH_CREDENTIAL_TOOL
                    | crate::transport::runtime_tools::SEND_SECRET_TO_TERMINAL_TOOL
                    | crate::transport::runtime_tools::REQUEST_POPUP_TOOL
            ) {
                return self
                    .dispatch_credential_runtime_tool_call(
                        &provider_runs[0],
                        canonical_tool_name,
                        arguments,
                    )
                    .await;
            }
            if matches!(
                canonical_tool_name,
                crate::transport::runtime_tools::SLICE_SCREEN_STATUS_TOOL
                    | crate::transport::runtime_tools::SLICE_SCREENSHOT_TOOL
                    | crate::transport::runtime_tools::SLICE_OCR_TOOL
                    | crate::transport::runtime_tools::SLICE_FIND_TEXT_TOOL
                    | crate::transport::runtime_tools::SLICE_MOUSE_TOOL
                    | crate::transport::runtime_tools::SLICE_KEYBOARD_TOOL
                    | crate::transport::runtime_tools::SLICE_OPEN_URL_TOOL
            ) {
                return self
                    .dispatch_slice_runtime_tool_call(
                        &provider_runs[0],
                        canonical_tool_name,
                        arguments,
                    )
                    .await;
            }
            self.dispatch_authenticated_workflow_runtime_tool_call(
                &provider_runs,
                canonical_tool_name,
                arguments,
            )
            .await
        }
    }

    fn runtime_auth_token_has_active_provider_run(&self, auth_token: &str) -> bool {
        !self
            .owned
            .provider_store
            .get_runs_by_runtime_mcp_auth_token(auth_token)
            .is_empty()
    }

    fn slice_kernel_id(&self) -> Option<String> {
        self.owned
            .config_projection
            .snapshot()
            .host_machine_id
            .strip_prefix("slice:")
            .map(str::to_string)
    }
}
