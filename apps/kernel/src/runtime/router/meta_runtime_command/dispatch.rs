use super::request::*;
use super::result::*;
use super::spawn_args::*;
use super::summary::summarize_spawned_agent;
use super::*;

impl CommandRouter {
    pub(in crate::runtime::router) async fn dispatch_meta_run_command(
        &self,
        auth_token: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        self.runtime_state.authorize_current_external_command()?;
        let result = self
            .dispatch_meta_run_command_inner(auth_token, arguments)
            .await?;
        // MP-08 / MP-11: every result, including prompt delegation, is released
        // only to the retained caller that still owns the admitted provider run.
        self.runtime_state.authorize_current_external_command()?;
        Ok(result)
    }

    async fn dispatch_meta_run_command_inner(
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
        if self.runtime_state.room_agent_tools_enabled() {
            if let Err(error) = crate::runtime::room_tool_admission::command(&tokens) {
                return Ok(meta_command_failure_result(&args.command, error));
            }
        }
        if !self.runtime_state.room_agent_tools_enabled()
            && meta_command_requires_task_plan(&tokens)
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
        if let Some(result) = self
            .dispatch_meta_agent_tree_command(
                Some(&provider_run),
                &session,
                &metaagent,
                &args.command,
                &tokens,
            )
            .await?
        {
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
        let app_grant = match &request {
            LocalDaemonRequest::GrantAgentExtension(grant) if grant.kind == ExtensionKind::App => {
                Some(grant.clone())
            }
            _ => None,
        };
        let dispatched = match app_grant {
            Some(grant) => match self
                .runtime_state
                .authorize_agent_app_binding(
                    session.id(),
                    metaagent.id(),
                    &grant.agent_ref,
                    &grant.name,
                )
                .await
            {
                // Room mode refuses non-terminal App grants at the router, so
                // the approved permit binds here through the same grant path.
                Ok(Some(permit)) if self.runtime_state.room_agent_tools_enabled() => self
                    .runtime_state
                    .grant_agent_app(
                        &grant.agent_ref,
                        crate::extension::ExtensionGrant::app(&grant.name),
                        metaagent.owner_user_id(),
                        Some(permit),
                    )
                    .await
                    .map(|agent| LocalDaemonResponse::AgentExtensionGranted { agent }),
                Ok(Some(_)) => self.dispatch(command, request).await,
                Ok(None) => Err(meta_command_error("App binding was not approved")),
                Err(error) => Err(error),
            },
            None => self.dispatch(command, request).await,
        };
        let response = match dispatched {
            Ok(response) => response,
            Err(error) => {
                let result = meta_command_failure_result(&args.command, &error);
                self.audit_meta_run_command(
                    Some(provider_run.id()),
                    &session,
                    &metaagent,
                    &args.command,
                    "failed",
                    result.payload.clone(),
                )
                .await;
                // MP-08/MP-11: keep the refusal typed for the MCP/client code
                // projection after recording the normal failed-command audit.
                if matches!(error, DaemonError::UserDomainRefused { .. }) {
                    return Err(error);
                }
                return Ok(result);
            }
        };
        let result = meta_command_success_result(
            &args.command,
            &response,
            &metaagent,
            self.runtime_state.room_agent_tools_enabled(),
        );
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
        if (!metaagent.is_metaagent() && !self.runtime_state.room_agent_tools_enabled())
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
        if self.runtime_state.room_agent_tools_enabled() {
            if let Err(error) = crate::runtime::room_tool_admission::command(&tokens) {
                return Ok(meta_command_failure_result(&args.command, error));
            }
        }
        if !self.runtime_state.room_agent_tools_enabled()
            && meta_command_requires_task_plan(&tokens)
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
        if let Some(result) = self
            .dispatch_meta_agent_tree_command(None, &session, &metaagent, &args.command, &tokens)
            .await?
        {
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
        let result = meta_command_success_result(
            &args.command,
            &response,
            &metaagent,
            self.runtime_state.room_agent_tools_enabled(),
        );
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
        if let Err(error) = crate::runtime::room_tool_admission::in_subtree(
            metaagent,
            &target,
            &self.runtime_state.session_agents(session.id()),
        ) {
            return Ok(meta_command_failure_result(command, error));
        }
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
        // `--room` with `--kernel` names a room on that kernel; this kernel's own name keeps the
        // spawn here. Without `--room` another kernel's reference stays: a sub-agent runs there.
        if let Some(room) = spawn.room.clone() {
            if let Some(kernel) = spawn.kernel_ref.take() {
                match self.runtime_state.resolve_peer_kernel(&kernel).await {
                    Ok(Some(kernel)) => {
                        return Ok(self
                            .spawn_peer_on_kernel(metaagent, command, &kernel, room, spawn)
                            .await)
                    }
                    Ok(None) if spawn.command_id.is_some() => {
                        return Ok(meta_command_failure_result(
                            command,
                            meta_command_error(
                                "usage: agent spawn --room does not accept --command-id for a room on this kernel",
                            ),
                        ))
                    }
                    Ok(None) => {}
                    Err(error) => return Ok(meta_command_failure_result(command, error)),
                }
            }
        }

        // A guest of this room reaches no other room: every one reads as missing.
        if spawn
            .room
            .as_deref()
            .is_some_and(|room| room != session.id())
            && self.runtime_state.drives_as_room_guest(session, metaagent)
        {
            return Ok(meta_command_failure_result(
                command,
                crate::runtime::room_tool_admission::denied("the room for the peer does not exist"),
            ));
        }

        if spawn.deadline.is_some() {
            if let Err(error) = self
                .runtime_state
                .ensure_spawn_deadline_runs_here(
                    spawn.kernel_ref.as_deref(),
                    spawn.slice_ref.is_some() || spawn.slice_create.is_some(),
                )
                .await
            {
                return Ok(meta_command_failure_result(command, error));
            }
        }

        if let Some(reference) = spawn.slice_ref.as_deref() {
            match self
                .runtime_state
                .resolve_spawn_slice(reference, metaagent.id())
            {
                Ok(slice) => spawn.slice_ref = Some(slice.id),
                Err(error) => return Ok(meta_command_failure_result(command, error)),
            }
        }

        let mut created_slice = None;
        if let Some(slice_create) = spawn.slice_create.take() {
            let worktree_id = spawn
                .worktree_id
                .clone()
                .or_else(|| metaagent.worktree_id().map(str::to_string))
                .unwrap_or_else(|| session.worktree_id().to_string());
            let create_request = LocalDaemonRequest::CreateSlice(CreateSliceRequest {
                source_slice_ref: None,
                name: metaagent_spawn_slice_name(spawn.alias.as_deref()),
                backend: crate::slice::SliceBackendKind::LocalDocker,
                os: "linux".to_string(),
                display_mode: slice_create.display_mode,
                display_backend: Default::default(),
                workspace_id: Some(session.workspace_id().to_string()),
                worktree_id: Some(worktree_id.clone()),
                workspace_mount: Some(worktree_id.clone()),
                development: Some(
                    self.runtime_state
                        .slice_development_selection_for_session(session, &worktree_id)?,
                ),
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
                interactive: false,
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

        let target_room = match spawn.room.as_deref() {
            Some(room) => match self.runtime_state.resolve_session_ref_id(room, None).await {
                Ok(room) => Some(room),
                Err(error) => return Ok(meta_command_failure_result(command, error)),
            },
            None => None,
        };
        // A reference to this kernel creates a local child: only another kernel decides the folder.
        let kernel_ref = if spawn.slice_ref.is_some() {
            None
        } else {
            match self.runtime_state.settle_kernel_ref(spawn.kernel_ref).await {
                Ok(kernel_ref) => kernel_ref,
                Err(error) => return Ok(meta_command_failure_result(command, error)),
            }
        };
        let same_provider = spawns_same_provider(spawn.provider.as_deref(), metaagent);
        let follow = target_room.is_some() && !spawn.no_follow;
        let request = LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            no_subscribe: spawn.no_follow,
            session_id: target_room
                .clone()
                .unwrap_or_else(|| session.id().to_string()),
            alias: spawn.alias,
            provider: spawn
                .provider
                .or_else(|| Some(metaagent.provider().to_string())),
            // The account profile is the caller's choice, not an authority limit.
            account_profile: spawn.account_profile.or_else(|| {
                same_provider
                    .then(|| metaagent.account_profile().map(str::to_string))
                    .flatten()
            }),
            model: spawn.model.or_else(|| {
                same_provider
                    .then(|| metaagent.model().map(str::to_string))
                    .flatten()
            }),
            effort: spawn.effort.or_else(|| {
                same_provider
                    .then(|| metaagent.effort().map(str::to_string))
                    .flatten()
            }),
            // Omitted, the child takes the parent's effective limit where it is created.
            execution_mode: None,
            permission_level: spawn.permission_level,
            // The parent's folder is a path on this machine: a child on another kernel that
            // names no folder starts where that kernel starts its agents.
            worktree_id: spawn.worktree_id.or_else(|| {
                (target_room.is_none() && kernel_ref.is_none())
                    .then(|| metaagent.worktree_id().map(str::to_string))
                    .flatten()
            }),
            kernel_ref,
            slice_ref: spawn.slice_ref,
            worktree_placement: spawn.worktree_placement,
            metaagent: false,
        });
        let mut spawn_command = meta_kernel_command(provider_run, metaagent, &request);
        spawn_command.idempotency_key = spawn.command_id.clone();
        // A peer in another room of the owner is created by the owner as that room knows it.
        if let Some(room) = target_room.as_deref().filter(|room| *room != session.id()) {
            spawn_command.caller.user_id = Some(
                self.runtime_state
                    .owner_identity_in(room, metaagent.owner_user_id()),
            );
        }
        let response = match Box::pin(self.dispatch(spawn_command, request)).await {
            Ok(response) => response,
            Err(error) => return Ok(meta_command_failure_result(command, error)),
        };
        if let (Some(after), LocalDaemonResponse::AgentSpawned { agent, .. }) =
            (spawn.deadline, &response)
        {
            if let Err(error) = self
                .arm_child_deadline(provider_run, session, metaagent, agent, after)
                .await
            {
                return Ok(meta_command_failure_result(command, error));
            }
        }
        // The spawner of a peer hears of its turns without polling, unless it said --no-follow.
        // The peer exists either way, so a follow that fails is reported next to it.
        let followed = match (&response, follow) {
            (LocalDaemonResponse::AgentSpawned { agent, .. }, true) => Some(
                self.runtime_state
                    .follow_spawned_peer(metaagent, agent)
                    .map_err(|error| error.to_string()),
            ),
            _ => None,
        };
        let mut result = meta_command_success_result(
            command,
            &response,
            metaagent,
            self.runtime_state.room_agent_tools_enabled(),
        );
        if let LocalDaemonResponse::AgentSpawned { agent, .. } = &response {
            // Read after the deadline was armed above, so the answer states it.
            let facts = match self.runtime_state.spawned_agent_facts(agent).await {
                Ok(facts) => facts,
                Err(error) => return Ok(meta_command_failure_result(command, error)),
            };
            result.payload["response"]["agent"] = summarize_spawned_agent(agent, &facts);
        }
        if target_room.is_some() {
            if let Some(payload) = result.payload.as_object_mut() {
                let (registration, refused) = match followed {
                    Some(Ok(registration)) => (Some(registration), None),
                    Some(Err(refused)) => (None, Some(refused)),
                    None => (None, None),
                };
                payload.insert(
                    "follow_registration_id".to_string(),
                    serde_json::json!(registration),
                );
                if let Some(refused) = refused {
                    payload.insert("follow_refused".to_string(), serde_json::json!(refused));
                }
            }
        }
        if let Some(slice) = created_slice {
            if let Some(payload) = result.payload.as_object_mut() {
                payload.insert("created_slice".to_string(), serde_json::json!(slice));
            }
        }
        Ok(result)
    }

    /// `agent spawn --room <room> --kernel <kernel>`: an ordinary agent in a room on another
    /// kernel of the owner, with the same defaults and the same follow a peer on this kernel
    /// gets, unless the caller says --no-follow.
    async fn spawn_peer_on_kernel(
        &self,
        metaagent: &crate::agent::AgentInstance,
        command: &str,
        kernel: &str,
        room: String,
        spawn: MetaAgentSpawnArgs,
    ) -> RuntimeToolResult {
        let same_provider = spawns_same_provider(spawn.provider.as_deref(), metaagent);
        let inherited =
            |value: Option<&str>| same_provider.then(|| value.map(str::to_string)).flatten();
        let request = crate::runtime::state::peer_agent_outbound::RoomPeerSpawn {
            room,
            provider: spawn
                .provider
                .unwrap_or_else(|| metaagent.provider().to_string()),
            account_profile: spawn
                .account_profile
                .or_else(|| inherited(metaagent.account_profile())),
            model: spawn.model.or_else(|| inherited(metaagent.model())),
            effort: spawn.effort.or_else(|| inherited(metaagent.effort())),
            alias: spawn.alias,
            prompt: None,
            permission_level: spawn
                .permission_level
                .or(metaagent.permission_level_override()),
            command_id: spawn.command_id,
            follow: !spawn.no_follow,
        };
        match self
            .runtime_state
            .spawn_room_peer(metaagent, kernel, request)
            .await
        {
            Ok(spawned) => RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({
                    "command": command,
                    "response": {"spawned": {"kernel": spawned.agent.kernel_id, "room": spawned.agent.room_id, "agent": spawned.agent.agent_id, "alias": spawned.alias, "duplicate": spawned.duplicate}},
                    "follow_registration_id": spawned.follow_registration_id,
                }),
            },
            Err(error) => meta_command_failure_result(command, error),
        }
    }

    /// `trigger notification <source> [prompt]` and `trigger remove <id>`: an agent acts on its own
    /// triggers only, so the request never carries an agent argument.
    fn meta_trigger_request(
        &self,
        session: &crate::session::RuntimeSession,
        actor: &crate::agent::AgentInstance,
        args: &[String],
    ) -> Result<LocalDaemonRequest, DaemonError> {
        if args
            .iter()
            .any(|arg| arg == "--agent" || arg.starts_with("--agent="))
        {
            return Err(meta_command_error(
                "an agent subscribes only itself; the command takes no agent argument",
            ));
        }
        match args.first().map(String::as_str) {
            Some("notification") => {
                let (source, prompt) = self.runtime_state.choose_notification_source(
                    session.owner_user_id(),
                    actor.owner_user_id(),
                    session.id(),
                    &args[1..],
                )?;
                Ok(LocalDaemonRequest::CreateAgentTrigger(
                    crate::local::CreateAgentTriggerRequest {
                        session_id: session.id().to_string(),
                        agent_id: actor.id().to_string(),
                        source: crate::local::AgentTriggerSourceRequest::Notification { source },
                        busy_policy: None,
                        prompt,
                    },
                ))
            }
            Some("remove") => {
                let [_, trigger_id] = args else {
                    return Err(meta_command_error("usage: trigger remove <trigger-id>"));
                };
                if !session
                    .agent_triggers()
                    .iter()
                    .any(|t| t.id() == trigger_id && t.agent_id() == actor.id())
                {
                    return Err(meta_command_error("an agent removes only its own triggers"));
                }
                Ok(LocalDaemonRequest::DeleteAgentTrigger(
                    crate::local::DeleteAgentTriggerRequest {
                        session_id: session.id().to_string(),
                        trigger_id: trigger_id.clone(),
                    },
                ))
            }
            _ => Err(meta_command_error(
                "usage: trigger notification <source> [prompt] | trigger remove <trigger-id>",
            )),
        }
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
                meta_agent_request(
                    session,
                    metaagent,
                    &tokens[1..],
                    &agents,
                    self.runtime_state.room_agent_tools_enabled(),
                )
            }
            "workflow" => {
                let agents = self.runtime_state.session_agents(session.id());
                meta_workflow_request(
                    session,
                    metaagent,
                    &tokens[1..],
                    &agents,
                    self.runtime_state.room_agent_tools_enabled(),
                )
            }
            "slice" => meta_slice_request(&tokens[1..]),
            "environment" => meta_environment_request(session, &tokens[1..], |from| {
                self.runtime_state
                    .environment_source(from)
                    .map(|(kernel_id, _)| kernel_id)
            }),
            "trigger" => self.meta_trigger_request(session, metaagent, &tokens[1..]),
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
            "extension" | "extensions"
                if matches!(tokens.get(1).map(String::as_str), Some("grant" | "revoke"))
                    && tokens.get(2).map(String::as_str) == Some("app") =>
            {
                let agents = self.runtime_state.session_agents(session.id());
                let request = meta_app_binding_request(
                    session,
                    metaagent,
                    &tokens[1..],
                    &agents,
                    self.runtime_state.room_agent_tools_enabled(),
                )?;
                Ok(request)
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

    pub(super) async fn ensure_metaagent_command_attachment(
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

    pub(super) async fn resolve_owned_regular_agent(
        &self,
        session_id: &str,
        metaagent: &crate::agent::AgentInstance,
        reference: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agents = self.runtime_state.session_agents(session_id);
        if self.runtime_state.room_agent_tools_enabled() {
            return crate::runtime::room_tool_admission::resolve_agent(
                &agents,
                metaagent.session_id(),
                reference,
            )
            .cloned();
        }
        let owned_agents = agents
            .into_iter()
            .filter(|agent| {
                !agent.is_metaagent() && agent.parent_agent_id() == Some(metaagent.id())
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
