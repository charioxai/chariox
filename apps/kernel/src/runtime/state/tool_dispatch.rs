//! Runtime MCP tool dispatch.
//!
//! Provider tool calls enter here and are routed to managed-I/O handlers or other runtime-owned
//! tool surfaces with consistent authorization and JSON payload shaping.

use super::*;

impl KernelRuntimeState {
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
            ) {
                return self
                    .dispatch_credential_runtime_tool_call(
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

    async fn dispatch_credential_runtime_tool_call(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let user_config = self.owned.config_projection.snapshot().user_config;
        let service = crate::secret::RuntimeSecretService::with_vault_service(
            user_config.credentials,
            user_config.credential_vault.service,
        );
        match tool_name {
            crate::transport::runtime_tools::LIST_CREDENTIAL_HANDLES_TOOL => {
                Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "credentials": service.list_handles()
                    }),
                })
            }
            crate::transport::runtime_tools::HTTP_REQUEST_WITH_CREDENTIAL_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::HttpRequestWithCredentialArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_http_request_with_credential",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let request = crate::secret::CredentialHttpRequest {
                    credential_id: args.credential_id,
                    method: args.method,
                    url: args.url,
                    headers: args.headers,
                    body_text: args.body_text,
                    body_json: args.body_json,
                };
                let response = tokio::task::spawn_blocking(move || {
                    service.http_request_with_credential(request)
                })
                .await
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_http_request_with_credential",
                    message: error.to_string(),
                })??;
                Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::to_value(response).map_err(|error| {
                        DaemonError::LocalTransport {
                            operation: "runtime_tool_http_request_with_credential",
                            message: error.to_string(),
                        }
                    })?,
                })
            }
            crate::transport::runtime_tools::SEND_SECRET_TO_TERMINAL_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::SendSecretToTerminalArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_send_secret_to_terminal",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let mut input = service.terminal_secret_input(&args.credential_id)?;
                if args.append_newline {
                    input.push('\n');
                }
                let provider_run_id = provider_run.id().to_string();
                self.with_app_side_effect(move |app| {
                    app.write_provider_pty_input_for_runtime(&provider_run_id, input.as_bytes())
                })
                .await?;
                Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "submitted": true,
                        "credential_id": args.credential_id,
                        "target": "current_provider_run",
                    }),
                })
            }
            _ => Err(DaemonError::LocalTransport {
                operation: "dispatch_credential_runtime_tool_call",
                message: format!("unknown credential runtime tool `{tool_name}`"),
            }),
        }
    }

    async fn try_dispatch_remote_capability_runtime_tool_call(
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
        let (mut result, skill_package) = match response {
            RelayPeerResponse::CapabilityRuntimeToolHandled {
                result,
                skill_package,
            } => (result, skill_package),
            other => {
                return Err(DaemonError::LocalTransport {
                    operation: "forward leased capability runtime tool",
                    message: format!("unexpected forwarded capability response: {other:?}"),
                });
            }
        };
        if result.ok {
            if let Some(skill_package) = skill_package {
                let materialized_root = crate::skill::materialize_skill_package(
                    &workspace_context
                        .root
                        .join(".arroba")
                        .join("remote")
                        .join("skills")
                        .join(&remote_context.home_kernel_id),
                    &skill_package,
                )?;
                if let Some(skill) = result
                    .payload
                    .get_mut("skill")
                    .and_then(serde_json::Value::as_object_mut)
                {
                    skill.insert(
                        "path".to_string(),
                        serde_json::Value::String(
                            materialized_root
                                .join("SKILL.md")
                                .to_string_lossy()
                                .to_string(),
                        ),
                    );
                    skill.insert(
                        "materialized_root".to_string(),
                        serde_json::Value::String(materialized_root.to_string_lossy().to_string()),
                    );
                    skill.insert(
                        "version_hash".to_string(),
                        serde_json::Value::String(skill_package.version_hash),
                    );
                    skill.insert(
                        "files".to_string(),
                        serde_json::Value::Array(
                            skill_package
                                .files
                                .into_iter()
                                .map(|file| serde_json::Value::String(file.path))
                                .collect(),
                        ),
                    );
                }
            }
        }
        Ok(Some(result))
    }

    pub(super) async fn ensure_remote_skill_packages_for_agent(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> Result<Vec<crate::transport::relay_peer::RemoteSkillMaterialization>, DaemonError> {
        let Some(remote_execution) = agent.remote_execution().cloned() else {
            return Ok(Vec::new());
        };
        if agent.skill_grants().is_empty() {
            return Ok(Vec::new());
        }
        let session = self.owned.session_store.get_session(agent.session_id())?;
        let skill_registry = skill_registry_for_workspace(session.workspace_id());
        let packages = package_granted_skills(&skill_registry, agent.skill_grants())?;
        if packages.is_empty() {
            return Ok(Vec::new());
        }
        let response = self
            .with_app_side_effect(|app| {
                app.block_on_relay_future(
                    crate::transport::relay_client::send_peer_request_via_temporary_connection(
                        app.config(),
                        ClientTarget {
                            daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                            daemon_alias: None,
                        },
                        RelayPeerRequest::EnsureRemoteSkillPackages {
                            context: crate::transport::relay_peer::RemoteSkillSyncContext {
                                home_kernel_id: app.config().daemon_id.clone(),
                                home_session_id: agent.session_id().to_string(),
                                home_agent_id: agent.id().to_string(),
                                leased_agent_id: remote_execution.leased_agent_id.clone(),
                            },
                            packages: packages.clone(),
                        },
                    ),
                )
            })
            .await?;
        match response {
            RelayPeerResponse::RemoteSkillPackagesEnsured { materialized } => Ok(materialized),
            other => Err(DaemonError::LocalTransport {
                operation: "ensure remote skill packages",
                message: format!("unexpected remote skill sync response: {other:?}"),
            }),
        }
    }

    pub(super) fn required_remote_mcps_for_agent(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> Result<Vec<crate::transport::relay_peer::RequiredRemoteMcp>, DaemonError> {
        if agent.mcp_grants().is_empty() {
            return Ok(Vec::new());
        }
        let session = self.owned.session_store.get_session(agent.session_id())?;
        let registry = mcp_registry_for_workspace(session.workspace_id());
        required_remote_mcps(&registry, agent.mcp_grants())
    }

    pub(super) async fn ensure_remote_mcp_availability_for_agent(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> Result<(), DaemonError> {
        let Some(remote_execution) = agent.remote_execution().cloned() else {
            return Ok(());
        };
        let required_mcps = self.required_remote_mcps_for_agent(agent)?;
        if required_mcps.is_empty() {
            return Ok(());
        }
        let response = self
            .with_app_side_effect(|app| {
                app.block_on_relay_future(
                    crate::transport::relay_client::send_peer_request_via_temporary_connection(
                        app.config(),
                        ClientTarget {
                            daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                            daemon_alias: None,
                        },
                        RelayPeerRequest::CheckRemoteMcpAvailability {
                            context: crate::transport::relay_peer::RemoteMcpCheckContext {
                                home_kernel_id: app.config().daemon_id.clone(),
                                home_session_id: agent.session_id().to_string(),
                                home_agent_id: agent.id().to_string(),
                                leased_agent_id: remote_execution.leased_agent_id.clone(),
                            },
                            required_mcps,
                        },
                    ),
                )
            })
            .await?;
        match response {
            RelayPeerResponse::RemoteMcpAvailabilityChecked { results } => {
                let unavailable = results
                    .iter()
                    .filter(|result| {
                        !matches!(
                            result.status,
                            crate::transport::relay_peer::RemoteMcpAvailabilityStatus::Available
                        )
                    })
                    .collect::<Vec<_>>();
                if unavailable.is_empty() {
                    Ok(())
                } else {
                    Err(DaemonError::LocalTransport {
                        operation: "remote mcp availability",
                        message: format_remote_mcp_unavailable(&unavailable),
                    })
                }
            }
            other => Err(DaemonError::LocalTransport {
                operation: "remote mcp availability",
                message: format!("unexpected remote MCP availability response: {other:?}"),
            }),
        }
    }

    pub(super) fn apply_remote_materialized_skill_prompt_context(
        &self,
        agent: &crate::agent::AgentInstance,
        prompt: &str,
        materialized: &[crate::transport::relay_peer::RemoteSkillMaterialization],
    ) -> Result<String, DaemonError> {
        if agent.skill_grants().is_empty() {
            return Ok(prompt.to_string());
        }
        let session = self.owned.session_store.get_session(agent.session_id())?;
        let registry = skill_registry_for_workspace(session.workspace_id());
        let materialized_by_name = materialized
            .iter()
            .map(|entry| (entry.name.as_str(), entry))
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut lines = vec![
            "Available Arroba skills for this remote agent:".to_string(),
            "These granted skills were synchronized from the home kernel and materialized on this worker before this prompt. Follow them when they match the task; assets, scripts, and references are available under each materialized_root.".to_string(),
        ];
        let mut bodies = Vec::new();
        for grant in agent.skill_grants() {
            let Some(skill) = registry.get(grant)? else {
                return Err(DaemonError::LocalTransport {
                    operation: "remote provider.prompt.skills",
                    message: format!(
                        "agent `{}` has missing skill grant `{grant}`",
                        agent.agent_ref()
                    ),
                });
            };
            let materialized = materialized_by_name
                .get(skill.name.as_str())
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "remote provider.prompt.skills",
                    message: format!("remote skill `{}` was not materialized", skill.name),
                })?;
            let summary = skill
                .short_description
                .as_ref()
                .unwrap_or(&skill.description);
            lines.push(format!(
                "- `{}`: {}; materialized_root: {}; version: {}",
                skill.name, summary, materialized.materialized_root, materialized.version_hash
            ));
            let body = std::fs::read_to_string(&skill.path).map_err(|error| {
                DaemonError::LocalTransport {
                    operation: "remote provider.prompt.skills",
                    message: format!(
                        "failed to read skill `{}` body at `{}`: {error}",
                        skill.name,
                        skill.path.display()
                    ),
                }
            })?;
            bodies.push((skill.name, materialized.materialized_root.clone(), body));
        }
        lines.push(String::new());
        lines.push("Full instructions for synchronized Arroba skills:".to_string());
        for (name, materialized_root, body) in bodies {
            lines.push(format!(
                "<arroba_skill name=\"{name}\" materialized_root=\"{materialized_root}\">"
            ));
            lines.push(body.trim().to_string());
            lines.push("</arroba_skill>".to_string());
        }
        Ok(format!("{}\n\n{}", lines.join("\n"), prompt))
    }

    pub(crate) async fn dispatch_forwarded_capability_runtime_tool_call(
        &self,
        context: crate::transport::relay_peer::RemoteManagedIoContext,
        tool_name: String,
        arguments: serde_json::Value,
    ) -> Result<
        (
            crate::transport::runtime_tools::RuntimeToolResult,
            Option<crate::skill::ArrobaSkillPackage>,
        ),
        DaemonError,
    > {
        self.dispatch_capability_runtime_tool_call_for_agent(
            &context.home_session_id,
            &context.home_agent_id,
            &tool_name,
            arguments,
            true,
        )
        .await
    }

    async fn dispatch_capability_runtime_tool_call(
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
        let workspace = std::path::PathBuf::from(session.workspace_id());
        let mut mcp_roots = vec![crate::mcp::ArrobaMcpRegistry::project_root(&workspace)];
        if let Some(user_root) = crate::mcp::ArrobaMcpRegistry::user_root() {
            mcp_roots.push(user_root);
        }
        let mcp_registry = crate::mcp::ArrobaMcpRegistry::new(mcp_roots);
        let mut skill_roots = vec![crate::skill::ArrobaSkillRegistry::project_root(&workspace)];
        if let Some(user_root) = crate::skill::ArrobaSkillRegistry::user_root() {
            skill_roots.push(user_root);
        }
        let skill_registry = crate::skill::ArrobaSkillRegistry::new(skill_roots);

        match tool_name {
            crate::transport::runtime_tools::LIST_CAPABILITIES_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ListCapabilitiesArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_list_capabilities",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let kind = args.kind.as_deref().unwrap_or("all");
                if !matches!(kind, "all" | "mcp" | "skill") {
                    return Ok((
                        crate::transport::runtime_tools::RuntimeToolResult {
                            ok: false,
                            payload: serde_json::json!({
                                "error": "kind must be one of: all, mcp, skill"
                            }),
                        },
                        None,
                    ));
                }
                let mcps = if matches!(kind, "all" | "mcp") {
                    mcp_registry
                        .list()?
                        .into_iter()
                        .map(|mcp| {
                            let granted = agent.mcp_grants().contains(&mcp.name);
                            serde_json::json!({
                                "kind": "mcp",
                                "name": mcp.name,
                                "enabled": mcp.enabled,
                                "required": mcp.required,
                                "granted": granted,
                                "effective_when_requested": "next_provider_launch"
                            })
                        })
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                let skills = if matches!(kind, "all" | "skill") {
                    skill_registry
                        .list()?
                        .into_iter()
                        .map(|skill| {
                            let granted = agent.skill_grants().contains(&skill.name);
                            serde_json::json!({
                                "kind": "skill",
                                "name": skill.name,
                                "description": skill.description,
                                "short_description": skill.short_description,
                                "granted": granted,
                                "effective_when_requested": "now"
                            })
                        })
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                Ok((
                    crate::transport::runtime_tools::RuntimeToolResult {
                        ok: true,
                        payload: serde_json::json!({
                            "agent_ref": agent.agent_ref(),
                            "capabilities": {
                                "mcps": mcps,
                                "skills": skills
                            }
                        }),
                    },
                    None,
                ))
            }
            crate::transport::runtime_tools::REQUEST_CAPABILITY_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::RequestCapabilityArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_request_capability",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let mut skill_payload = serde_json::Value::Null;
                let mut skill_package = None;
                let (agent, effective_when, requires_provider_restart) = match args.kind.as_str() {
                    "mcp" => {
                        if mcp_registry.get(&args.name)?.is_none() {
                            return Ok((
                                crate::transport::runtime_tools::RuntimeToolResult {
                                    ok: false,
                                    payload: serde_json::json!({
                                        "error": format!("MCP `{}` is not installed", args.name),
                                        "kind": "mcp",
                                        "name": args.name,
                                    }),
                                },
                                None,
                            ));
                        }
                        if agent.remote_execution().is_some()
                            && !agent.mcp_grants().contains(&args.name)
                        {
                            let mut checked = agent.clone();
                            checked.grant_mcp(args.name.clone());
                            self.ensure_remote_mcp_availability_for_agent(&checked)
                                .await?;
                        }
                        let granted_agent = self.owned.grant_agent_mcp(
                            agent.id(),
                            args.name.clone(),
                            agent.owner_user_id(),
                        )?;
                        self.append_agent_durable_event(
                            "agent.mcp_granted",
                            &granted_agent,
                            Some(&args.name),
                        )
                        .await?;
                        let (source_attachment_id, previous_prompt) = self
                            .owned
                            .session_store
                            .get_session(session_id)
                            .ok()
                            .and_then(|session| {
                                self.owned
                                    .prompt_state_owner
                                    .active_prompt_for_agent(&session, granted_agent.id())
                                    .map(|prompt| {
                                        (
                                            prompt.source_attachment_id().to_string(),
                                            prompt.prompt().to_string(),
                                        )
                                    })
                            })
                            .unwrap_or_else(|| ("arroba-runtime".to_string(), String::new()));
                        self.remember_pending_mcp_continuation(
                            session_id,
                            granted_agent.id(),
                            &source_attachment_id,
                            &args.name,
                            &previous_prompt,
                        );
                        (granted_agent, "after_provider_reload", true)
                    }
                    "skill" => {
                        let Some(skill) = skill_registry.get(&args.name)? else {
                            return Ok((
                                crate::transport::runtime_tools::RuntimeToolResult {
                                    ok: false,
                                    payload: serde_json::json!({
                                        "error": format!("skill `{}` is not installed", args.name),
                                        "kind": "skill",
                                        "name": args.name,
                                    }),
                                },
                                None,
                            ));
                        };
                        if include_skill_package {
                            skill_package = skill_registry.package(&args.name)?;
                        }
                        if args.return_body.unwrap_or(true) {
                            let body = std::fs::read_to_string(&skill.path).map_err(|error| {
                                DaemonError::LocalTransport {
                                    operation: "runtime_tool_request_capability",
                                    message: format!(
                                        "failed to read skill `{}` body: {error}",
                                        skill.name
                                    ),
                                }
                            })?;
                            skill_payload = serde_json::json!({
                                "name": skill.name,
                                "description": skill.description,
                                "short_description": skill.short_description,
                                "path": skill.path,
                                "body": body
                            });
                        }
                        let granted_agent = self.owned.grant_agent_skill(
                            agent.id(),
                            args.name.clone(),
                            agent.owner_user_id(),
                        )?;
                        self.append_agent_durable_event(
                            "agent.skill_granted",
                            &granted_agent,
                            Some(&args.name),
                        )
                        .await?;
                        (granted_agent, "now", false)
                    }
                    _ => {
                        return Ok((
                            crate::transport::runtime_tools::RuntimeToolResult {
                                ok: false,
                                payload: serde_json::json!({
                                    "error": "kind must be one of: mcp, skill"
                                }),
                            },
                            None,
                        ));
                    }
                };
                let mut payload = serde_json::json!({
                    "granted": true,
                    "kind": args.kind,
                    "name": args.name,
                    "agent_ref": agent.agent_ref(),
                    "effective": effective_when,
                    "requires_provider_restart": requires_provider_restart,
                    "note": match effective_when {
                        "after_provider_reload" => "Arroba will reload this provider conversation after the current turn and send an automatic continuation prompt once the MCP is available.",
                        "next_provider_launch" => "MCP grants are rendered into provider-native MCP config when the provider run launches; restart/relaunch the agent provider run before using this MCP.",
                        "now" => "The skill grant is persisted and the returned SKILL.md body can be followed immediately in this turn.",
                        _ => "The capability grant is persisted."
                    }
                });
                if !skill_payload.is_null() {
                    payload["skill"] = skill_payload;
                }
                Ok((
                    crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload },
                    skill_package,
                ))
            }
            _ => Err(DaemonError::LocalTransport {
                operation: "dispatch_capability_runtime_tool_call",
                message: format!("unknown capability runtime tool `{tool_name}`"),
            }),
        }
    }

    async fn dispatch_managed_io_runtime_tool_call(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let workspace_context = self
            .managed_io_workspace_for_provider_run(provider_run)
            .await?;
        if !workspace_context.valid {
            return Ok(managed_io_workspace_identity_rejected(&workspace_context));
        }
        let workspace_root = workspace_context.root.clone();
        let workspace_identity = workspace_context.identity.clone();
        let mut coordinator = self.owned.managed_io_coordinator.lock().await;
        match tool_name {
            crate::transport::runtime_tools::READ_ARTIFACT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManagedReadArtifactArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_read_artifact",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain =
                    KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
                let read = crate::io::ManagedFileIo::read_artifact(
                    &mut coordinator,
                    crate::io::ManagedFileReadRequest {
                        workspace_identity: workspace_identity.clone(),
                        workspace_root: workspace_root.clone(),
                        path: PathBuf::from(args.path),
                        domain,
                    },
                )
                .map_err(managed_io_daemon_error)?;
                self.owned.managed_io_external_changes.observe_managed_read(
                    provider_run.id(),
                    &workspace_identity,
                    &workspace_root,
                    &read.path,
                );
                let mut payload = managed_io_read_payload(read);
                add_managed_io_workspace_payload(&mut payload, &workspace_context);
                Ok(crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload })
            }
            crate::transport::runtime_tools::EDIT_ARTIFACT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManagedEditArtifactArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_edit_artifact",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain =
                    KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
                if domain != crate::io::ArtifactDomainKind::TextDocument {
                    return Err(DaemonError::LocalTransport {
                        operation: "runtime_tool_edit_artifact",
                        message: "managed edit currently supports only text artifacts".to_string(),
                    });
                }
                let operation = match (args.range, args.old_text) {
                    (Some(range), Some(old_text)) => crate::io::AgentEditOperation::ReplaceRange {
                        range: crate::io::TextRange::new(range.start, range.end),
                        old_text,
                        new_text: args.new_text,
                    },
                    (None, Some(old_text)) => crate::io::AgentEditOperation::ReplaceText {
                        old_text,
                        new_text: args.new_text,
                    },
                    (Some(_), None) => {
                        return Err(DaemonError::LocalTransport {
                            operation: "runtime_tool_edit_artifact",
                            message: "range edits require old_text".to_string(),
                        });
                    }
                    (None, None) => {
                        return Err(DaemonError::LocalTransport {
                            operation: "runtime_tool_edit_artifact",
                            message: "managed text edits require old_text or range+old_text"
                                .to_string(),
                        });
                    }
                };
                let path = PathBuf::from(args.path.clone());
                let before = managed_io_text_for_diff(&workspace_root, &path, false);
                let reservation_ranges = managed_io_reservation_ranges_for_operation(
                    &operation,
                    before.as_ref(),
                    crate::io::TextRange::new(0, usize::MAX),
                );
                let reservation = match managed_io_try_reserve_ranges(
                    &mut coordinator,
                    &workspace_identity,
                    &path,
                    reservation_ranges,
                    managed_io_reservation_owner(provider_run, tool_name),
                ) {
                    Ok(reservation) => reservation,
                    Err(mut output) => {
                        add_managed_io_workspace_payload(&mut output.payload, &workspace_context);
                        return Ok(output);
                    }
                };
                let external_change_notice = self
                    .owned
                    .managed_io_external_changes
                    .external_change_notice(&workspace_identity, &path);
                let result = crate::io::ManagedFileIo::apply_edit(
                    &mut coordinator,
                    crate::io::ManagedFileWriteRequest {
                        workspace_identity: workspace_identity.clone(),
                        workspace_root: workspace_root.clone(),
                        domain,
                        intent: crate::io::AgentEditIntent {
                            path: path.clone(),
                            snapshot_id: managed_io_snapshot_id_from_arg(args.snapshot_id),
                            operation,
                        },
                    },
                );
                coordinator.release_reservation(reservation);
                record_managed_io_external_change_if_rejected(
                    &self.owned.managed_io_external_changes,
                    &workspace_identity,
                    &path,
                    &result,
                );
                record_managed_io_write_if_applied(
                    &self.owned.managed_io_external_changes,
                    provider_run.id(),
                    &workspace_identity,
                    &workspace_root,
                    &path,
                    &result,
                );
                let after = managed_io_result_applied(&result)
                    .then(|| managed_io_text_for_diff(&workspace_root, &path, true))
                    .flatten();
                let mut output = managed_io_edit_result(
                    result,
                    ManagedIoChangeContext {
                        path,
                        before,
                        after,
                    },
                    external_change_notice,
                );
                add_managed_io_workspace_payload(&mut output.payload, &workspace_context);
                Ok(output)
            }
            crate::transport::runtime_tools::APPLY_PATCH_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManagedApplyPatchArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_apply_patch",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain =
                    KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
                if domain != crate::io::ArtifactDomainKind::TextDocument {
                    return Err(DaemonError::LocalTransport {
                        operation: "runtime_tool_apply_patch",
                        message: "managed apply_patch currently supports only text artifacts"
                            .to_string(),
                    });
                }
                let operations = parse_managed_apply_patch(&args.patch_text)?;
                let mut output = apply_managed_patch_operations(
                    &mut coordinator,
                    workspace_identity,
                    workspace_root.clone(),
                    domain,
                    operations,
                    managed_io_reservation_owner(provider_run, tool_name),
                    &self.owned.managed_io_external_changes,
                )?;
                add_managed_io_workspace_payload(&mut output.payload, &workspace_context);
                Ok(output)
            }
            crate::transport::runtime_tools::DELETE_ARTIFACT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManagedDeleteArtifactArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_delete_artifact",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain =
                    KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
                let mut output = if domain == crate::io::ArtifactDomainKind::TextDocument {
                    apply_managed_patch_operations(
                        &mut coordinator,
                        workspace_identity,
                        workspace_root.clone(),
                        domain,
                        vec![ManagedPatchOperation::Delete {
                            path: PathBuf::from(args.path),
                        }],
                        managed_io_reservation_owner(provider_run, tool_name),
                        &self.owned.managed_io_external_changes,
                    )?
                } else {
                    apply_managed_whole_file_operations(
                        &mut coordinator,
                        workspace_identity,
                        workspace_root.clone(),
                        domain,
                        vec![ManagedWholeFileOperation::Delete {
                            path: PathBuf::from(args.path),
                        }],
                        managed_io_reservation_owner(provider_run, tool_name),
                        &self.owned.managed_io_external_changes,
                    )?
                };
                add_managed_io_workspace_payload(&mut output.payload, &workspace_context);
                Ok(output)
            }
            crate::transport::runtime_tools::MOVE_ARTIFACT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManagedMoveArtifactArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_move_artifact",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain =
                    KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
                let mut output = if domain == crate::io::ArtifactDomainKind::TextDocument {
                    apply_managed_patch_operations(
                        &mut coordinator,
                        workspace_identity,
                        workspace_root.clone(),
                        domain,
                        vec![ManagedPatchOperation::Move {
                            from_path: PathBuf::from(args.from_path),
                            to_path: PathBuf::from(args.to_path),
                            old_text: args.old_text,
                            new_text: args.new_text,
                        }],
                        managed_io_reservation_owner(provider_run, tool_name),
                        &self.owned.managed_io_external_changes,
                    )?
                } else {
                    if args.has_non_text_transform_fields() {
                        return Err(DaemonError::LocalTransport {
                            operation: "runtime_tool_move_artifact",
                            message: "non-text managed moves cannot transform content; omit old_text and new_text".to_string(),
                        });
                    }
                    apply_managed_whole_file_operations(
                        &mut coordinator,
                        workspace_identity,
                        workspace_root.clone(),
                        domain,
                        vec![ManagedWholeFileOperation::Move {
                            from_path: PathBuf::from(args.from_path),
                            to_path: PathBuf::from(args.to_path),
                        }],
                        managed_io_reservation_owner(provider_run, tool_name),
                        &self.owned.managed_io_external_changes,
                    )?
                };
                add_managed_io_workspace_payload(&mut output.payload, &workspace_context);
                Ok(output)
            }
            crate::transport::runtime_tools::WRITE_ARTIFACT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManagedWriteArtifactArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_write_artifact",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain =
                    KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
                let path = PathBuf::from(args.path.clone());
                let before = managed_io_text_for_diff(&workspace_root, &path, true);
                let content = managed_io_write_content_from_args(
                    "runtime_tool_write_artifact",
                    domain,
                    &args,
                )?;
                let reservation = match managed_io_try_reserve_ranges(
                    &mut coordinator,
                    &workspace_identity,
                    &path,
                    vec![crate::io::TextRange::new(0, usize::MAX)],
                    managed_io_reservation_owner(provider_run, tool_name),
                ) {
                    Ok(reservation) => reservation,
                    Err(mut output) => {
                        add_managed_io_workspace_payload(&mut output.payload, &workspace_context);
                        return Ok(output);
                    }
                };
                let external_change_notice = self
                    .owned
                    .managed_io_external_changes
                    .external_change_notice(&workspace_identity, &path);
                let result = crate::io::ManagedFileIo::apply_edit(
                    &mut coordinator,
                    crate::io::ManagedFileWriteRequest {
                        workspace_identity: workspace_identity.clone(),
                        workspace_root: workspace_root.clone(),
                        domain,
                        intent: crate::io::AgentEditIntent {
                            path: path.clone(),
                            snapshot_id: managed_io_write_snapshot_id_from_arg(
                                args.snapshot_id,
                                &path,
                            ),
                            operation: crate::io::AgentEditOperation::WriteArtifact { content },
                        },
                    },
                );
                coordinator.release_reservation(reservation);
                record_managed_io_external_change_if_rejected(
                    &self.owned.managed_io_external_changes,
                    &workspace_identity,
                    &path,
                    &result,
                );
                record_managed_io_write_if_applied(
                    &self.owned.managed_io_external_changes,
                    provider_run.id(),
                    &workspace_identity,
                    &workspace_root,
                    &path,
                    &result,
                );
                let after = managed_io_result_applied(&result)
                    .then(|| managed_io_text_for_diff(&workspace_root, &path, true))
                    .flatten();
                let mut output = managed_io_edit_result(
                    result,
                    ManagedIoChangeContext {
                        path,
                        before,
                        after,
                    },
                    external_change_notice,
                );
                add_managed_io_workspace_payload(&mut output.payload, &workspace_context);
                Ok(output)
            }
            other => Err(DaemonError::LocalTransport {
                operation: "dispatch_managed_io_runtime_tool_call",
                message: format!("unsupported managed I/O tool `{other}`"),
            }),
        }
    }

    async fn try_dispatch_remote_managed_io_runtime_tool_call(
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
        let session = owned.session_store.get_session(&context.home_session_id)?;
        let Some(active_prompt) = owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &context.home_agent_id)
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

    pub(crate) async fn dispatch_forwarded_managed_io_runtime_tool_call(
        &self,
        context: crate::transport::relay_peer::RemoteManagedIoContext,
        tool_name: String,
        arguments: serde_json::Value,
        artifact_states: Vec<crate::transport::relay_peer::RemoteManagedIoArtifactState>,
    ) -> Result<
        (
            crate::transport::runtime_tools::RuntimeToolResult,
            Vec<crate::transport::relay_peer::RemoteManagedIoArtifactState>,
        ),
        DaemonError,
    > {
        let session = self
            .owned
            .session_store
            .get_session(&context.home_session_id)?;
        let home_root = PathBuf::from(session.worktree_id());
        let home_identity = workspace_identity_for_root_off_thread(home_root.clone()).await?;
        if !managed_io_workspace_identities_match(
            &home_identity,
            &context.worker_workspace_identity,
        ) {
            let result = crate::transport::runtime_tools::RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "applied": false,
                    "reason": {
                        "kind": "remote_workspace_not_coordinated",
                        "message": "The remote agent workspace does not match the home session repo/branch, so Arroba will not coordinate this managed I/O operation through the home kernel."
                    },
                    "next_action": "Move the remote agent to the same repo and branch as the home session, then retry through Arroba managed I/O.",
                }),
            };
            return Ok((result, Vec::new()));
        }
        let workspace_context = ManagedIoWorkspaceContext {
            root: home_root,
            identity: context.worker_workspace_identity.clone(),
            generation: 0,
            identity_changed: false,
            valid: true,
        };
        let mut coordinator = self.owned.managed_io_coordinator.lock().await;
        match tool_name.as_str() {
            crate::transport::runtime_tools::READ_ARTIFACT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManagedReadArtifactArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "forwarded_managed_io_read_artifact",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain =
                    KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
                let state =
                    remote_managed_io_state_for_path(&artifact_states, &PathBuf::from(&args.path))
                        .ok_or_else(|| DaemonError::LocalTransport {
                            operation: "forwarded_managed_io_read_artifact",
                            message: "missing forwarded artifact state".to_string(),
                        })?;
                let content = remote_managed_io_content_from_state(state, domain)?;
                let read = coordinator.read_artifact(crate::io::ArtifactReadRequest {
                    workspace_identity: context.worker_workspace_identity,
                    path: PathBuf::from(args.path),
                    domain,
                    content,
                });
                let mut payload = managed_io_read_payload(read);
                add_managed_io_workspace_payload(&mut payload, &workspace_context);
                Ok((
                    crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload },
                    Vec::new(),
                ))
            }
            crate::transport::runtime_tools::EDIT_ARTIFACT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManagedEditArtifactArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "forwarded_managed_io_edit_artifact",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain =
                    KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
                if domain != crate::io::ArtifactDomainKind::TextDocument {
                    return Err(DaemonError::LocalTransport {
                        operation: "forwarded_managed_io_edit_artifact",
                        message: "remote managed edit currently supports only text artifacts"
                            .to_string(),
                    });
                }
                let operation = managed_io_edit_operation_from_args(args.clone())?;
                let path = PathBuf::from(args.path.clone());
                let state =
                    remote_managed_io_state_for_path(&artifact_states, &path).ok_or_else(|| {
                        DaemonError::LocalTransport {
                            operation: "forwarded_managed_io_edit_artifact",
                            message: "missing forwarded artifact state".to_string(),
                        }
                    })?;
                let before = remote_managed_io_text_snapshot_from_state(state);
                coordinator.read_artifact(crate::io::ArtifactReadRequest {
                    workspace_identity: context.worker_workspace_identity.clone(),
                    path: path.clone(),
                    domain,
                    content: remote_managed_io_content_from_state(state, domain)?,
                });
                let reservation = match managed_io_try_reserve_ranges(
                    &mut coordinator,
                    &context.worker_workspace_identity,
                    &path,
                    managed_io_reservation_ranges_for_operation(
                        &operation,
                        before.as_ref(),
                        crate::io::TextRange::new(0, usize::MAX),
                    ),
                    crate::io::ArtifactReservationOwner::new(
                        format!("remote:{}", context.worker_provider_run_id),
                        Some(context.home_agent_id.clone()),
                        tool_name.clone(),
                    ),
                ) {
                    Ok(reservation) => reservation,
                    Err(mut result) => {
                        add_managed_io_workspace_payload(&mut result.payload, &workspace_context);
                        return Ok((result, Vec::new()));
                    }
                };
                let result = coordinator.apply_edit(crate::io::ArtifactWriteRequest {
                    workspace_identity: context.worker_workspace_identity,
                    intent: crate::io::AgentEditIntent {
                        path: path.clone(),
                        snapshot_id: managed_io_snapshot_id_from_arg(args.snapshot_id.clone()),
                        operation,
                    },
                });
                coordinator.release_reservation(reservation);
                let after = managed_io_result_applied(&result)
                    .then(|| {
                        let artifact_id =
                            coordinator.resolve_artifact_id(&workspace_context.identity, &path);
                        coordinator
                            .current_content(&artifact_id)
                            .and_then(|content| content.as_text().map(str::to_string))
                            .map(|text| ManagedIoTextSnapshot {
                                existed: true,
                                text,
                            })
                    })
                    .flatten();
                let final_states = after
                    .as_ref()
                    .map(|after| vec![remote_managed_io_state(&path, Some(after.text.clone()))])
                    .unwrap_or_default();
                let mut output = managed_io_edit_result(
                    result,
                    ManagedIoChangeContext {
                        path,
                        before,
                        after,
                    },
                    None,
                );
                add_managed_io_workspace_payload(&mut output.payload, &workspace_context);
                Ok((output, final_states))
            }
            crate::transport::runtime_tools::WRITE_ARTIFACT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManagedWriteArtifactArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "forwarded_managed_io_write_artifact",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain =
                    KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
                let path = PathBuf::from(args.path.clone());
                let state =
                    remote_managed_io_state_for_path(&artifact_states, &path).ok_or_else(|| {
                        DaemonError::LocalTransport {
                            operation: "forwarded_managed_io_write_artifact",
                            message: "missing forwarded artifact state".to_string(),
                        }
                    })?;
                let before = remote_managed_io_text_snapshot_from_state(state);
                coordinator.read_artifact(crate::io::ArtifactReadRequest {
                    workspace_identity: context.worker_workspace_identity.clone(),
                    path: path.clone(),
                    domain,
                    content: remote_managed_io_content_from_state(state, domain)?,
                });
                let reservation = match managed_io_try_reserve_ranges(
                    &mut coordinator,
                    &context.worker_workspace_identity,
                    &path,
                    vec![crate::io::TextRange::new(0, usize::MAX)],
                    crate::io::ArtifactReservationOwner::new(
                        format!("remote:{}", context.worker_provider_run_id),
                        Some(context.home_agent_id.clone()),
                        tool_name.clone(),
                    ),
                ) {
                    Ok(reservation) => reservation,
                    Err(mut result) => {
                        add_managed_io_workspace_payload(&mut result.payload, &workspace_context);
                        return Ok((result, Vec::new()));
                    }
                };
                let result = coordinator.apply_edit(crate::io::ArtifactWriteRequest {
                    workspace_identity: context.worker_workspace_identity,
                    intent: crate::io::AgentEditIntent {
                        path: path.clone(),
                        snapshot_id: managed_io_write_snapshot_id_from_arg(
                            args.snapshot_id.clone(),
                            &path,
                        ),
                        operation: crate::io::AgentEditOperation::WriteArtifact {
                            content: managed_io_write_content_from_args(
                                "forwarded_managed_io_write_artifact",
                                domain,
                                &args,
                            )?,
                        },
                    },
                });
                coordinator.release_reservation(reservation);
                let (after, final_states) = if managed_io_result_applied(&result) {
                    let artifact_id =
                        coordinator.resolve_artifact_id(&workspace_context.identity, &path);
                    let content = coordinator.current_content(&artifact_id).cloned();
                    let after = content.as_ref().and_then(|content| match content {
                        crate::io::ArtifactContent::Text(text) => Some(ManagedIoTextSnapshot {
                            existed: true,
                            text: text.clone(),
                        }),
                        crate::io::ArtifactContent::Bytes(_) => None,
                    });
                    let final_states = content
                        .map(|content| {
                            vec![remote_managed_io_state_from_content_with_domain(
                                &path,
                                Some(content),
                                domain,
                            )]
                        })
                        .unwrap_or_default();
                    (after, final_states)
                } else {
                    (None, Vec::new())
                };
                let mut output = managed_io_edit_result(
                    result,
                    ManagedIoChangeContext {
                        path,
                        before,
                        after,
                    },
                    None,
                );
                add_managed_io_workspace_payload(&mut output.payload, &workspace_context);
                Ok((output, final_states))
            }
            crate::transport::runtime_tools::APPLY_PATCH_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManagedApplyPatchArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "forwarded_managed_io_apply_patch",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain =
                    KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
                if domain != crate::io::ArtifactDomainKind::TextDocument {
                    return Err(DaemonError::LocalTransport {
                        operation: "forwarded_managed_io_apply_patch",
                        message:
                            "remote managed apply_patch currently supports only text artifacts"
                                .to_string(),
                    });
                }
                let operations = parse_managed_apply_patch(&args.patch_text)?;
                apply_remote_managed_patch_operations(
                    &mut coordinator,
                    context.worker_workspace_identity,
                    domain,
                    operations,
                    artifact_states,
                    crate::io::ArtifactReservationOwner::new(
                        format!("remote:{}", context.worker_provider_run_id),
                        Some(context.home_agent_id),
                        tool_name,
                    ),
                    &workspace_context,
                )
            }
            crate::transport::runtime_tools::DELETE_ARTIFACT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManagedDeleteArtifactArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "forwarded_managed_io_delete_artifact",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain =
                    KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
                if domain == crate::io::ArtifactDomainKind::TextDocument {
                    apply_remote_managed_patch_operations(
                        &mut coordinator,
                        context.worker_workspace_identity,
                        domain,
                        vec![ManagedPatchOperation::Delete {
                            path: PathBuf::from(args.path),
                        }],
                        artifact_states,
                        crate::io::ArtifactReservationOwner::new(
                            format!("remote:{}", context.worker_provider_run_id),
                            Some(context.home_agent_id),
                            tool_name,
                        ),
                        &workspace_context,
                    )
                } else {
                    apply_remote_managed_whole_file_operations(
                        &mut coordinator,
                        context.worker_workspace_identity,
                        domain,
                        vec![ManagedWholeFileOperation::Delete {
                            path: PathBuf::from(args.path),
                        }],
                        artifact_states,
                        crate::io::ArtifactReservationOwner::new(
                            format!("remote:{}", context.worker_provider_run_id),
                            Some(context.home_agent_id),
                            tool_name,
                        ),
                        &workspace_context,
                    )
                }
            }
            crate::transport::runtime_tools::MOVE_ARTIFACT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManagedMoveArtifactArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "forwarded_managed_io_move_artifact",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain =
                    KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
                if domain == crate::io::ArtifactDomainKind::TextDocument {
                    apply_remote_managed_patch_operations(
                        &mut coordinator,
                        context.worker_workspace_identity,
                        domain,
                        vec![ManagedPatchOperation::Move {
                            from_path: PathBuf::from(args.from_path),
                            to_path: PathBuf::from(args.to_path),
                            old_text: args.old_text,
                            new_text: args.new_text,
                        }],
                        artifact_states,
                        crate::io::ArtifactReservationOwner::new(
                            format!("remote:{}", context.worker_provider_run_id),
                            Some(context.home_agent_id),
                            tool_name,
                        ),
                        &workspace_context,
                    )
                } else {
                    if args.has_non_text_transform_fields() {
                        return Err(DaemonError::LocalTransport {
                            operation: "forwarded_managed_io_move_artifact",
                            message: "non-text managed moves cannot transform content; omit old_text and new_text".to_string(),
                        });
                    }
                    apply_remote_managed_whole_file_operations(
                        &mut coordinator,
                        context.worker_workspace_identity,
                        domain,
                        vec![ManagedWholeFileOperation::Move {
                            from_path: PathBuf::from(args.from_path),
                            to_path: PathBuf::from(args.to_path),
                        }],
                        artifact_states,
                        crate::io::ArtifactReservationOwner::new(
                            format!("remote:{}", context.worker_provider_run_id),
                            Some(context.home_agent_id),
                            tool_name,
                        ),
                        &workspace_context,
                    )
                }
            }
            _ => Ok((
                crate::transport::runtime_tools::RuntimeToolResult {
                    ok: false,
                    payload: serde_json::json!({
                        "applied": false,
                        "reason": {
                            "kind": "unsupported_remote_managed_io_tool",
                            "message": format!("remote coordinated managed I/O does not yet support `{tool_name}`")
                        },
                        "next_action": "Use arroba.read_artifact, arroba.edit_artifact, or arroba.write_artifact for remote coordinated text edits until patch/move/delete remote routing lands.",
                    }),
                },
                Vec::new(),
            )),
        }
    }
}

