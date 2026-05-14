//! Runtime MCP tool dispatch.
//!
//! Provider tool calls enter here and are routed to managed-I/O handlers or other runtime-owned
//! tool surfaces with consistent authorization and JSON payload shaping.

use super::*;

mod capability;
mod capability_registry;
mod credential;
mod managed_io_local;
mod managed_io_permission;
mod managed_io_remote_dispatch;
mod remote_capability_sync;
mod slice;
mod workflow_forwarding;

use managed_io_permission::{managed_io_permission_interaction, managed_io_tool_requires_popup};

impl KernelRuntimeState {
    pub(crate) fn runtime_tool_specs_for_auth_token(
        &self,
        auth_token: &str,
    ) -> Vec<crate::transport::runtime_tools::RuntimeToolSpec> {
        let mut specs = crate::transport::runtime_tools::managed_io_runtime_tool_specs()
            .into_iter()
            .chain(crate::transport::runtime_tools::capability_runtime_tool_specs())
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
                        crate::transport::runtime_tools::canonical_capability_tool_name(tool_name)
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
                crate::transport::runtime_tools::LIST_CAPABILITIES_TOOL
                    | crate::transport::runtime_tools::REQUEST_CAPABILITY_TOOL
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
                    serde_json::from_value::<
                        crate::transport::runtime_tools::ValidateWorkflowOutputArgs,
                    >(arguments.clone())
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

    async fn managed_io_workspace_for_provider_run(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
    ) -> Result<ManagedIoWorkspaceContext, DaemonError> {
        let session = self
            .owned
            .session_store
            .get_session(provider_run.session_id())?;
        let workspace_root = provider_run
            .working_directory()
            .cloned()
            .unwrap_or_else(|| PathBuf::from(session.worktree_id()));
        let identity = workspace_identity_for_root_off_thread(workspace_root.clone()).await?;
        let identity =
            managed_io_identity_for_session_workspace_link(identity, &session, &workspace_root);
        let snapshot = self.owned.workspace_identity_monitor.observe_provider_run(
            provider_run.id(),
            workspace_root.clone(),
            identity,
        );
        Ok(ManagedIoWorkspaceContext {
            root: workspace_root,
            identity: snapshot.current_identity,
            generation: snapshot.generation,
            identity_changed: snapshot.identity_changed,
            valid: snapshot.valid,
        })
    }

    async fn maybe_gate_managed_io_mutation(
        &self,
        session_id: &str,
        agent_id: Option<&str>,
        permission_level: crate::provider::AgentPermissionLevel,
        tool_name: &str,
        arguments: &serde_json::Value,
    ) -> Result<Option<crate::transport::runtime_tools::RuntimeToolResult>, DaemonError> {
        if permission_level != crate::provider::AgentPermissionLevel::Required {
            return Ok(None);
        }
        let Some(agent_id) = agent_id else {
            return Ok(None);
        };
        if !managed_io_tool_requires_popup(tool_name) {
            return Ok(None);
        }
        let interaction = managed_io_permission_interaction(agent_id, tool_name, arguments)?;
        let interaction_id = interaction.id().to_string();
        let resolution = self
            .create_runtime_interaction(session_id, interaction)
            .await?
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "managed_io_permission_popup",
                message: format!("managed I/O approval dropped before resolution: {error}"),
            })?;
        if resolution.choice_id.as_deref() == Some("allow") {
            return Ok(None);
        }
        Ok(Some(crate::transport::runtime_tools::RuntimeToolResult {
            ok: false,
            payload: serde_json::json!({
                "applied": false,
                "interaction_id": interaction_id,
                "reason": {
                    "kind": "permission_denied",
                    "message": "The managed I/O operation was not approved."
                },
                "next_action": "Retry after approving the managed I/O request, or switch the session/agent permissions to yolo.",
            }),
        }))
    }

    async fn effective_permission_level_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<crate::provider::AgentPermissionLevel, DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        let session = self.owned.session_store.get_session(session_id)?;
        Ok(crate::session::effective_agent_permission_level(
            &session,
            Some(&agent),
        ))
    }
}
