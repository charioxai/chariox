use super::request::*;
use super::result::*;
use super::spawn_args::*;
use super::*;

impl CommandRouter {
    pub(in crate::runtime::router) async fn dispatch_meta_run_command(
        &self,
        auth_token: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let (provider_run, session, metaagent) = self
            .runtime_state
            .metaagent_context_for_auth_token(auth_token)?;
        let args = serde_json::from_value::<MetaRunCommandArgs>(arguments).map_err(|error| {
            DaemonError::LocalTransport {
                operation: "runtime_tool_meta.run_command",
                message: format!("invalid tool arguments: {error}"),
            }
        })?;
        let tokens =
            match crate::runtime::metaagent_command_registry::tokenize_command(&args.command) {
                Ok(tokens) => tokens,
                Err(error) => {
                    let result = meta_command_failure_result(
                        &args.command,
                        meta_command_error(error.message()),
                    );
                    self.audit_meta_run_command(
                        Some(provider_run.id()),
                        &session,
                        &metaagent,
                        &args.command,
                        "invalid",
                        result.payload.clone(),
                    )
                    .await;
                    return Ok(result);
                }
            };
        match crate::runtime::metaagent_command_registry::execution_policy(&tokens) {
            crate::runtime::metaagent_command_registry::MetaCommandExecutionPolicy::Routed => {}
            crate::runtime::metaagent_command_registry::MetaCommandExecutionPolicy::Denied {
                message,
            } => {
                let result =
                    meta_command_failure_result(&args.command, meta_command_error(message));
                self.audit_meta_run_command(
                    Some(provider_run.id()),
                    &session,
                    &metaagent,
                    &args.command,
                    "denied",
                    result.payload.clone(),
                )
                .await;
                return Ok(result);
            }
            crate::runtime::metaagent_command_registry::MetaCommandExecutionPolicy::NotRouted {
                message,
            } => {
                let result =
                    meta_command_failure_result(&args.command, meta_command_error(message));
                self.audit_meta_run_command(
                    Some(provider_run.id()),
                    &session,
                    &metaagent,
                    &args.command,
                    "not_routed",
                    result.payload.clone(),
                )
                .await;
                return Ok(result);
            }
        }
        if meta_command_requires_task_plan(&tokens)
            && metaagent_active_task_plan_is_empty(&session, &metaagent)
        {
            let result =
                meta_command_failure_result(&args.command, meta_task_plan_required_error());
            self.audit_meta_run_command(
                Some(provider_run.id()),
                &session,
                &metaagent,
                &args.command,
                "denied",
                result.payload.clone(),
            )
            .await;
            return Ok(result);
        }
        if tokens.first().map(String::as_str) == Some("prompt") {
            let result = self
                .dispatch_meta_prompt_command(&session, &metaagent, &args.command, &tokens[1..])
                .await?;
            self.audit_meta_run_command(
                Some(provider_run.id()),
                &session,
                &metaagent,
                &args.command,
                if result.ok { "succeeded" } else { "failed" },
                result.payload.clone(),
            )
            .await;
            return Ok(result);
        }
        if tokens.first().map(String::as_str) == Some("agent")
            && tokens.get(1).map(String::as_str) == Some("spawn")
        {
            let result = Box::pin(self.dispatch_meta_agent_spawn_command(
                Some(&provider_run),
                &session,
                &metaagent,
                &args.command,
                &tokens[2..],
            ))
            .await?;
            self.audit_meta_run_command(
                Some(provider_run.id()),
                &session,
                &metaagent,
                &args.command,
                if result.ok { "succeeded" } else { "failed" },
                result.payload.clone(),
            )
            .await;
            return Ok(result);
        }
        let request = match self
            .meta_command_request(&session, &metaagent, &tokens)
            .await
        {
            Ok(request) => request,
            Err(error) => {
                let result = meta_command_failure_result(&args.command, error);
                self.audit_meta_run_command(
                    Some(provider_run.id()),
                    &session,
                    &metaagent,
                    &args.command,
                    "failed",
                    result.payload.clone(),
                )
                .await;
                return Ok(result);
            }
        };
        let command = meta_kernel_command(Some(&provider_run), &metaagent, &request);
        let response = match self.dispatch(command, request).await {
            Ok(response) => response,
            Err(error) => {
                let result = meta_command_failure_result(&args.command, error);
                self.audit_meta_run_command(
                    Some(provider_run.id()),
                    &session,
                    &metaagent,
                    &args.command,
                    "failed",
                    result.payload.clone(),
                )
                .await;
                return Ok(result);
            }
        };
        let result = meta_command_success_result(&args.command, &response, &metaagent);
        self.audit_meta_run_command(
            Some(provider_run.id()),
            &session,
            &metaagent,
            &args.command,
            "succeeded",
            result.payload.clone(),
        )
        .await;
        Ok(result)
    }

