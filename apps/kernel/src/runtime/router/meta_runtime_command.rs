use crate::agent::GitWorktreePlacement;
use crate::attachment::ClientCapabilityLevel;
use crate::error::DaemonError;
use crate::local::{
    AddWorkflowEdgeRequest, AddWorkflowNodeRequest, AliasAgentRequest,
    AliasWorkflowEndpointRequest, AliasWorkflowRequest, AttachToSessionRequest,
    CancelWorkflowRunRequest, CreateSliceRequest, CreateWorkflowEndpointRequest,
    CreateWorkflowRequest, DestroyAgentRequest, ExtensionKind, FocusAgentRequest,
    GetCredentialRequest, GetCredentialVaultStatusRequest, GetMcpServerRequest, GetSkillRequest,
    GetWorkflowRunRequest, GrantAgentExtensionRequest, ImportMcpServersRequest,
    ImportProviderCapabilitiesRequest, ImportSkillsRequest, InstallMcpServerRequest,
    InstallSkillRequest, InvokeWorkflowEndpointRequest, ListAgentsRequest, ListCredentialsRequest,
    ListMcpServersRequest, ListSkillsRequest, ListWorkflowRunsRequest, ListWorkflowsRequest,
    LocalDaemonRequest, LocalDaemonResponse, ManageCredentialVaultRequest, RemoveCredentialRequest,
    RemoveWorkflowEdgeRequest, RemoveWorkflowNodeRequest, ResolveWorkflowRequest,
    ResumeWorkflowRunRequest, RevokeAgentExtensionRequest, SetWorkflowNodeCanCompleteRunRequest,
    SetWorkflowNodeCanEmitIntermediateOutputRequest, SetWorkflowNodeMaxTurnsRequest,
    SetWorkflowNodeWaitForAllInputsRequest, SliceRefRequest, SpawnAgentRequest,
    UninstallMcpServerRequest, UninstallSkillRequest, UpdateMcpServerRequest, UpdateSkillRequest,
    UpdateWorkflowNodeInstructionsRequest, UpsertCredentialRequest,
};
use crate::runtime::command::{KernelCaller, KernelCallerKind, KernelCommand, KernelCommandSource};
use crate::transport::runtime_tools::{MetaRunCommandArgs, RuntimeToolResult};

use super::CommandRouter;

impl CommandRouter {
    pub(super) async fn dispatch_meta_run_command(
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

    pub(super) async fn dispatch_forwarded_meta_run_command(
        &self,
        context: crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let (session, metaagent) = {
            let app = self.app.lock().await;
            let session = app.sessions().get_session(&context.home_session_id)?;
            let agent = app.agents().get_agent(&context.home_agent_id)?;
            (session, agent)
        };
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
        let durable_state = {
            let app = self.app.lock().await;
            app.durable_state_store()
        };
        if let Err(error) = durable_state.append_event(
            "metaagent.command.executed",
            Some(metaagent.id().to_string()),
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
                    "metaagent prompt does not support blocking reply flags (`{flag}`); use events and turn_overview"
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
                    ))
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
                    ))
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
            return Err(meta_command_error("empty metaagent command"));
        };
        match command {
            "agent" => {
                let agents = {
                    let app = self.app.lock().await;
                    app.agents().get_session_agents(session.id())
                };
                meta_agent_request(session, metaagent, &tokens[1..], &agents)
            }
            "workflow" => {
                let agents = {
                    let app = self.app.lock().await;
                    app.agents().get_session_agents(session.id())
                };
                meta_workflow_request(session, metaagent, &tokens[1..], &agents)
            }
            "mcp" => {
                let agents = {
                    let app = self.app.lock().await;
                    app.agents().get_session_agents(session.id())
                };
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
                let agents = {
                    let app = self.app.lock().await;
                    app.agents().get_session_agents(session.id())
                };
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
                "`{other}` is registered but not implemented by the metaagent command router"
            ))),
        }
    }

    async fn ensure_metaagent_command_attachment(
        &self,
        session_id: &str,
        metaagent: &crate::agent::AgentInstance,
    ) -> Result<String, DaemonError> {
        let client_id = metaagent_command_client_id(metaagent.id());
        {
            let app = self.app.lock().await;
            if let Some(attachment) = app
                .attachments()
                .list_client_attachments(&client_id)
                .into_iter()
                .find(|attachment| attachment.session_id() == session_id)
            {
                return Ok(attachment.id().to_string());
            }
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
        let agents = {
            let app = self.app.lock().await;
            app.agents().get_session_agents(session_id)
        };
        agents
            .into_iter()
            .find(|agent| {
                !agent.is_metaagent()
                    && agent.controlled_by_metaagent_id() == Some(metaagent.id())
                    && (agent.id() == reference
                        || agent.agent_ref() == reference
                        || agent.alias() == Some(reference))
            })
            .ok_or_else(|| {
                meta_command_error(format!(
                    "agent `{reference}` is not an owned regular agent in this session"
                ))
            })
    }
}

#[derive(Debug, Clone)]
struct MetaAgentSpawnArgs {
    alias: Option<String>,
    provider: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    worktree_id: Option<String>,
    kernel_ref: Option<String>,
    slice_ref: Option<String>,
    worktree_placement: Option<GitWorktreePlacement>,
    slice_create: Option<MetaAgentSliceCreate>,
}

#[derive(Debug, Clone)]
struct MetaAgentSliceCreate {
    display_mode: crate::slice::SliceDisplayMode,
}

