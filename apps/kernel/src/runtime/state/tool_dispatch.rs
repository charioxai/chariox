//! Runtime MCP tool dispatch.
//!
//! Provider tool calls enter here and are routed to workspace live sync handlers or other runtime-owned
//! tool surfaces with consistent authorization and JSON payload shaping.

use super::*;

mod agent_messaging;
mod capability_registry;
mod connector;
mod credential;
mod extension_list_tool;
mod extension_registration_tool;
mod extension_request_tool;
mod home_connector_executor;
mod home_extension_authorizer;
mod home_extension_execution_policy;
mod home_mcp_proxy_executor;
mod home_script_executor;
mod meta;
mod recall;
mod remote_capability_sync;
mod remote_extension_control_plane;
mod script;
mod skill_package_response;
mod slice;
mod worker_home_credential_client;
mod worker_home_extension_client;
mod workflow_authenticated;
mod workflow_forwarding;
mod workspace_live_sync_access;
mod workspace_live_sync_local;
mod workspace_live_sync_managed_fanout;
mod workspace_live_sync_permission;
mod workspace_live_sync_remote_dispatch;

use workspace_live_sync_managed_fanout::*;

impl KernelRuntimeState {
    #[cfg(test)]
    pub(crate) fn runtime_mcp_auth_token_for_provider_run(
        &self,
        provider_run_id: &str,
    ) -> Option<String> {
        self.owned
            .provider_store
            .get_run(provider_run_id)
            .ok()
            .and_then(|run| run.runtime_mcp_auth_token().map(str::to_string))
    }