    pub(in crate::runtime::router) async fn dispatch_forwarded_meta_run_command(
        &self,
        context: crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let (session, metaagent) = self
            .runtime_state
            .session_agent_snapshot(&context.home_session_id, &context.home_agent_id)?;
        let Some(remote) = metaagent.remote_execution() else {
            return Err(meta_command_error(format!(
                "home agent `{}` is not remote-backed",
                context.home_agent_id
            )));
        };
        if !metaagent.is_metaagent()
            || metaagent.session_id() != context.home_session_id
            || remote.leased_agent_id != context.leased_agent_id
            || remote.worker_kernel_id != context.worker_kernel_id
        {
            return Err(meta_command_error(
                "forwarded metaagent context does not match a home remote metaagent",
            ));
        }
        let args = serde_json::from_value::<MetaRunCommandArgs>(arguments).map_err(|error| {
            DaemonError::LocalTransport {
                operation: "runtime_tool_meta.run_command",
                message: format!("invalid tool arguments: {error}"),
            }
        })?;
        let tokens =
            match crate::runtime::metaagent_command_registry::tokenize_command(&args.command) {
                Ok(tokens) => tokens,
                Err(error) => {
                    let result = meta_command_failure_result(
                        &args.command,
                        meta_command_error(error.message()),
                    );
                    self.audit_meta_run_command(
                        None,
                        &session,
                        &metaagent,
                        &args.command,
                        "invalid",
                        result.payload.clone(),
                    )
                    .await;
                    return Ok(result);
                }
            };
        match crate::runtime::metaagent_command_registry::execution_policy(&tokens) {
            crate::runtime::metaagent_command_registry::MetaCommandExecutionPolicy::Routed => {}
            crate::runtime::metaagent_command_registry::MetaCommandExecutionPolicy::Denied {
                message,
            } => {
                let result =
                    meta_command_failure_result(&args.command, meta_command_error(message));
                self.audit_meta_run_command(
                    None,
                    &session,
                    &metaagent,
                    &args.command,
                    "denied",
                    result.payload.clone(),
                )
                .await;
                return Ok(result);
            }
            crate::runtime::metaagent_command_registry::MetaCommandExecutionPolicy::NotRouted {
                message,
            } => {
                let result =
                    meta_command_failure_result(&args.command, meta_command_error(message));
                self.audit_meta_run_command(
                    None,
                    &session,
                    &metaagent,
                    &args.command,
                    "not_routed",
                    result.payload.clone(),
                )
                .await;
                return Ok(result);
            }
        }
        if meta_command_requires_task_plan(&tokens)
            && metaagent_active_task_plan_is_empty(&session, &metaagent)
        {
            let result =
                meta_command_failure_result(&args.command, meta_task_plan_required_error());
            self.audit_meta_run_command(
                None,
                &session,
                &metaagent,
                &args.command,
                "denied",
                result.payload.clone(),
            )
            .await;
            return Ok(result);
        }
        if tokens.first().map(String::as_str) == Some("prompt") {
            let result = self
                .dispatch_meta_prompt_command(&session, &metaagent, &args.command, &tokens[1..])
                .await?;
            self.audit_meta_run_command(
                None,
                &session,
                &metaagent,
                &args.command,
                if result.ok { "succeeded" } else { "failed" },
                result.payload.clone(),
            )
            .await;
            return Ok(result);
        }
        if tokens.first().map(String::as_str) == Some("agent")
            && tokens.get(1).map(String::as_str) == Some("spawn")
        {
            let result = Box::pin(self.dispatch_meta_agent_spawn_command(
                None,
                &session,
                &metaagent,
                &args.command,
                &tokens[2..],
            ))
            .await?;
            self.audit_meta_run_command(
                None,
                &session,
                &metaagent,
                &args.command,
                if result.ok { "succeeded" } else { "failed" },
                result.payload.clone(),
            )
            .await;
            return Ok(result);
        }
        let request = match self
            .meta_command_request(&session, &metaagent, &tokens)
            .await
        {
            Ok(request) => request,
            Err(error) => {
                let result = meta_command_failure_result(&args.command, error);
                self.audit_meta_run_command(
                    None,
                    &session,
                    &metaagent,
                    &args.command,
                    "failed",
                    result.payload.clone(),
                )
                .await;
                return Ok(result);
            }
        };
        let command = meta_kernel_command(None, &metaagent, &request);
        let response = match self.dispatch(command, request).await {
            Ok(response) => response,
            Err(error) => {
                let result = meta_command_failure_result(&args.command, error);
                self.audit_meta_run_command(
                    None,
                    &session,
                    &metaagent,
                    &args.command,
                    "failed",
                    result.payload.clone(),
                )
                .await;
                return Ok(result);
            }
        };
        let result = meta_command_success_result(&args.command, &response, &metaagent);
        self.audit_meta_run_command(
            None,
            &session,
            &metaagent,
            &args.command,
            "succeeded",
            result.payload.clone(),
        )
        .await;
        Ok(result)
    }