fn parse_meta_agent_spawn_args(
    args: &[String],
    session: &crate::session::RuntimeSession,
) -> Result<MetaAgentSpawnArgs, DaemonError> {
    let mut positional = Vec::new();
    let mut provider = None;
    let mut explicit_model = None;
    let mut effort = None;
    let mut directory = None;
    let mut git_worktree = None;
    let mut branch = None;
    let mut from_ref = None;
    let mut kernel_ref = None;
    let mut slice_ref = None;
    let mut slice_create = None;
    let mut slice_display_mode = None;

    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        match arg.as_str() {
            "--meta" | "--metaagent" | "--as-metaagent" => {
                return Err(meta_command_error(
                    "metaagents cannot spawn another metaagent through run_command",
                ));
            }
            "--provider" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(meta_command_error(
                        "usage: agent spawn --provider <provider>",
                    ));
                };
                if value.starts_with("--") {
                    return Err(meta_command_error(
                        "usage: agent spawn --provider <provider>",
                    ));
                }
                provider = Some(value.clone());
                index += 2;
            }
            "--model" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(meta_command_error("usage: agent spawn --model <model>"));
                };
                if value.starts_with("--") {
                    return Err(meta_command_error("usage: agent spawn --model <model>"));
                }
                explicit_model = Some(value.clone());
                index += 2;
            }
            "--effort" | "--variant" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(meta_command_error(
                        "usage: agent spawn --effort <effort>|--variant <variant>",
                    ));
                };
                if value.starts_with("--") {
                    return Err(meta_command_error(
                        "usage: agent spawn --effort <effort>|--variant <variant>",
                    ));
                }
                effort = Some(value.clone());
                index += 2;
            }
            "--dir" | "--directory" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(meta_command_error(format!(
                        "usage: {}",
                        crate::runtime::metaagent_command_registry::AGENT_SPAWN_USAGE
                    )));
                };
                if value.starts_with("--") {
                    return Err(meta_command_error("usage: agent spawn --dir <directory>"));
                }
                directory = Some(value.clone());
                index += 2;
            }
            "--worktree" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(meta_command_error(
                        "usage: agent spawn --worktree <directory> [--branch <branch>] [--from <ref>]",
                    ));
                };
                if value.starts_with("--") {
                    return Err(meta_command_error(
                        "usage: agent spawn --worktree <directory> [--branch <branch>] [--from <ref>]",
                    ));
                }
                git_worktree = Some(value.clone());
                index += 2;
            }
            "--branch" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(meta_command_error("usage: agent spawn --branch <branch>"));
                };
                if value.starts_with("--") {
                    return Err(meta_command_error("usage: agent spawn --branch <branch>"));
                }
                branch = Some(value.clone());
                index += 2;
            }
            "--from" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(meta_command_error("usage: agent spawn --from <ref>"));
                };
                if value.starts_with("--") {
                    return Err(meta_command_error("usage: agent spawn --from <ref>"));
                }
                from_ref = Some(value.clone());
                index += 2;
            }
            "--machine" | "--kernel" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(meta_command_error(
                        "usage: agent spawn --machine <machine-ref>|--kernel <kernel-ref>",
                    ));
                };
                if value.starts_with("--") {
                    return Err(meta_command_error(
                        "usage: agent spawn --machine <machine-ref>|--kernel <kernel-ref>",
                    ));
                }
                if kernel_ref.is_some() {
                    return Err(meta_command_error(
                        "usage: agent spawn uses either --machine or --kernel, not both",
                    ));
                }
                kernel_ref = Some(value.clone());
                index += 2;
            }
            "--slice" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(meta_command_error(
                        "usage: agent spawn --slice off|new|new:headless|new:headed|<slice-ref>",
                    ));
                };
                if value.starts_with("--") {
                    return Err(meta_command_error(
                        "usage: agent spawn --slice off|new|new:headless|new:headed|<slice-ref>",
                    ));
                }
                match value.as_str() {
                    "off" => {
                        slice_ref = None;
                        slice_create = None;
                    }
                    "new" | "new:headless" => {
                        slice_ref = None;
                        slice_create = Some(MetaAgentSliceCreate {
                            display_mode: crate::slice::SliceDisplayMode::Headless,
                        });
                    }
                    "new:headed" => {
                        slice_ref = None;
                        slice_create = Some(MetaAgentSliceCreate {
                            display_mode: crate::slice::SliceDisplayMode::Headed,
                        });
                    }
                    _ => {
                        slice_ref = Some(value.clone());
                        slice_create = None;
                    }
                }
                index += 2;
            }
            "--slice-display" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(meta_command_error(
                        "usage: agent spawn --slice-display headless|headed",
                    ));
                };
                let mode = match value.as_str() {
                    "headless" => crate::slice::SliceDisplayMode::Headless,
                    "headed" => crate::slice::SliceDisplayMode::Headed,
                    _ => {
                        return Err(meta_command_error(
                            "usage: agent spawn --slice-display headless|headed",
                        ));
                    }
                };
                slice_display_mode = Some(mode);
                index += 2;
            }
            value if value.starts_with("--") => {
                return Err(meta_command_error(format!(
                    "unknown agent spawn option `{value}`; usage: {}",
                    crate::runtime::metaagent_command_registry::AGENT_SPAWN_USAGE
                )));
            }
            _ => {
                positional.push(arg.clone());
                index += 1;
            }
        }
    }

    if positional.len() > 2 {
        return Err(meta_command_error(format!(
            "usage: {}",
            crate::runtime::metaagent_command_registry::AGENT_SPAWN_USAGE
        )));
    }
    if positional.get(1).is_some() && explicit_model.is_some() {
        return Err(meta_command_error(
            "usage: agent spawn accepts either positional [model] or --model <model>, not both",
        ));
    }
    if directory.is_some() && git_worktree.is_some() {
        return Err(meta_command_error(
            "usage: agent spawn uses either --dir or --worktree/--branch, not both",
        ));
    }
    if (branch.is_some() || from_ref.is_some()) && git_worktree.is_none() {
        return Err(meta_command_error(
            "usage: agent spawn --branch/--from require --worktree",
        ));
    }
    if let (Some(slice), None) = (slice_ref.as_deref(), slice_create.as_ref()) {
        if kernel_ref.is_some() {
            return Err(meta_command_error(
                "usage: agent spawn uses either --kernel/--machine or a reusable --slice, not both",
            ));
        }
        if directory.is_some() || git_worktree.is_some() {
            return Err(meta_command_error(
                "usage: agent spawn --slice <slice-ref> does not accept --dir or --worktree",
            ));
        }
        if slice.is_empty() {
            return Err(meta_command_error(
                "usage: agent spawn --slice off|new|new:headless|new:headed|<slice-ref>",
            ));
        }
    }
    if let Some(mode) = slice_display_mode {
        let Some(create) = slice_create.as_mut() else {
            return Err(meta_command_error(
                "usage: agent spawn --slice-display requires --slice new",
            ));
        };
        create.display_mode = mode;
    }
    let worktree_id = directory.map(|directory| resolve_metaagent_directory(session, &directory));
    let worktree_placement = if let Some(target_directory) = git_worktree {
        Some(GitWorktreePlacement {
            target_directory: Some(target_directory),
            branch,
            from_ref,
        })
    } else {
        None
    };

    Ok(MetaAgentSpawnArgs {
        alias: positional.first().cloned(),
        provider,
        model: explicit_model.or_else(|| positional.get(1).cloned()),
        effort,
        worktree_id,
        kernel_ref,
        slice_ref,
        worktree_placement,
        slice_create,
    })
}

fn resolve_metaagent_directory(
    session: &crate::session::RuntimeSession,
    directory: &str,
) -> String {
    let path = std::path::Path::new(directory);
    if path.is_absolute() {
        directory.to_string()
    } else {
        std::path::Path::new(session.worktree_id())
            .join(path)
            .to_string_lossy()
            .to_string()
    }
}

fn metaagent_spawn_slice_name(alias: Option<&str>) -> String {
    let base = alias.unwrap_or("metaagent-worker");
    let sanitized = base
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    let base = if sanitized.is_empty() {
        "metaagent-worker".to_string()
    } else {
        sanitized
    };
    let suffix = crate::session::unix_epoch_ms().to_string();
    format!("{base}-slice-{}", &suffix[suffix.len().saturating_sub(5)..])
}

fn meta_agent_request(
    session: &crate::session::RuntimeSession,
    metaagent: &crate::agent::AgentInstance,
    args: &[String],
    agents: &[crate::agent::AgentInstance],
) -> Result<LocalDaemonRequest, DaemonError> {
    match args.first().map(String::as_str) {
        Some("list" | "ls") => Ok(LocalDaemonRequest::ListAgents(ListAgentsRequest {
            session_id: session.id().to_string(),
        })),
        Some("spawn") => {
            let spawn = parse_meta_agent_spawn_args(&args[1..], session)?;
            if spawn.slice_create.is_some() {
                return Err(meta_command_error(
                    "agent spawn --slice new requires composed metaagent spawn dispatch",
                ));
            }
            Ok(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                session_id: session.id().to_string(),
                alias: spawn.alias,
                provider: spawn
                    .provider
                    .or_else(|| Some(metaagent.provider().to_string())),
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
                kernel_ref: spawn.kernel_ref,
                slice_ref: spawn.slice_ref,
                worktree_placement: spawn.worktree_placement,
                metaagent: false,
            }))
        }
        Some("focus") => {
            let Some(reference) = args.get(1) else {
                return Err(meta_command_error("usage: agent focus <owned-agent-ref>"));
            };
            if args.len() > 2 {
                return Err(meta_command_error("usage: agent focus <owned-agent-ref>"));
            }
            let agent = meta_owned_regular_agent_from_session(agents, metaagent, reference)?;
            Ok(LocalDaemonRequest::FocusAgent(FocusAgentRequest {
                session_id: session.id().to_string(),
                agent_id: agent.id().to_string(),
            }))
        }
        Some("alias" | "name") => {
            if args.len() < 3 {
                return Err(meta_command_error(
                    "usage: agent alias <owned-agent-ref> <alias|clear>",
                ));
            }
            let agent = meta_owned_regular_agent_from_session(agents, metaagent, &args[1])?;
            let alias = args[2..].join(" ");
            let alias = if matches!(alias.as_str(), "clear" | "none" | "-") {
                String::new()
            } else {
                alias
            };
            Ok(LocalDaemonRequest::AliasAgent(AliasAgentRequest {
                session_id: session.id().to_string(),
                agent_id: agent.id().to_string(),
                alias,
            }))
        }
        Some("delete" | "destroy" | "remove") => {
            let Some(reference) = args.get(1) else {
                return Err(meta_command_error("usage: agent delete <owned-agent-ref>"));
            };
            if args.len() > 2 {
                return Err(meta_command_error("usage: agent delete <owned-agent-ref>"));
            }
            let agent = meta_owned_regular_agent_from_session(agents, metaagent, reference)?;
            Ok(LocalDaemonRequest::DestroyAgent(DestroyAgentRequest {
                session_id: session.id().to_string(),
                agent_id: agent.id().to_string(),
            }))
        }
        _ => Err(meta_command_error(
            "usage: agent <list|spawn|focus|alias|delete|destroy> ...",
        )),
    }
}

