use crate::attachment::ClientCapabilityLevel;
use crate::error::DaemonError;
use crate::local::{
    AddWorkflowNodeRequest, AliasAgentRequest, AttachToSessionRequest, CancelWorkflowRunRequest,
    CreateWorkflowEndpointRequest, CreateWorkflowRequest, DestroyAgentRequest, ExtensionKind,
    FocusAgentRequest, GetCredentialRequest, GetCredentialVaultStatusRequest, GetMcpServerRequest,
    GetSkillRequest, GrantAgentExtensionRequest, ImportMcpServersRequest, ImportSkillsRequest,
    InstallMcpServerRequest, InstallSkillRequest, InvokeWorkflowEndpointRequest, ListAgentsRequest,
    ListCredentialsRequest, ListMcpServersRequest, ListSkillsRequest, ListWorkflowRunsRequest,
    ListWorkflowsRequest, LocalDaemonRequest, LocalDaemonResponse, ManageCredentialVaultRequest,
    RemoveCredentialRequest, ResumeWorkflowRunRequest, RevokeAgentExtensionRequest,
    SpawnAgentRequest, UninstallMcpServerRequest, UninstallSkillRequest, UpdateMcpServerRequest,
    UpdateSkillRequest, UpsertCredentialRequest,
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
        let result = meta_command_success_result(&args.command, &response);
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
        let result = meta_command_success_result(&args.command, &response);
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
                "command": command,
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
                    "command": command,
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
        let prompt = args[1..].join(" ");
        let attachment_id = match self
            .ensure_metaagent_command_attachment(session.id(), metaagent)
            .await
        {
            Ok(attachment_id) => attachment_id,
            Err(error) => return Ok(meta_command_failure_result(command, error)),
        };
        let mut result = self
            .runtime_state
            .submit_metaagent_command_prompt(
                session.id(),
                metaagent,
                &attachment_id,
                target.id(),
                prompt,
            )
            .await?;
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
                    && agent.owner_user_id() == metaagent.owner_user_id()
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
            let spawn_args = &args[1..];
            if spawn_args
                .iter()
                .any(|arg| matches!(arg.as_str(), "--meta" | "--metaagent" | "--as-metaagent"))
            {
                return Err(meta_command_error(
                    "metaagents cannot spawn another metaagent through run_command",
                ));
            }
            if spawn_args.iter().any(|arg| arg.starts_with("--slice")) {
                return Err(meta_command_error(
                    "metaagent run_command does not enable slice placement yet",
                ));
            }
            if spawn_args.len() > 2 {
                return Err(meta_command_error("usage: agent spawn [alias] [model]"));
            }
            Ok(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                session_id: session.id().to_string(),
                alias: spawn_args.first().cloned(),
                provider: Some(metaagent.provider().to_string()),
                model: spawn_args
                    .get(1)
                    .cloned()
                    .or_else(|| metaagent.model().map(str::to_string)),
                effort: metaagent.effort().map(str::to_string),
                execution_mode: metaagent.execution_mode_override(),
                permission_level: metaagent.permission_level_override(),
                worktree_id: metaagent.worktree_id().map(str::to_string),
                kernel_ref: None,
                slice_ref: None,
                worktree_placement: None,
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
                && agent.owner_user_id() == metaagent.owner_user_id()
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
            _ => Err(meta_command_error(
                "usage: workflow node add <workflow-ref> <owned-agent-ref>",
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
            _ => Err(meta_command_error(
                "usage: workflow endpoint new <workflow-ref> <entry-node-id> [alias]",
            )),
        },
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
            "usage: workflow <list|new|node|endpoint|run|runs|cancel|resume> ...",
        )),
    }
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
            "command": command,
            "error": error.to_string(),
        }),
    }
}

fn meta_command_success_result(command: &str, response: &LocalDaemonResponse) -> RuntimeToolResult {
    RuntimeToolResult {
        ok: true,
        payload: serde_json::json!({
            "command": command,
            "response": summarize_meta_command_response(response),
        }),
    }
}

fn summarize_meta_command_response(response: &LocalDaemonResponse) -> serde_json::Value {
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
            "agents": agents.iter().map(summarize_meta_agent).collect::<Vec<_>>(),
        }),
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => serde_json::json!({
            "type": "WorkflowCreated",
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
        LocalDaemonResponse::WorkflowEndpointCreated {
            endpoint, workflow, ..
        } => serde_json::json!({
            "type": "WorkflowEndpointCreated",
            "endpoint": summarize_meta_workflow_endpoint(endpoint),
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
    serde_json::json!({
        "id": run.id(),
        "workflow_id": run.workflow_id(),
        "endpoint_id": run.endpoint_id(),
        "entry_node_id": run.entry_node_id(),
        "status": run.status(),
        "active_node_run_id": run.active_node_run_id(),
    })
}