    async fn audit_meta_run_command(
        &self,
        provider_run_id: Option<&str>,
        session: &crate::session::RuntimeSession,
        metaagent: &crate::agent::AgentInstance,
        command: &str,
        status: &str,
        result: serde_json::Value,
    ) {
        let timestamp_ms = crate::session::unix_epoch_ms();
        let causation_id = provider_run_id.unwrap_or_else(|| metaagent.id());
        let correlation_id = format!("metaagent:{}:command:{timestamp_ms}", metaagent.id());
        let display_command = redacted_meta_command_for_payload(command);
        if let Err(error) = self.runtime_state.append_metaagent_command_audit_event(
            metaagent.id(),
            serde_json::json!({
                "session_id": session.id(),
                "user_id": metaagent.owner_user_id(),
                "metaagent_id": metaagent.id(),
                "provider_run_id": provider_run_id,
                "command": display_command,
                "status": status,
                "result": result,
                "causation_id": causation_id,
                "correlation_id": correlation_id,
                "timestamp_ms": timestamp_ms,
            }),
        ) {
            crate::logging::warn_with_fields(
                "metaagent.audit",
                "failed to persist metaagent command audit",
                serde_json::json!({
                    "session_id": session.id(),
                    "metaagent_id": metaagent.id(),
                    "command": display_command,
                    "status": status,
                    "error": error.to_string(),
                }),
            );
        }
    }

    async fn dispatch_meta_prompt_command(
        &self,
        session: &crate::session::RuntimeSession,
        metaagent: &crate::agent::AgentInstance,
        command: &str,
        args: &[String],
    ) -> Result<RuntimeToolResult, DaemonError> {
        if args.len() < 2 {
            return Ok(meta_command_failure_result(
                command,
                meta_command_error("usage: prompt <owned-agent-ref> <prompt text>"),
            ));
        }
        let target = match self
            .resolve_owned_regular_agent(session.id(), metaagent, &args[0])
            .await
        {
            Ok(target) => target,
            Err(error) => return Ok(meta_command_failure_result(command, error)),
        };
        if let Some(flag) = args[1..]
            .iter()
            .find(|arg| matches!(arg.as_str(), "--wait" | "--show-reply" | "--show-summary"))
        {
            return Ok(meta_command_failure_result(
                command,
                meta_command_error(format!(
                    "Meta-mode prompt delegation does not support blocking reply flags (`{flag}`); use events and turn_overview"
                )),
            ));
        }
        let prompt = args[1..].join(" ");
        let attachment_id = match self
            .ensure_metaagent_command_attachment(session.id(), metaagent)
            .await
        {
            Ok(attachment_id) => attachment_id,
            Err(error) => return Ok(meta_command_failure_result(command, error)),
        };
        let mut result = match self
            .runtime_state
            .submit_metaagent_command_prompt(
                session.id(),
                metaagent,
                &attachment_id,
                target.id(),
                prompt,
            )
            .await
        {
            Ok(result) => result,
            Err(error) => return Ok(meta_command_failure_result(command, error)),
        };
        if let Some(payload) = result.payload.as_object_mut() {
            payload.insert(
                "command".to_string(),
                serde_json::Value::String(command.to_string()),
            );
            payload.insert(
                "target_agent_id".to_string(),
                serde_json::Value::String(target.id().to_string()),
            );
            payload.insert(
                "target_agent_ref".to_string(),
                serde_json::Value::String(target.agent_ref().to_string()),
            );
        }
        Ok(result)
    }