fn meta_owned_regular_agent_from_session(
    agents: &[crate::agent::AgentInstance],
    metaagent: &crate::agent::AgentInstance,
    reference: &str,
) -> Result<crate::agent::AgentInstance, DaemonError> {
    agents
        .iter()
        .find(|agent| {
            !agent.is_metaagent()
                && agent.controlled_by_metaagent_id() == Some(metaagent.id())
                && (agent.id() == reference
                    || agent.agent_ref() == reference
                    || agent.alias() == Some(reference))
        })
        .cloned()
        .ok_or_else(|| {
            meta_command_error(format!(
                "agent `{reference}` is not an owned regular agent in this session"
            ))
        })
}

fn meta_workflow_request(
    session: &crate::session::RuntimeSession,
    metaagent: &crate::agent::AgentInstance,
    args: &[String],
    agents: &[crate::agent::AgentInstance],
) -> Result<LocalDaemonRequest, DaemonError> {
    match args.first().map(String::as_str) {
        Some("list" | "ls") | None => Ok(LocalDaemonRequest::ListWorkflows(ListWorkflowsRequest {
            session_id: session.id().to_string(),
        })),
        Some("new" | "create") => {
            if args.len() > 2 {
                return Err(meta_command_error("usage: workflow new [alias]"));
            }
            if args.get(1).is_some_and(|arg| arg.starts_with('-')) {
                return Err(meta_command_error("usage: workflow new [alias]"));
            }
            Ok(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
                session_id: session.id().to_string(),
                alias: args.get(1).cloned(),
            }))
        }
        Some("resolve" | "show" | "get") => {
            let Some(workflow_ref) = args.get(1) else {
                return Err(meta_command_error("usage: workflow resolve <workflow-ref>"));
            };
            if args.len() > 2 {
                return Err(meta_command_error("usage: workflow resolve <workflow-ref>"));
            }
            Ok(LocalDaemonRequest::ResolveWorkflow(ResolveWorkflowRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow_ref.clone(),
            }))
        }
        Some("alias" | "name") => {
            if args.len() < 3 {
                return Err(meta_command_error(
                    "usage: workflow alias <workflow-ref> <alias>",
                ));
            }
            Ok(LocalDaemonRequest::AliasWorkflow(AliasWorkflowRequest {
                session_id: session.id().to_string(),
                workflow_ref: args[1].clone(),
                alias: args[2..].join(" "),
                expected_workflow_revision: None,
            }))
        }
        Some("node") => match args.get(1).map(String::as_str) {
            Some("add") => {
                if args.len() != 4 {
                    return Err(meta_command_error(
                        "usage: workflow node add <workflow-ref> <owned-agent-ref>",
                    ));
                }
                let agent = meta_owned_regular_agent_from_session(agents, metaagent, &args[3])?;
                Ok(LocalDaemonRequest::AddWorkflowNode(
                    AddWorkflowNodeRequest {
                        session_id: session.id().to_string(),
                        workflow_ref: args[2].clone(),
                        agent_id: agent.id().to_string(),
                        expected_workflow_revision: None,
                    },
                ))
            }
            Some("remove" | "delete") => {
                if args.len() != 4 {
                    return Err(meta_command_error(
                        "usage: workflow node remove <workflow-ref> <node-id>",
                    ));
                }
                Ok(LocalDaemonRequest::RemoveWorkflowNode(
                    RemoveWorkflowNodeRequest {
                        session_id: session.id().to_string(),
                        workflow_ref: args[2].clone(),
                        node_id: args[3].clone(),
                        expected_workflow_revision: None,
                    },
                ))
            }
            Some("instructions" | "instruct") => {
                if args.len() < 4 {
                    return Err(meta_command_error(
                        "usage: workflow node instructions <workflow-ref> <node-id> [instructions]",
                    ));
                }
                let instructions = (!args[4..].is_empty())
                    .then(|| args[4..].join(" "))
                    .filter(|value| !matches!(value.as_str(), "clear" | "none" | "-"));
                Ok(LocalDaemonRequest::UpdateWorkflowNodeInstructions(
                    UpdateWorkflowNodeInstructionsRequest {
                        session_id: session.id().to_string(),
                        workflow_ref: args[2].clone(),
                        node_id: args[3].clone(),
                        instructions,
                        expected_workflow_revision: None,
                    },
                ))
            }
            Some("can-complete" | "complete") => {
                if args.len() != 5 {
                    return Err(meta_command_error(
                        "usage: workflow node can-complete <workflow-ref> <node-id> <true|false>",
                    ));
                }
                Ok(LocalDaemonRequest::SetWorkflowNodeCanCompleteRun(
                    SetWorkflowNodeCanCompleteRunRequest {
                        session_id: session.id().to_string(),
                        workflow_ref: args[2].clone(),
                        node_id: args[3].clone(),
                        can_complete_workflow_run: parse_meta_bool(&args[4])?,
                        expected_workflow_revision: None,
                    },
                ))
            }
            Some("intermediate-output" | "intermediate") => {
                if args.len() != 5 {
                    return Err(meta_command_error(
                        "usage: workflow node intermediate-output <workflow-ref> <node-id> <true|false>",
                    ));
                }
                Ok(LocalDaemonRequest::SetWorkflowNodeCanEmitIntermediateOutput(
                    SetWorkflowNodeCanEmitIntermediateOutputRequest {
                        session_id: session.id().to_string(),
                        workflow_ref: args[2].clone(),
                        node_id: args[3].clone(),
                        can_emit_intermediate_workflow_run_output: parse_meta_bool(&args[4])?,
                        expected_workflow_revision: None,
                    },
                ))
            }
            Some("wait-for-all-inputs" | "wait-all" | "join") => {
                if args.len() != 5 {
                    return Err(meta_command_error(
                        "usage: workflow node wait-for-all-inputs <workflow-ref> <node-id> <true|false>",
                    ));
                }
                Ok(LocalDaemonRequest::SetWorkflowNodeWaitForAllInputs(
                    SetWorkflowNodeWaitForAllInputsRequest {
                        session_id: session.id().to_string(),
                        workflow_ref: args[2].clone(),
                        node_id: args[3].clone(),
                        wait_for_all_inputs: parse_meta_bool(&args[4])?,
                        expected_workflow_revision: None,
                    },
                ))
            }
            Some("max-turns") => {
                if args.len() != 5 {
                    return Err(meta_command_error(
                        "usage: workflow node max-turns <workflow-ref> <node-id> <number|none>",
                    ));
                }
                let max_turns = match args[4].as_str() {
                    "none" | "clear" | "-" => None,
                    value => Some(value.parse::<u32>().map_err(|error| {
                        meta_command_error(format!("invalid max turns `{value}`: {error}"))
                    })?),
                };
                Ok(LocalDaemonRequest::SetWorkflowNodeMaxTurns(
                    SetWorkflowNodeMaxTurnsRequest {
                        session_id: session.id().to_string(),
                        workflow_ref: args[2].clone(),
                        node_id: args[3].clone(),
                        max_turns,
                        expected_workflow_revision: None,
                    },
                ))
            }
            _ => Err(meta_command_error(
                "usage: workflow node <add|remove|instructions|can-complete|intermediate-output|wait-for-all-inputs|max-turns> ...",
            )),
        },
        Some("endpoint") => match args.get(1).map(String::as_str) {
            Some("new" | "create") => {
                if args.len() < 4 || args.len() > 5 {
                    return Err(meta_command_error(
                        "usage: workflow endpoint new <workflow-ref> <entry-node-id> [alias]",
                    ));
                }
                Ok(LocalDaemonRequest::CreateWorkflowEndpoint(
                    CreateWorkflowEndpointRequest {
                        session_id: session.id().to_string(),
                        workflow_ref: args[2].clone(),
                        entry_node_id: args[3].clone(),
                        alias: args.get(4).cloned(),
                        expected_workflow_revision: None,
                    },
                ))
            }
            Some("alias" | "name") => {
                if args.len() < 5 {
                    return Err(meta_command_error(
                        "usage: workflow endpoint alias <workflow-ref> <endpoint-ref> <alias>",
                    ));
                }
                Ok(LocalDaemonRequest::AliasWorkflowEndpoint(
                    AliasWorkflowEndpointRequest {
                        session_id: session.id().to_string(),
                        workflow_ref: args[2].clone(),
                        endpoint_ref: args[3].clone(),
                        alias: args[4..].join(" "),
                        expected_workflow_revision: None,
                    },
                ))
            }
            _ => Err(meta_command_error(
                "usage: workflow endpoint <new|alias> ...",
            )),
        },
        Some("edge") => match args.get(1).map(String::as_str) {
            Some("add") => {
                if args.len() != 5 {
                    return Err(meta_command_error(
                        "usage: workflow edge add <workflow-ref> <from-node-id> <to-node-id>",
                    ));
                }
                Ok(LocalDaemonRequest::AddWorkflowEdge(AddWorkflowEdgeRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: args[2].clone(),
                    from_node_id: args[3].clone(),
                    to_node_id: args[4].clone(),
                    handoff_schema_ref: None,
                    validation_policy: None,
                    source_side: None,
                    target_side: None,
                    expected_workflow_revision: None,
                }))
            }
            Some("remove" | "delete") => {
                if args.len() != 4 {
                    return Err(meta_command_error(
                        "usage: workflow edge remove <workflow-ref> <edge-id>",
                    ));
                }
                Ok(LocalDaemonRequest::RemoveWorkflowEdge(
                    RemoveWorkflowEdgeRequest {
                        session_id: session.id().to_string(),
                        workflow_ref: args[2].clone(),
                        edge_id: args[3].clone(),
                        expected_workflow_revision: None,
                    },
                ))
            }
            _ => Err(meta_command_error(
                "usage: workflow edge <add|remove> ...",
            )),
        },
        Some("run") if args.get(1).map(String::as_str) == Some("get") => {
            let Some(workflow_run_ref) = args.get(2) else {
                return Err(meta_command_error("usage: workflow run get <run-ref>"));
            };
            if args.len() > 3 {
                return Err(meta_command_error("usage: workflow run get <run-ref>"));
            }
            Ok(LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: workflow_run_ref.clone(),
            }))
        }
        Some("run" | "start") => {
            if args.len() < 3 {
                return Err(meta_command_error(
                    "usage: workflow run <workflow-ref> <endpoint-ref> [prompt]",
                ));
            }
            Ok(LocalDaemonRequest::InvokeWorkflowEndpoint(
                InvokeWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: args[1].clone(),
                    endpoint_ref: args[2].clone(),
                    queue_ref: None,
                    prompt: (!args[3..].is_empty()).then(|| args[3..].join(" ")),
                    publication_invocation: None,
                },
            ))
        }
        Some("runs") => Ok(LocalDaemonRequest::ListWorkflowRuns(
            ListWorkflowRunsRequest {
                session_id: session.id().to_string(),
                workflow_ref: args.get(1).cloned(),
            },
        )),
        Some("get-run" | "run-status") => {
            let Some(workflow_run_ref) = args.get(1) else {
                return Err(meta_command_error("usage: workflow get-run <run-ref>"));
            };
            if args.len() > 2 {
                return Err(meta_command_error("usage: workflow get-run <run-ref>"));
            }
            Ok(LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: workflow_run_ref.clone(),
            }))
        }
        Some("cancel") => {
            let Some(workflow_run_ref) = args.get(1) else {
                return Err(meta_command_error("usage: workflow cancel <run-ref>"));
            };
            if args.len() > 2 {
                return Err(meta_command_error("usage: workflow cancel <run-ref>"));
            }
            Ok(LocalDaemonRequest::CancelWorkflowRun(
                CancelWorkflowRunRequest {
                    session_id: session.id().to_string(),
                    workflow_run_ref: workflow_run_ref.clone(),
                },
            ))
        }
        Some("resume") => {
            let Some(workflow_run_ref) = args.get(1) else {
                return Err(meta_command_error("usage: workflow resume <run-ref>"));
            };
            if args.len() > 2 {
                return Err(meta_command_error("usage: workflow resume <run-ref>"));
            }
            Ok(LocalDaemonRequest::ResumeWorkflowRun(
                ResumeWorkflowRunRequest {
                    session_id: session.id().to_string(),
                    workflow_run_ref: workflow_run_ref.clone(),
                },
            ))
        }
        _ => Err(meta_command_error(
            "usage: workflow <list|new|resolve|alias|node|endpoint|edge|run|runs|get-run|cancel|resume> ...",
        )),
    }
}