fn skill_registry_for_workspace(workspace: &str) -> crate::skill::ArrobaSkillRegistry {
    let workspace = std::path::PathBuf::from(workspace);
    let mut roots = vec![crate::skill::ArrobaSkillRegistry::project_root(&workspace)];
    if let Some(user_root) = crate::skill::ArrobaSkillRegistry::user_root() {
        roots.push(user_root);
    }
    crate::skill::ArrobaSkillRegistry::new(roots)
}

fn mcp_registry_for_workspace(workspace: &str) -> crate::mcp::ArrobaMcpRegistry {
    let workspace = std::path::PathBuf::from(workspace);
    let mut roots = vec![crate::mcp::ArrobaMcpRegistry::project_root(&workspace)];
    if let Some(user_root) = crate::mcp::ArrobaMcpRegistry::user_root() {
        roots.push(user_root);
    }
    crate::mcp::ArrobaMcpRegistry::new(roots)
}

fn required_remote_mcps(
    registry: &crate::mcp::ArrobaMcpRegistry,
    grants: &[String],
) -> Result<Vec<crate::transport::relay_peer::RequiredRemoteMcp>, DaemonError> {
    grants
        .iter()
        .map(|grant| {
            let config = registry
                .get(grant)?
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "required remote MCPs",
                    message: format!("MCP `{grant}` is granted but is not installed"),
                })?;
            let definition_hash = config.definition_hash()?;
            Ok(crate::transport::relay_peer::RequiredRemoteMcp {
                config,
                definition_hash,
            })
        })
        .collect()
}