    async fn dispatch_meta_agent_spawn_command(
        &self,
        provider_run: Option<&crate::provider::RuntimeProviderRun>,
        session: &crate::session::RuntimeSession,
        metaagent: &crate::agent::AgentInstance,
        command: &str,
        args: &[String],
    ) -> Result<RuntimeToolResult, DaemonError> {
        let mut spawn = match parse_meta_agent_spawn_args(args, session) {
            Ok(spawn) => spawn,
            Err(error) => return Ok(meta_command_failure_result(command, error)),
        };

        let mut created_slice = None;
        if let Some(slice_create) = spawn.slice_create.take() {
            let worktree_id = spawn
                .worktree_id
                .clone()
                .or_else(|| metaagent.worktree_id().map(str::to_string))
                .unwrap_or_else(|| session.worktree_id().to_string());
            let create_request = LocalDaemonRequest::CreateSlice(CreateSliceRequest {
                name: metaagent_spawn_slice_name(spawn.alias.as_deref()),
                backend: crate::slice::SliceBackendKind::LocalDocker,
                os: "linux".to_string(),
                display_mode: slice_create.display_mode,
                display_backend: Default::default(),
                workspace_id: Some(session.workspace_id().to_string()),
                worktree_id: Some(worktree_id.clone()),
                workspace_mount: Some(worktree_id),
                worker_kernel_ref: spawn.kernel_ref.clone(),
                display_url: None,
                provider_auth: Vec::new(),
                from_saved_state: None,
                base: None,
            });
            let create_response = match Box::pin(self.dispatch(
                meta_kernel_command(provider_run, metaagent, &create_request),
                create_request,
            ))
            .await
            {
                Ok(response) => response,
                Err(error) => return Ok(meta_command_failure_result(command, error)),
            };
            let slice = match create_response {
                LocalDaemonResponse::SliceCreated { slice } => slice,
                other => {
                    return Ok(meta_command_failure_result(
                        command,
                        meta_command_error(format!("unexpected slice create response: {other:?}")),
                    ));
                }
            };
            let start_request = LocalDaemonRequest::StartSlice(SliceRefRequest {
                slice_ref: slice.id.clone(),
            });
            let start_response = match Box::pin(self.dispatch(
                meta_kernel_command(provider_run, metaagent, &start_request),
                start_request,
            ))
            .await
            {
                Ok(response) => response,
                Err(error) => return Ok(meta_command_failure_result(command, error)),
            };
            let slice = match start_response {
                LocalDaemonResponse::SliceStarted { slice } => slice,
                other => {
                    return Ok(meta_command_failure_result(
                        command,
                        meta_command_error(format!("unexpected slice start response: {other:?}")),
                    ));
                }
            };
            spawn.slice_ref = Some(slice.id.clone());
            spawn.kernel_ref = None;
            created_slice = Some(slice);
        }

        let request = LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: spawn.alias,
            provider: spawn
                .provider
                .or_else(|| Some(metaagent.provider().to_string())),
            account_profile: metaagent.account_profile().map(str::to_string),
            model: spawn
                .model
                .or_else(|| metaagent.model().map(str::to_string)),
            effort: spawn
                .effort
                .or_else(|| metaagent.effort().map(str::to_string)),
            execution_mode: metaagent.execution_mode_override(),
            permission_level: metaagent.permission_level_override(),
            worktree_id: spawn
                .worktree_id
                .or_else(|| metaagent.worktree_id().map(str::to_string)),
            kernel_ref: if spawn.slice_ref.is_some() {
                None
            } else {
                spawn.kernel_ref
            },
            slice_ref: spawn.slice_ref,
            worktree_placement: spawn.worktree_placement,
            metaagent: false,
        });
        let response = match Box::pin(self.dispatch(
            meta_kernel_command(provider_run, metaagent, &request),
            request,
        ))
        .await
        {
            Ok(response) => response,
            Err(error) => return Ok(meta_command_failure_result(command, error)),
        };
        let mut result = meta_command_success_result(command, &response, metaagent);
        if let Some(slice) = created_slice {
            if let Some(payload) = result.payload.as_object_mut() {
                payload.insert("created_slice".to_string(), serde_json::json!(slice));
            }
        }
        Ok(result)
    }

    async fn meta_command_request(
        &self,
        session: &crate::session::RuntimeSession,
        metaagent: &crate::agent::AgentInstance,
        tokens: &[String],
    ) -> Result<LocalDaemonRequest, DaemonError> {
        let Some(command) = tokens.first().map(String::as_str) else {
            return Err(meta_command_error("empty Meta-mode command"));
        };
        match command {
            "agent" => {
                let agents = self.runtime_state.session_agents(session.id());
                meta_agent_request(session, metaagent, &tokens[1..], &agents)
            }
            "workflow" => {
                let agents = self.runtime_state.session_agents(session.id());
                meta_workflow_request(session, metaagent, &tokens[1..], &agents)
            }
            "slice" => meta_slice_request(&tokens[1..]),
            "mcp" => {
                let agents = self.runtime_state.session_agents(session.id());
                meta_extension_request(
                    session,
                    metaagent,
                    ExtensionKind::Mcp,
                    "mcp",
                    &tokens[1..],
                    &agents,
                )
            }
            "skill" | "skills" => {
                let agents = self.runtime_state.session_agents(session.id());
                meta_extension_request(
                    session,
                    metaagent,
                    ExtensionKind::Skill,
                    "skill",
                    &tokens[1..],
                    &agents,
                )
            }
            "extension" | "extensions" => meta_extension_import_request(session, &tokens[1..]),
            "credential" | "credentials" => {
                meta_credential_request(session, metaagent, &tokens[1..])
            }
            other => Err(meta_command_error(format!(
                "`{other}` is registered but not implemented by the Meta-mode command router"
            ))),
        }
    }

    async fn ensure_metaagent_command_attachment(
        &self,
        session_id: &str,
        metaagent: &crate::agent::AgentInstance,
    ) -> Result<String, DaemonError> {
        let client_id = metaagent_command_client_id(metaagent.id());
        if let Some(attachment) = self
            .runtime_state
            .client_attachment_for_session(&client_id, session_id)
        {
            return Ok(attachment.id().to_string());
        }
        let request = LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
            session_id: session_id.to_string(),
            client_id,
            capability_level: ClientCapabilityLevel::AutomationOnly,
        });
        let command = meta_kernel_command_without_request(metaagent, &request);
        match self.dispatch(command, request).await? {
            LocalDaemonResponse::SessionAttached { attachment } => Ok(attachment.id().to_string()),
            other => Err(DaemonError::LocalTransport {
                operation: "runtime_tool_meta.run_command",
                message: format!("unexpected attachment response: {other:?}"),
            }),
        }
    }

    async fn resolve_owned_regular_agent(
        &self,
        session_id: &str,
        metaagent: &crate::agent::AgentInstance,
        reference: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agents = self.runtime_state.session_agents(session_id);
        let owned_agents = agents
            .into_iter()
            .filter(|agent| {
                !agent.is_metaagent() && agent.controlled_by_metaagent_id() == Some(metaagent.id())
            })
            .collect::<Vec<_>>();
        owned_agents
            .iter()
            .find(|agent| {
                agent.id() == reference
                    || agent.agent_ref() == reference
                    || agent.alias() == Some(reference)
            })
            .cloned()
            .ok_or_else(|| {
                meta_command_error(owned_regular_agent_error_message(reference, &owned_agents))
            })
    }
}