fn meta_extension_import_request(
    session: &crate::session::RuntimeSession,
    args: &[String],
) -> Result<LocalDaemonRequest, DaemonError> {
    if args.first().map(String::as_str) != Some("import")
        || args.get(1).map(String::as_str) != Some("providers")
    {
        return Err(meta_command_error(
            "usage: extension import providers [--provider codex|opencode|claude] [--kind all|mcp|skill] [--name <capability>] [--dry-run]",
        ));
    }
    let mut providers = Vec::new();
    let mut kind = None;
    let mut name = None;
    let mut dry_run = false;
    let mut index = 2;
    while index < args.len() {
        match args[index].as_str() {
            "--provider" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(meta_command_error(
                        "usage: extension import providers --provider <provider>",
                    ));
                };
                if value.starts_with("--") {
                    return Err(meta_command_error(
                        "usage: extension import providers --provider <provider>",
                    ));
                }
                providers.push(value.clone());
                index += 2;
            }
            "--kind" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(meta_command_error(
                        "usage: extension import providers --kind all|mcp|skill",
                    ));
                };
                if value.starts_with("--") {
                    return Err(meta_command_error(
                        "usage: extension import providers --kind all|mcp|skill",
                    ));
                }
                kind = Some(value.clone());
                index += 2;
            }
            "--name" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(meta_command_error(
                        "usage: extension import providers --name <capability>",
                    ));
                };
                if value.starts_with("--") {
                    return Err(meta_command_error(
                        "usage: extension import providers --name <capability>",
                    ));
                }
                name = Some(value.clone());
                index += 2;
            }
            "--dry-run" => {
                dry_run = true;
                index += 1;
            }
            other => {
                return Err(meta_command_error(format!(
                    "unsupported extension import option `{other}`"
                )));
            }
        }
    }
    Ok(LocalDaemonRequest::ImportProviderCapabilities(
        ImportProviderCapabilitiesRequest {
            workspace_id: Some(session.workspace_id().to_string()),
            providers,
            kind,
            name,
            dry_run,
        },
    ))
}