    pub(crate) fn runtime_tool_specs_for_auth_token(
        &self,
        auth_token: &str,
    ) -> Vec<crate::transport::runtime_tools::RuntimeToolSpec> {
        let provider_runs = self
            .owned
            .provider_store
            .get_runs_by_runtime_mcp_auth_token(auth_token);
        let mut specs = Vec::new();
        if self.meta_runtime_tool_specs_enabled_for_auth_token(auth_token) {
            specs.extend(crate::transport::runtime_tools::meta_runtime_tool_specs());
            specs.extend(crate::transport::runtime_tools::agent_messaging_runtime_tool_specs());
            specs.extend(
                crate::transport::runtime_tools::workspace_live_sync_runtime_tool_specs()
                    .into_iter()
                    .filter(|spec| {
                        spec.name == crate::transport::runtime_tools::READ_ARTIFACT_TOOL
                    }),
            );
            specs.extend(crate::transport::runtime_tools::recall_runtime_tool_specs());
            if self.slice_kernel_id().is_some() {
                specs.extend(crate::transport::runtime_tools::slice_runtime_tool_specs());
            }
            return specs;
        }
        if matches!(provider_runs.as_slice(), [_]) {
            specs.extend(crate::transport::runtime_tools::agent_messaging_runtime_tool_specs());
            specs.extend(crate::transport::runtime_tools::workspace_live_sync_runtime_tool_specs());
            specs.extend(crate::transport::runtime_tools::extension_runtime_tool_specs());
            specs.extend(crate::transport::runtime_tools::recall_runtime_tool_specs());
            specs.extend(provider_runs.iter().flat_map(|run| {
                run.remote_extension_manifest()
                    .home_proxy_runtime_tool_specs()
            }));
            specs.extend(self.script_runtime_tool_specs_for_auth_token(auth_token));
            specs.extend(self.connector_runtime_tool_specs_for_auth_token(auth_token));
            specs.extend(crate::transport::runtime_tools::credential_runtime_tool_specs());
            if self.slice_kernel_id().is_some() {
                specs.extend(crate::transport::runtime_tools::slice_runtime_tool_specs());
            }
        }
        // MCP discovery runs independently of the app mutex.  Leased-run identity is
        // projected into the lock-free provider projection when the lease launches or
        // reuses a backing run, so contention cannot silently hide lease-only tools.
        let leased_provider_run = provider_runs.iter().any(|run| {
            self.owned
                .provider_run_projection
                .is_leased_provider_run(run.id())
        });
        let workflow_tools_enabled =
            leased_provider_run || provider_runs.iter().any(|run| run.workflow_tools_enabled());
        if workflow_tools_enabled {
            specs.extend(
                crate::transport::runtime_tools::workflow_runtime_tool_specs_without_event_reply(),
            );
            if provider_runs
                .iter()
                .any(|run| run.workflow_event_reply_enabled())
            {
                specs.push(crate::transport::runtime_tools::workflow_reply_to_event_tool_spec());
            }
            if provider_runs
                .iter()
                .any(|run| run.workflow_event_actions_enabled())
            {
                specs.push(crate::transport::runtime_tools::workflow_event_action_tool_spec());
            }
            if provider_runs
                .iter()
                .any(|run| run.workflow_event_context_enabled())
            {
                specs.push(crate::transport::runtime_tools::workflow_event_context_tool_spec());
            }
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
                crate::transport::runtime_tools::canonical_agent_messaging_tool_name(tool_name)
                    .or_else(|| {
                        crate::transport::runtime_tools::canonical_workspace_live_sync_tool_name(
                            tool_name,
                        )
                    })
                    .or_else(|| {
                        crate::transport::runtime_tools::canonical_extension_tool_name(tool_name)
                    })
                    .or_else(|| {
                        crate::transport::runtime_tools::canonical_recall_tool_name(tool_name)
                    })
                    .or_else(|| {
                        crate::transport::runtime_tools::canonical_credential_tool_name(tool_name)
                    })
                    .or_else(|| {
                        crate::transport::runtime_tools::canonical_slice_tool_name(tool_name)
                    })
                    .or_else(|| {
                        crate::transport::runtime_tools::canonical_meta_tool_name(tool_name)
                    })
                    .or_else(|| {
                        crate::transport::runtime_tools::canonical_workflow_tool_name(tool_name)
                    })
                    .unwrap_or_else(|| tool_name.strip_prefix("chariox_").unwrap_or(tool_name));
            let provider_runs = owned
                .provider_store
                .get_runs_by_runtime_mcp_auth_token(auth_token);
            if provider_runs.is_empty() {
                return Err(DaemonError::LocalTransport {
                    operation: "dispatch_authenticated_runtime_tool_call",
                    message: "invalid runtime MCP auth token".to_string(),
                });
            }
            let is_metaagent_auth_token =
                self.meta_runtime_tool_specs_enabled_for_auth_token(auth_token);
            let is_meta_tool =
                crate::transport::runtime_tools::canonical_meta_tool_name(tool_name).is_some();
            let is_metaagent_allowed_direct_tool = is_metaagent_direct_runtime_tool_allowed(
                canonical_tool_name,
                self.slice_kernel_id().is_some(),
            );
            if is_metaagent_auth_token && !is_meta_tool && !is_metaagent_allowed_direct_tool {
                return Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: false,
                    payload: serde_json::json!({
                        "error": format!("runtime tool `{canonical_tool_name}` is not available to agents in Meta mode"),
                        "tool": canonical_tool_name,
                    }),
                });
            }
            if is_meta_tool {
                let (provider_run, _, _) = self.metaagent_context_for_auth_token(auth_token)?;
                return self
                    .dispatch_meta_runtime_tool_call(&provider_run, canonical_tool_name, arguments)
                    .await;
            }
            let is_workflow_tool =
                crate::transport::runtime_tools::canonical_workflow_tool_name(tool_name).is_some();
            let provider_run = if is_workflow_tool {
                None
            } else {
                Some(unambiguous_runtime_tool_provider_run(
                    &provider_runs,
                    canonical_tool_name,
                )?)
            };
            if is_workflow_tool {
                return self
                    .dispatch_authenticated_workflow_runtime_tool_call(
                        &provider_runs,
                        canonical_tool_name,
                        arguments,
                    )
                    .await;
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
                    .try_dispatch_remote_workspace_live_sync_runtime_tool_call(
                        provider_run.expect("non-workflow tool should have provider run"),
                        canonical_tool_name,
                        arguments.clone(),
                    )
                    .await?
                {
                    return Ok(result);
                }
                return self
                    .dispatch_workspace_live_sync_runtime_tool_call(
                        provider_run.expect("non-workflow tool should have provider run"),
                        canonical_tool_name,
                        arguments,
                    )
                    .await;
            }
            if matches!(
                canonical_tool_name,
                crate::transport::runtime_tools::LIST_SESSION_AGENTS_TOOL
                    | crate::transport::runtime_tools::GET_SESSION_AGENT_TOOL
                    | crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL
                    | crate::transport::runtime_tools::LIST_EXTENSIONS_TOOL
                    | crate::transport::runtime_tools::REQUEST_EXTENSION_TOOL
                    | crate::transport::runtime_tools::REGISTER_MCP_TOOL
                    | crate::transport::runtime_tools::REGISTER_SKILL_PATH_TOOL
                    | crate::transport::runtime_tools::REGISTER_ENVIRONMENT_TOOL
                    | crate::transport::runtime_tools::REGISTER_SCRIPT_PATH_TOOL
                    | crate::transport::runtime_tools::REGISTER_CONNECTOR_PATH_TOOL
                    | crate::transport::runtime_tools::REGISTER_CONNECTOR_ADAPTER_PATH_TOOL
            ) {
                if let Some(result) = self
                    .try_dispatch_remote_capability_runtime_tool_call(
                        provider_run.expect("non-workflow tool should have provider run"),
                        canonical_tool_name,
                        arguments.clone(),
                    )
                    .await?
                {
                    return Ok(result);
                }
                return self
                    .dispatch_capability_runtime_tool_call(
                        provider_run.expect("non-workflow tool should have provider run"),
                        canonical_tool_name,
                        arguments,
                    )
                    .await;
            }
            if matches!(
                canonical_tool_name,
                crate::transport::runtime_tools::SEARCH_RECALL_TOOL
                    | crate::transport::runtime_tools::QUERY_RECALL_TOOL
            ) {
                return self
                    .dispatch_recall_runtime_tool_call(
                        provider_run.expect("non-workflow tool should have provider run"),
                        canonical_tool_name,
                        arguments,
                    )
                    .await;
            }
            if let Some(result) = self
                .try_dispatch_remote_home_extension_runtime_tool_call(
                    provider_run.expect("non-workflow tool should have provider run"),
                    canonical_tool_name,
                    arguments.clone(),
                )
                .await?
            {
                return Ok(result);
            }
            if let Some(result) = self
                .try_dispatch_script_runtime_tool_call(
                    provider_run.expect("non-workflow tool should have provider run"),
                    canonical_tool_name,
                    arguments.clone(),
                )
                .await?
            {
                return Ok(result);
            }
            if let Some(result) = self
                .try_dispatch_connector_runtime_tool_call(
                    provider_run.expect("non-workflow tool should have provider run"),
                    canonical_tool_name,
                    arguments.clone(),
                )
                .await?
            {
                return Ok(result);
            }
            if matches!(
                canonical_tool_name,
                crate::transport::runtime_tools::META_SESSION_OVERVIEW_TOOL
                    | crate::transport::runtime_tools::META_SEARCH_COMMANDS_TOOL
                    | crate::transport::runtime_tools::META_LIST_COMMANDS_TOOL
                    | crate::transport::runtime_tools::META_COMMAND_DOCS_TOOL
                    | crate::transport::runtime_tools::META_SEARCH_GUIDES_TOOL
                    | crate::transport::runtime_tools::META_LIST_GUIDES_TOOL
                    | crate::transport::runtime_tools::META_READ_GUIDE_TOOL
                    | crate::transport::runtime_tools::META_RUN_COMMAND_TOOL
                    | crate::transport::runtime_tools::META_LIST_EVENTS_TOOL
                    | crate::transport::runtime_tools::META_READ_EVENT_TOOL
                    | crate::transport::runtime_tools::META_ACK_EVENT_TOOL
                    | crate::transport::runtime_tools::META_TURN_OVERVIEW_TOOL
                    | crate::transport::runtime_tools::META_TURN_BLOB_TOOL
                    | crate::transport::runtime_tools::META_SUBSCRIBE_TRACE_TOOL
                    | crate::transport::runtime_tools::META_POLL_TRACE_TOOL
                    | crate::transport::runtime_tools::META_WAIT_TRACE_TOOL
                    | crate::transport::runtime_tools::META_UNSUBSCRIBE_TRACE_TOOL
                    | crate::transport::runtime_tools::META_SUBSCRIBE_EVENTS_TOOL
                    | crate::transport::runtime_tools::META_UNSUBSCRIBE_EVENTS_TOOL
                    | crate::transport::runtime_tools::META_LIST_SUBSCRIPTIONS_TOOL
                    | crate::transport::runtime_tools::META_WORKFLOW_CODE_CREATE_TOOL
                    | crate::transport::runtime_tools::META_WORKFLOW_CODE_READ_TOOL
                    | crate::transport::runtime_tools::META_WORKFLOW_CODE_LIST_TOOL
                    | crate::transport::runtime_tools::META_WORKFLOW_CODE_UPDATE_TOOL
                    | crate::transport::runtime_tools::META_WORKFLOW_CODE_DELETE_TOOL
                    | crate::transport::runtime_tools::META_WORKFLOW_CODE_VALIDATE_TOOL
                    | crate::transport::runtime_tools::META_WORKFLOW_CODE_APPLY_TOOL
                    | crate::transport::runtime_tools::META_WORKFLOW_CODE_RUN_TOOL
                    | crate::transport::runtime_tools::META_WORKFLOW_CODE_EXPORT_TOOL
                    | crate::transport::runtime_tools::META_WORKFLOW_CODE_IMPORT_TOOL
                    | crate::transport::runtime_tools::META_WORKFLOW_CODE_PACKAGE_EXPORT_TOOL
                    | crate::transport::runtime_tools::META_WORKFLOW_CODE_PACKAGE_IMPORT_TOOL
                    | crate::transport::runtime_tools::META_WORKFLOW_CODE_SOURCE_EXPORT_TOOL
                    | crate::transport::runtime_tools::META_WORKFLOW_CODE_SOURCE_EXPORT_DIRECTORY_TOOL
                    | crate::transport::runtime_tools::META_WORKFLOW_CODE_CANVAS_CONTRACT_TOOL
                    | crate::transport::runtime_tools::META_RESOLVE_RUNTIME_INTERACTION_TOOL
            ) {
                if let Some(result) = self
                    .try_dispatch_remote_meta_runtime_tool_call(
                        provider_run.expect("non-workflow tool should have provider run"),
                        canonical_tool_name,
                        arguments.clone(),
                    )
                    .await?
                {
                    return Ok(result);
                }
                return self
                    .dispatch_meta_runtime_tool_call(
                        provider_run.expect("non-workflow tool should have provider run"),
                        canonical_tool_name,
                        arguments,
                    )
                    .await;
            }
            if matches!(
                canonical_tool_name,
                crate::transport::runtime_tools::LIST_CREDENTIAL_HANDLES_TOOL
                    | crate::transport::runtime_tools::CREATE_GENERATED_CREDENTIAL_TOOL
                    | crate::transport::runtime_tools::REQUEST_CREDENTIAL_SECRET_TOOL
                    | crate::transport::runtime_tools::HTTP_REQUEST_WITH_CREDENTIAL_TOOL
                    | crate::transport::runtime_tools::SEND_SECRET_TO_TERMINAL_TOOL
                    | crate::transport::runtime_tools::REQUEST_POPUP_TOOL
            ) {
                if let Some(result) = self
                    .try_dispatch_remote_home_credential_runtime_tool_call(
                        provider_run.expect("non-workflow tool should have provider run"),
                        canonical_tool_name,
                        arguments.clone(),
                    )
                    .await?
                {
                    return Ok(result);
                }
                return self
                    .dispatch_credential_runtime_tool_call(
                        provider_run.expect("non-workflow tool should have provider run"),
                        canonical_tool_name,
                        arguments,
                    )
                    .await;
            }
            if is_slice_runtime_tool(canonical_tool_name) {
                return self
                    .dispatch_slice_runtime_tool_call(
                        provider_run.expect("non-workflow tool should have provider run"),
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

    fn slice_kernel_id(&self) -> Option<String> {
        self.owned
            .config_projection
            .snapshot()
            .host_machine_id
            .strip_prefix("slice:")
            .map(str::to_string)
    }
}

fn is_slice_runtime_tool(tool_name: &str) -> bool {
    crate::transport::runtime_tools::canonical_slice_tool_name(tool_name).is_some()
}

fn unambiguous_runtime_tool_provider_run<'a>(
    provider_runs: &'a [crate::provider::RuntimeProviderRun],
    tool_name: &str,
) -> Result<&'a crate::provider::RuntimeProviderRun, DaemonError> {
    match provider_runs {
        [provider_run] => Ok(provider_run),
        [] => Err(DaemonError::LocalTransport {
            operation: "dispatch_authenticated_runtime_tool_call",
            message: "invalid runtime MCP auth token".to_string(),
        }),
        runs => {
            let mut run_ids = runs
                .iter()
                .map(|run| run.id().to_string())
                .collect::<Vec<_>>();
            run_ids.sort();
            Err(DaemonError::LocalTransport {
                operation: "dispatch_authenticated_runtime_tool_call",
                message: format!(
                    "runtime MCP auth token is bound to multiple active provider runs ({}) while dispatching `{tool_name}`. Non-workflow runtime tools require one authoritative provider run; run /kernel health and /provider processes, then stop duplicate provider runs before retrying.",
                    run_ids.join(",")
                ),
            })
        }
    }
}

fn is_metaagent_direct_runtime_tool_allowed(tool_name: &str, slice_available: bool) -> bool {
    matches!(
        tool_name,
        crate::transport::runtime_tools::LIST_SESSION_AGENTS_TOOL
            | crate::transport::runtime_tools::GET_SESSION_AGENT_TOOL
            | crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL
            | crate::transport::runtime_tools::READ_ARTIFACT_TOOL
            | crate::transport::runtime_tools::SEARCH_RECALL_TOOL
            | crate::transport::runtime_tools::QUERY_RECALL_TOOL
    ) || (slice_available
        && crate::transport::runtime_tools::canonical_slice_tool_name(tool_name).is_some())
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_advertised_slice_tool_uses_the_slice_dispatch_path() {
        for spec in crate::transport::runtime_tools::slice_runtime_tool_specs() {
            assert!(
                super::is_slice_runtime_tool(&spec.name),
                "advertised slice tool {} must use slice dispatch",
                spec.name
            );
        }
    }
}