fn format_remote_mcp_unavailable(
    unavailable: &[&crate::transport::relay_peer::RemoteMcpAvailability],
) -> String {
    let details =
        unavailable
            .iter()
            .map(|entry| {
                let status = match &entry.status {
                crate::transport::relay_peer::RemoteMcpAvailabilityStatus::Available => {
                    "available".to_string()
                }
                crate::transport::relay_peer::RemoteMcpAvailabilityStatus::Missing => {
                    "missing on worker".to_string()
                }
                crate::transport::relay_peer::RemoteMcpAvailabilityStatus::DefinitionMismatch {
                    worker_hash,
                } => format!("definition mismatch; worker has {worker_hash}"),
                crate::transport::relay_peer::RemoteMcpAvailabilityStatus::MissingCommand {
                    command,
                } => format!("missing command `{command}` on worker"),
                crate::transport::relay_peer::RemoteMcpAvailabilityStatus::MissingEnv { names } => {
                    format!("missing environment variable(s) on worker: {}", names.join(", "))
                }
                crate::transport::relay_peer::RemoteMcpAvailabilityStatus::Invalid { reason } => {
                    reason.clone()
                }
            };
                format!(
                    "- {} expected hash {}: {}",
                    entry.name, entry.expected_hash, status
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
    format!(
        "remote MCP unavailable on worker. Install the matching MCP definition in the worker project or user registry, then retry.\n{details}"
    )
}

fn package_granted_skills(
    registry: &crate::skill::ArrobaSkillRegistry,
    grants: &[String],
) -> Result<Vec<crate::skill::ArrobaSkillPackage>, DaemonError> {
    grants
        .iter()
        .map(|grant| {
            registry
                .package(grant)?
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "package granted skill",
                    message: format!("skill `{grant}` is granted but is not installed"),
                })
        })
        .collect()
}