fn meta_extension_request(
    session: &crate::session::RuntimeSession,
    metaagent: &crate::agent::AgentInstance,
    kind: ExtensionKind,
    family: &str,
    args: &[String],
    agents: &[crate::agent::AgentInstance],
) -> Result<LocalDaemonRequest, DaemonError> {
    match args.first().map(String::as_str) {
        Some("list" | "ls") => match kind {
            ExtensionKind::Mcp => Ok(LocalDaemonRequest::ListMcpServers(ListMcpServersRequest {
                workspace_id: Some(session.workspace_id().to_string()),
            })),
            ExtensionKind::Skill => Ok(LocalDaemonRequest::ListSkills(ListSkillsRequest {
                workspace_id: Some(session.workspace_id().to_string()),
            })),
            _ => Err(meta_command_error(format!(
                "`{family} list` is not supported by metaagent run_command"
            ))),
        },
        Some("show" | "get") => {
            let Some(name) = args.get(1) else {
                return Err(meta_command_error(format!("usage: {family} show <name>")));
            };
            if args.len() > 2 {
                return Err(meta_command_error(format!("usage: {family} show <name>")));
            }
            match kind {
                ExtensionKind::Mcp => Ok(LocalDaemonRequest::GetMcpServer(GetMcpServerRequest {
                    workspace_id: Some(session.workspace_id().to_string()),
                    name: name.clone(),
                })),
                ExtensionKind::Skill => Ok(LocalDaemonRequest::GetSkill(GetSkillRequest {
                    workspace_id: Some(session.workspace_id().to_string()),
                    name: name.clone(),
                })),
                _ => Err(meta_command_error(format!(
                    "`{family} show` is not supported by metaagent run_command"
                ))),
            }
        }
        Some("install-json" | "update-json") if kind == ExtensionKind::Mcp => {
            let Some(json) = args.get(1) else {
                return Err(meta_command_error(format!(
                    "usage: {family} install-json <mcp-json>"
                )));
            };
            if args.len() > 2 {
                return Err(meta_command_error(format!(
                    "usage: {family} install-json <mcp-json>"
                )));
            }
            let config = serde_json::from_str::<crate::mcp::ArrobaMcpServerConfig>(json)
                .map_err(|error| meta_command_error(format!("invalid MCP JSON config: {error}")))?;
            if args.first().map(String::as_str) == Some("install-json") {
                Ok(LocalDaemonRequest::InstallMcpServer(
                    InstallMcpServerRequest {
                        workspace_id: Some(session.workspace_id().to_string()),
                        config,
                    },
                ))
            } else {
                Ok(LocalDaemonRequest::UpdateMcpServer(
                    UpdateMcpServerRequest {
                        workspace_id: Some(session.workspace_id().to_string()),
                        config,
                    },
                ))
            }
        }
        Some("install" | "update") if kind == ExtensionKind::Skill => {
            let Some(source_path) = args.get(1) else {
                return Err(meta_command_error(format!(
                    "usage: {family} install <path>"
                )));
            };
            if args.len() > 2 {
                return Err(meta_command_error(format!(
                    "usage: {family} install <path>"
                )));
            }
            let source_path = std::path::PathBuf::from(source_path);
            if args.first().map(String::as_str) == Some("install") {
                Ok(LocalDaemonRequest::InstallSkill(InstallSkillRequest {
                    workspace_id: Some(session.workspace_id().to_string()),
                    source_path,
                }))
            } else {
                Ok(LocalDaemonRequest::UpdateSkill(UpdateSkillRequest {
                    workspace_id: Some(session.workspace_id().to_string()),
                    source_path,
                }))
            }
        }
        Some("uninstall" | "remove") => {
            let Some(name) = args.get(1) else {
                return Err(meta_command_error(format!(
                    "usage: {family} uninstall <name>"
                )));
            };
            if args.len() > 2 {
                return Err(meta_command_error(format!(
                    "usage: {family} uninstall <name>"
                )));
            }
            match kind {
                ExtensionKind::Mcp => Ok(LocalDaemonRequest::UninstallMcpServer(
                    UninstallMcpServerRequest {
                        workspace_id: Some(session.workspace_id().to_string()),
                        name: name.clone(),
                    },
                )),
                ExtensionKind::Skill => {
                    Ok(LocalDaemonRequest::UninstallSkill(UninstallSkillRequest {
                        workspace_id: Some(session.workspace_id().to_string()),
                        name: name.clone(),
                    }))
                }
                _ => Err(meta_command_error(format!(
                    "`{family} uninstall` is not supported by metaagent run_command"
                ))),
            }
        }
        Some("import") => {
            let Some(provider) = args.get(1) else {
                return Err(meta_command_error(format!(
                    "usage: {family} import <provider> [name]"
                )));
            };
            if args.len() > 3 {
                return Err(meta_command_error(format!(
                    "usage: {family} import <provider> [name]"
                )));
            }
            match kind {
                ExtensionKind::Mcp => Ok(LocalDaemonRequest::ImportMcpServers(
                    ImportMcpServersRequest {
                        workspace_id: Some(session.workspace_id().to_string()),
                        provider: provider.clone(),
                        name: args.get(2).cloned(),
                    },
                )),
                ExtensionKind::Skill => Ok(LocalDaemonRequest::ImportSkills(ImportSkillsRequest {
                    workspace_id: Some(session.workspace_id().to_string()),
                    provider: provider.clone(),
                    name: args.get(2).cloned(),
                })),
                _ => Err(meta_command_error(format!(
                    "`{family} import` is not supported by metaagent run_command"
                ))),
            }
        }
        Some("grant") => {
            if args.len() != 3 {
                return Err(meta_command_error(format!(
                    "usage: {family} grant <owned-agent-ref> <name>"
                )));
            }
            let agent = meta_owned_regular_agent_from_session(agents, metaagent, &args[1])?;
            Ok(LocalDaemonRequest::GrantAgentExtension(
                GrantAgentExtensionRequest {
                    workspace_id: Some(session.workspace_id().to_string()),
                    agent_ref: agent.agent_ref().to_string(),
                    kind,
                    name: args[2].clone(),
                    environment: None,
                    credential: None,
                    max_safety: None,
                },
            ))
        }
        Some("revoke") => {
            if args.len() != 3 {
                return Err(meta_command_error(format!(
                    "usage: {family} revoke <owned-agent-ref> <name>"
                )));
            }
            let agent = meta_owned_regular_agent_from_session(agents, metaagent, &args[1])?;
            Ok(LocalDaemonRequest::RevokeAgentExtension(
                RevokeAgentExtensionRequest {
                    agent_ref: agent.agent_ref().to_string(),
                    kind,
                    name: args[2].clone(),
                },
            ))
        }
        _ => Err(meta_command_error(format!(
            "usage: {family} <list|show|install|update|uninstall|import|grant|revoke> ..."
        ))),
    }
}

fn parse_meta_bool(value: &str) -> Result<bool, DaemonError> {
    match value {
        "true" | "yes" | "on" | "1" => Ok(true),
        "false" | "no" | "off" | "0" => Ok(false),
        _ => Err(meta_command_error(format!(
            "expected true or false, got `{value}`"
        ))),
    }
}

fn meta_credential_request(
    session: &crate::session::RuntimeSession,
    metaagent: &crate::agent::AgentInstance,
    args: &[String],
) -> Result<LocalDaemonRequest, DaemonError> {
    match args.first().map(String::as_str) {
        Some("list" | "ls") | None => {
            Ok(LocalDaemonRequest::ListCredentials(ListCredentialsRequest))
        }
        Some("get" | "show") => {
            let Some(id) = args.get(1) else {
                return Err(meta_command_error("usage: credential get <id>"));
            };
            if args.len() > 2 {
                return Err(meta_command_error("usage: credential get <id>"));
            }
            Ok(LocalDaemonRequest::GetCredential(GetCredentialRequest {
                id: id.clone(),
            }))
        }
        Some("upsert-json") => {
            let Some(json) = args.get(1) else {
                return Err(meta_command_error(
                    "usage: credential upsert-json <credential-json>",
                ));
            };
            if args.len() > 2 {
                return Err(meta_command_error(
                    "usage: credential upsert-json <credential-json>",
                ));
            }
            let credential = serde_json::from_str::<crate::config::UserCredentialConfig>(json)
                .map_err(|error| {
                    meta_command_error(format!("invalid credential JSON config: {error}"))
                })?;
            Ok(LocalDaemonRequest::UpsertCredential(
                UpsertCredentialRequest { credential },
            ))
        }
        Some("remove") => {
            let Some(id) = args.get(1) else {
                return Err(meta_command_error("usage: credential remove <id>"));
            };
            if args.len() > 2 {
                return Err(meta_command_error("usage: credential remove <id>"));
            }
            Ok(LocalDaemonRequest::RemoveCredential(
                RemoveCredentialRequest { id: id.clone() },
            ))
        }
        Some("vault") => match args.get(1).map(String::as_str) {
            Some("status") => Ok(LocalDaemonRequest::GetCredentialVaultStatus(
                GetCredentialVaultStatusRequest,
            )),
            Some("manage") => Ok(LocalDaemonRequest::ManageCredentialVault(
                ManageCredentialVaultRequest {
                    session_id: session.id().to_string(),
                    agent_id: Some(metaagent.id().to_string()),
                },
            )),
            _ => Err(meta_command_error(
                "usage: credential vault <status|manage>",
            )),
        },
        _ => Err(meta_command_error(
            "usage: credential <list|get|upsert-json|remove|vault> ...",
        )),
    }
}

fn meta_kernel_command(
    provider_run: Option<&crate::provider::RuntimeProviderRun>,
    metaagent: &crate::agent::AgentInstance,
    request: &LocalDaemonRequest,
) -> KernelCommand {
    let mut command = meta_kernel_command_without_request(metaagent, request);
    command.provider_run_id = provider_run.map(|run| run.id().to_string());
    command
}

fn meta_kernel_command_without_request(
    metaagent: &crate::agent::AgentInstance,
    request: &LocalDaemonRequest,
) -> KernelCommand {
    KernelCommand::from_local_request_with_caller(
        format!(
            "metaagent-{}-{}",
            metaagent.id(),
            crate::session::unix_epoch_ms()
        ),
        KernelCommandSource::DaemonBackground,
        KernelCaller {
            caller_id: format!("metaagent:{}", metaagent.id()),
            caller_kind: KernelCallerKind::Metaagent,
            user_id: Some(metaagent.owner_user_id().to_string()),
            client_id: Some(metaagent_command_client_id(metaagent.id())),
            machine_id: None,
            realm_id: None,
            public_key_thumbprint: None,
            metaagent_id: Some(metaagent.id().to_string()),
        },
        None,
        Some(metaagent.id().to_string()),
        request,
    )
}

fn metaagent_command_client_id(metaagent_id: &str) -> String {
    format!("metaagent:{metaagent_id}:commands")
}

fn meta_command_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "runtime_tool_meta.run_command",
        message: message.into(),
    }
}

fn meta_command_failure_result(command: &str, error: DaemonError) -> RuntimeToolResult {
    RuntimeToolResult {
        ok: false,
        payload: serde_json::json!({
            "command": redacted_meta_command_for_payload(command),
            "error": error.to_string(),
        }),
    }
}

fn meta_command_success_result(
    command: &str,
    response: &LocalDaemonResponse,
    metaagent: &crate::agent::AgentInstance,
) -> RuntimeToolResult {
    RuntimeToolResult {
        ok: true,
        payload: serde_json::json!({
            "command": redacted_meta_command_for_payload(command),
            "response": summarize_meta_command_response(response, metaagent),
        }),
    }
}

fn redacted_meta_command_for_payload(command: &str) -> String {
    let Ok(tokens) = crate::runtime::metaagent_command_registry::tokenize_command(command) else {
        return command.to_string();
    };
    match (
        tokens.first().map(String::as_str),
        tokens.get(1).map(String::as_str),
    ) {
        (Some("credential" | "credentials"), Some("set" | "set-secret" | "delete-secret")) => {
            let credential_ref = tokens.get(2).map_or("<credential-ref>", String::as_str);
            format!(
                "{} {} {} <redacted-secret>",
                tokens[0], tokens[1], credential_ref
            )
        }
        _ => command.to_string(),
    }
}

fn summarize_meta_command_response(
    response: &LocalDaemonResponse,
    metaagent: &crate::agent::AgentInstance,
) -> serde_json::Value {
    match response {
        LocalDaemonResponse::AgentSpawned { agent } => serde_json::json!({
            "type": "AgentSpawned",
            "agent": summarize_meta_agent(agent),
        }),
        LocalDaemonResponse::AgentAliased { agent, .. } => serde_json::json!({
            "type": "AgentAliased",
            "agent": summarize_meta_agent(agent),
        }),
        LocalDaemonResponse::AgentDestroyed { agent } => serde_json::json!({
            "type": "AgentDestroyed",
            "agent": summarize_meta_agent(agent),
        }),
        LocalDaemonResponse::AgentFocused { agent } => serde_json::json!({
            "type": "AgentFocused",
            "agent": summarize_meta_agent(agent),
        }),
        LocalDaemonResponse::AgentsListed { agents } => serde_json::json!({
            "type": "AgentsListed",
            "agents": agents
                .iter()
                .filter(|agent| {
                    !agent.is_metaagent()
                        && agent.controlled_by_metaagent_id() == Some(metaagent.id())
                })
                .map(summarize_meta_agent)
                .collect::<Vec<_>>(),
        }),
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => serde_json::json!({
            "type": "WorkflowCreated",
            "workflow": summarize_meta_workflow(workflow),
        }),
        LocalDaemonResponse::WorkflowAliased { workflow, .. }
        | LocalDaemonResponse::WorkflowResolved { workflow } => serde_json::json!({
            "type": "Workflow",
            "workflow": summarize_meta_workflow(workflow),
        }),
        LocalDaemonResponse::WorkflowsListed { workflows } => serde_json::json!({
            "type": "WorkflowsListed",
            "workflows": workflows.iter().map(summarize_meta_workflow).collect::<Vec<_>>(),
        }),
        LocalDaemonResponse::WorkflowNodeAdded { node, workflow, .. } => serde_json::json!({
            "type": "WorkflowNodeAdded",
            "node": summarize_meta_workflow_node(node),
            "workflow": summarize_meta_workflow(workflow),
        }),
        LocalDaemonResponse::WorkflowNodeRemoved { node, workflow, .. } => serde_json::json!({
            "type": "WorkflowNodeRemoved",
            "node": summarize_meta_workflow_node(node),
            "workflow": summarize_meta_workflow(workflow),
        }),
        LocalDaemonResponse::WorkflowNodeInstructionsUpdated { node, workflow, .. } => {
            serde_json::json!({
                "type": "WorkflowNodeInstructionsUpdated",
                "node": summarize_meta_workflow_node(node),
                "workflow": summarize_meta_workflow(workflow),
            })
        }
        LocalDaemonResponse::WorkflowNodeCanCompleteRunUpdated { node, workflow, .. } => {
            serde_json::json!({
                "type": "WorkflowNodeCanCompleteRunUpdated",
                "node": summarize_meta_workflow_node(node),
                "workflow": summarize_meta_workflow(workflow),
            })
        }
        LocalDaemonResponse::WorkflowNodeCanEmitIntermediateOutputUpdated {
            node,
            workflow,
            ..
        } => serde_json::json!({
            "type": "WorkflowNodeCanEmitIntermediateOutputUpdated",
            "node": summarize_meta_workflow_node(node),
            "workflow": summarize_meta_workflow(workflow),
        }),
        LocalDaemonResponse::WorkflowNodeWaitForAllInputsUpdated { node, workflow, .. } => {
            serde_json::json!({
                "type": "WorkflowNodeWaitForAllInputsUpdated",
                "node": summarize_meta_workflow_node(node),
                "workflow": summarize_meta_workflow(workflow),
            })
        }
        LocalDaemonResponse::WorkflowNodeMaxTurnsUpdated { node, workflow, .. } => {
            serde_json::json!({
                "type": "WorkflowNodeMaxTurnsUpdated",
                "node": summarize_meta_workflow_node(node),
                "workflow": summarize_meta_workflow(workflow),
            })
        }
        LocalDaemonResponse::WorkflowEndpointCreated {
            endpoint, workflow, ..
        } => serde_json::json!({
            "type": "WorkflowEndpointCreated",
            "endpoint": summarize_meta_workflow_endpoint(endpoint),
            "workflow": summarize_meta_workflow(workflow),
        }),
        LocalDaemonResponse::WorkflowEndpointAliased {
            endpoint, workflow, ..
        } => serde_json::json!({
            "type": "WorkflowEndpointAliased",
            "endpoint": summarize_meta_workflow_endpoint(endpoint),
            "workflow": summarize_meta_workflow(workflow),
        }),
        LocalDaemonResponse::WorkflowEdgeAdded { edge, workflow, .. } => serde_json::json!({
            "type": "WorkflowEdgeAdded",
            "edge": summarize_meta_workflow_edge(edge),
            "workflow": summarize_meta_workflow(workflow),
        }),
        LocalDaemonResponse::WorkflowEdgeRemoved { edge, workflow, .. } => serde_json::json!({
            "type": "WorkflowEdgeRemoved",
            "edge": summarize_meta_workflow_edge(edge),
            "workflow": summarize_meta_workflow(workflow),
        }),
        LocalDaemonResponse::WorkflowRunInvoked {
            workflow_run,
            workflow,
            endpoint,
            ..
        } => serde_json::json!({
            "type": "WorkflowRunInvoked",
            "workflow_run": summarize_meta_workflow_run(workflow_run),
            "workflow": summarize_meta_workflow(workflow),
            "endpoint": summarize_meta_workflow_endpoint(endpoint),
        }),
        LocalDaemonResponse::WorkflowRunsListed { workflow_runs } => serde_json::json!({
            "type": "WorkflowRunsListed",
            "workflow_runs": workflow_runs
                .iter()
                .map(summarize_meta_workflow_run)
                .collect::<Vec<_>>(),
        }),
        LocalDaemonResponse::WorkflowRun { workflow_run } => serde_json::json!({
            "type": "WorkflowRun",
            "workflow_run": summarize_meta_workflow_run(workflow_run),
        }),
        LocalDaemonResponse::WorkflowRunCancelled { workflow_run, .. } => serde_json::json!({
            "type": "WorkflowRunCancelled",
            "workflow_run": summarize_meta_workflow_run(workflow_run),
        }),
        LocalDaemonResponse::WorkflowRunResumed { workflow_run, .. } => serde_json::json!({
            "type": "WorkflowRunResumed",
            "workflow_run": summarize_meta_workflow_run(workflow_run),
        }),
        _ => serde_json::json!({
            "type": "CommandAccepted",
            "detail": "response omitted from metaagent tool output; inspect session_overview or a dedicated list command for current state",
        }),
    }
}

fn summarize_meta_agent(agent: &crate::agent::AgentInstance) -> serde_json::Value {
    serde_json::json!({
        "id": agent.id(),
        "agent_ref": agent.agent_ref(),
        "alias": agent.alias(),
        "role": agent.role(),
        "provider": agent.provider(),
        "model": agent.model(),
    })
}

fn summarize_meta_workflow(workflow: &crate::session::WorkflowDefinition) -> serde_json::Value {
    serde_json::json!({
        "id": workflow.id(),
        "alias": workflow.alias(),
        "revision": workflow.revision(),
        "nodes": workflow
            .nodes()
            .iter()
            .map(summarize_meta_workflow_node)
            .collect::<Vec<_>>(),
        "edges": workflow
            .edges()
            .iter()
            .map(summarize_meta_workflow_edge)
            .collect::<Vec<_>>(),
        "endpoints": workflow
            .endpoints()
            .iter()
            .map(summarize_meta_workflow_endpoint)
            .collect::<Vec<_>>(),
    })
}

fn summarize_meta_workflow_node(
    node: &crate::session::WorkflowNodeDefinition,
) -> serde_json::Value {
    serde_json::json!({
        "id": node.id(),
        "agent_id": node.agent_id(),
        "public_label": node.public_label(),
        "can_complete_workflow_run": node.can_complete_workflow_run(),
        "can_emit_intermediate_run_output": node.can_emit_intermediate_run_output(),
        "wait_for_all_inputs": node.wait_for_all_inputs(),
    })
}

fn summarize_meta_workflow_edge(
    edge: &crate::session::WorkflowEdgeDefinition,
) -> serde_json::Value {
    serde_json::json!({
        "id": edge.id(),
        "from_node_id": edge.from_node_id(),
        "to_node_id": edge.to_node_id(),
        "source_side": edge.source_side(),
        "target_side": edge.target_side(),
        "handoff_schema_ref": edge.handoff_schema_ref(),
        "validation_policy": edge.validation_policy(),
    })
}

fn summarize_meta_workflow_endpoint(
    endpoint: &crate::session::WorkflowEndpointDefinition,
) -> serde_json::Value {
    serde_json::json!({
        "id": endpoint.id(),
        "alias": endpoint.alias(),
        "entry_node_id": endpoint.entry_node_id(),
    })
}

fn summarize_meta_workflow_run(run: &crate::session::WorkflowRun) -> serde_json::Value {
    let active_node_run = run
        .active_node_run_id()
        .and_then(|active_node_run_id| {
            run.node_runs()
                .iter()
                .find(|node_run| node_run.id() == active_node_run_id)
        })
        .map(summarize_meta_workflow_node_run);
    let unconsumed_messages = run
        .messages()
        .iter()
        .filter(|message| message.consumed_by_node_run_id().is_none())
        .count();
    let latest_failure = run
        .failure_events()
        .last()
        .map(summarize_meta_workflow_failure);
    let latest_intermediate_output = run
        .intermediate_outputs()
        .last()
        .map(summarize_meta_workflow_intermediate_output);
    serde_json::json!({
        "id": run.id(),
        "workflow_id": run.workflow_id(),
        "endpoint_id": run.endpoint_id(),
        "entry_node_id": run.entry_node_id(),
        "status": run.status(),
        "invocation_prompt_present": run.invocation_prompt().is_some(),
        "active_node_run_id": run.active_node_run_id(),
        "active_node_run": active_node_run,
        "node_runs": run
            .node_runs()
            .iter()
            .map(summarize_meta_workflow_node_run)
            .collect::<Vec<_>>(),
        "node_run_counts_by_status": summarize_meta_workflow_node_run_counts(run),
        "message_count": run.messages().len(),
        "unconsumed_message_count": unconsumed_messages,
        "messages": run
            .messages()
            .iter()
            .map(summarize_meta_workflow_message)
            .collect::<Vec<_>>(),
        "failure_count": run.failure_events().len(),
        "latest_failure": latest_failure,
        "failure_events": run
            .failure_events()
            .iter()
            .map(summarize_meta_workflow_failure)
            .collect::<Vec<_>>(),
        "intermediate_output_count": run.intermediate_outputs().len(),
        "latest_intermediate_output": latest_intermediate_output,
        "final_output_present": run.final_output().is_some(),
        "final_output_valid": run.final_output_valid(),
        "final_output_warning": run.final_output_warning(),
        "final_output": run.final_output().map(summarize_meta_workflow_output_payload),
        "completed_by_node_run_id": run.completed_by_node_run_id(),
    })
}

fn summarize_meta_workflow_node_run(
    node_run: &crate::session::WorkflowNodeRun,
) -> serde_json::Value {
    let turn = node_run.turn_envelope();
    let completion = node_run.completion();
    serde_json::json!({
        "id": node_run.id(),
        "node_id": node_run.node_id(),
        "agent_id": node_run.agent_id(),
        "status": node_run.status(),
        "summary": node_run.summary(),
        "created_at_ms": node_run.created_at_ms(),
        "started_at_ms": node_run.started_at_ms(),
        "completed_at_ms": node_run.completed_at_ms(),
        "completion": completion.map(summarize_meta_workflow_completion),
        "turn": turn.map(summarize_meta_workflow_turn),
        "thinking_trace_count": node_run.thinking_traces().len(),
        "has_valid_pending_final_output": node_run.has_valid_pending_final_output(),
    })
}

fn summarize_meta_workflow_node_run_counts(run: &crate::session::WorkflowRun) -> serde_json::Value {
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for node_run in run.node_runs() {
        *counts
            .entry(format!("{:?}", node_run.status()))
            .or_default() += 1;
    }
    serde_json::json!(counts)
}

fn summarize_meta_workflow_turn(turn: &crate::session::WorkflowTurnEnvelope) -> serde_json::Value {
    serde_json::json!({
        "delivery_token": turn.delivery_token(),
        "state": turn.state(),
        "rendered_prompt_present": turn.rendered_prompt().is_some(),
        "mailbox_content_present": turn.mailbox_content().is_some(),
        "handoff_payloads_present": turn.handoff_payloads_json().is_some(),
        "runtime_tool_call_count": turn.runtime_tool_calls().len(),
        "pending_output_submissions": turn
            .pending_output_submissions()
            .map(summarize_meta_workflow_pending_outputs),
        "intermediate_released_downstream": turn.intermediate_released_downstream(),
    })
}

fn summarize_meta_workflow_pending_outputs(
    submissions: &crate::session::WorkflowTurnOutputSubmissions,
) -> serde_json::Value {
    serde_json::json!({
        "intermediate": submissions
            .intermediate()
            .map(summarize_meta_workflow_output_submission),
        "final": submissions
            .final_output()
            .map(summarize_meta_workflow_output_submission),
    })
}

fn summarize_meta_workflow_output_submission(
    submission: &crate::session::WorkflowRunOutputSubmission,
) -> serde_json::Value {
    serde_json::json!({
        "valid": submission.valid(),
        "warning": submission.warning(),
        "submitted_at_ms": submission.submitted_at_ms(),
        "output": summarize_meta_workflow_output_payload(submission.output()),
    })
}

fn summarize_meta_workflow_completion(
    completion: &crate::session::WorkflowCompletionSnapshot,
) -> serde_json::Value {
    serde_json::json!({
        "summary": trim_meta_text(completion.summary(), 512),
        "output": completion.output().map(summarize_meta_workflow_output_payload),
    })
}

fn summarize_meta_workflow_message(message: &crate::session::WorkflowMessage) -> serde_json::Value {
    serde_json::json!({
        "id": message.id(),
        "source_node_run_id": message.source_node_run_id(),
        "target_node_id": message.target_node_id(),
        "message_type": message.message_type(),
        "summary": trim_meta_text(message.summary(), 512),
        "handoff_payload_present": !message.handoff_payload().is_empty(),
        "consumed_by_node_run_id": message.consumed_by_node_run_id(),
        "created_at_ms": message.created_at_ms(),
    })
}

fn summarize_meta_workflow_failure(
    failure: &crate::session::WorkflowFailureEvent,
) -> serde_json::Value {
    serde_json::json!({
        "kind": failure.kind(),
        "source_node_run_id": failure.source_node_run_id(),
        "edge_ids": failure.edge_ids(),
        "message": trim_meta_text(failure.message(), 1024),
        "timestamp_ms": failure.timestamp_ms(),
    })
}

fn summarize_meta_workflow_intermediate_output(
    output: &crate::session::WorkflowIntermediateOutput,
) -> serde_json::Value {
    serde_json::json!({
        "id": output.id(),
        "source_node_run_id": output.source_node_run_id(),
        "valid": output.valid(),
        "warning": output.warning(),
        "timestamp_ms": output.timestamp_ms(),
        "output": summarize_meta_workflow_output_payload(output.output()),
    })
}

fn summarize_meta_workflow_output_payload(
    output: &crate::session::WorkflowOutputPayload,
) -> serde_json::Value {
    serde_json::json!({
        "message": trim_meta_text(output.message(), 1024),
        "artifacts": output
            .artifacts()
            .iter()
            .map(|artifact| serde_json::json!({
                "id": artifact.id(),
                "kind": artifact.kind(),
                "path": artifact.path(),
                "display_name": artifact.display_name(),
            }))
            .collect::<Vec<_>>(),
    })
}

fn trim_meta_text(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let trimmed = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{trimmed}...")
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_session() -> crate::session::RuntimeSession {
        crate::session::RuntimeSession::new(
            "session-1",
            None,
            "workspace",
            "/repo",
            "machine",
            "daemon",
        )
    }

    #[test]
    fn meta_agent_spawn_parser_supports_new_slice_launch_parameters() {
        let args = vec![
            "builder".to_string(),
            "gpt-5.5".to_string(),
            "--slice".to_string(),
            "new:headed".to_string(),
            "--kernel".to_string(),
            "linux-worker".to_string(),
        ];

        let parsed = parse_meta_agent_spawn_args(&args, &test_session())
            .expect("new slice spawn args should parse");

        assert_eq!(parsed.alias.as_deref(), Some("builder"));
        assert_eq!(parsed.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(parsed.kernel_ref.as_deref(), Some("linux-worker"));
        assert!(parsed.slice_ref.is_none());
        assert_eq!(
            parsed.slice_create.map(|create| create.display_mode),
            Some(crate::slice::SliceDisplayMode::Headed)
        );
    }

    #[test]
    fn meta_agent_spawn_parser_supports_existing_slice_placement() {
        let args = vec![
            "checker".to_string(),
            "--slice".to_string(),
            "linux-dev".to_string(),
        ];

        let parsed = parse_meta_agent_spawn_args(&args, &test_session())
            .expect("existing slice spawn args should parse");

        assert_eq!(parsed.alias.as_deref(), Some("checker"));
        assert_eq!(parsed.slice_ref.as_deref(), Some("linux-dev"));
        assert!(parsed.slice_create.is_none());
    }

    #[test]
    fn meta_extension_import_parser_supports_provider_sync() {
        let args = vec![
            "import".to_string(),
            "providers".to_string(),
            "--provider".to_string(),
            "codex".to_string(),
            "--provider".to_string(),
            "claude".to_string(),
            "--kind".to_string(),
            "skill".to_string(),
            "--name".to_string(),
            "docs-helper".to_string(),
            "--dry-run".to_string(),
        ];

        let request = meta_extension_import_request(&test_session(), &args)
            .expect("extension import args should parse");

        let LocalDaemonRequest::ImportProviderCapabilities(request) = request else {
            panic!("unexpected request");
        };
        assert_eq!(request.workspace_id.as_deref(), Some("workspace"));
        assert_eq!(request.providers, vec!["codex", "claude"]);
        assert_eq!(request.kind.as_deref(), Some("skill"));
        assert_eq!(request.name.as_deref(), Some("docs-helper"));
        assert!(request.dry_run);
    }

    #[test]
    fn meta_agent_spawn_parser_supports_provider_model_and_effort() {
        let args = vec![
            "verifier".to_string(),
            "--provider".to_string(),
            "opencode".to_string(),
            "--model".to_string(),
            "opencode/gpt-5.2".to_string(),
            "--variant".to_string(),
            "high".to_string(),
        ];

        let parsed = parse_meta_agent_spawn_args(&args, &test_session())
            .expect("provider launch profile should parse");

        assert_eq!(parsed.alias.as_deref(), Some("verifier"));
        assert_eq!(parsed.provider.as_deref(), Some("opencode"));
        assert_eq!(parsed.model.as_deref(), Some("opencode/gpt-5.2"));
        assert_eq!(parsed.effort.as_deref(), Some("high"));
    }

    #[test]
    fn meta_agent_spawn_parser_rejects_positional_and_flag_model() {
        let args = vec![
            "verifier".to_string(),
            "gpt-5.5".to_string(),
            "--model".to_string(),
            "opencode/gpt-5.2".to_string(),
        ];

        let error = parse_meta_agent_spawn_args(&args, &test_session())
            .expect_err("model should be specified once");

        assert!(format!("{error}").contains("either positional [model] or --model"));
    }
}
