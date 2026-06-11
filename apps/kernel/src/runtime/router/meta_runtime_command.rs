use crate::attachment::ClientCapabilityLevel;
use crate::error::DaemonError;
use crate::local::{
    AliasAgentRequest, AttachToSessionRequest, CancelWorkflowRunRequest, CreateWorkflowRequest,
    DestroyAgentRequest, FocusAgentRequest, InvokeWorkflowEndpointRequest, ListAgentsRequest,
    ListWorkflowRunsRequest, ListWorkflowsRequest, LocalDaemonRequest, LocalDaemonResponse,
    ResumeWorkflowRunRequest, SpawnAgentRequest,
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
        let tokens = match tokenize_meta_command(&args.command) {
            Ok(tokens) => tokens,
            Err(error) => return Ok(meta_command_failure_result(&args.command, error)),
        };
        match crate::runtime::metaagent_command_registry::execution_policy(&tokens) {
            crate::runtime::metaagent_command_registry::MetaCommandExecutionPolicy::Routed => {}
            crate::runtime::metaagent_command_registry::MetaCommandExecutionPolicy::Denied {
                message,
            } => {
                return Ok(meta_command_failure_result(
                    &args.command,
                    meta_command_error(message),
                ))
            }
            crate::runtime::metaagent_command_registry::MetaCommandExecutionPolicy::NotRouted {
                message,
            } => {
                return Ok(meta_command_failure_result(
                    &args.command,
                    meta_command_error(message),
                ))
            }
        }
        if tokens.first().map(String::as_str) == Some("prompt") {
            return self
                .dispatch_meta_prompt_command(&session, &metaagent, &args.command, &tokens[1..])
                .await;
        }
        let request = match self
            .meta_command_request(&session, &metaagent, &tokens)
            .await
        {
            Ok(request) => request,
            Err(error) => return Ok(meta_command_failure_result(&args.command, error)),
        };
        let command = meta_kernel_command(Some(&provider_run), &metaagent, &request);
        let response = match self.dispatch(command, request).await {
            Ok(response) => response,
            Err(error) => return Ok(meta_command_failure_result(&args.command, error)),
        };
        Ok(RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "command": args.command,
                "response": response,
            }),
        })
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
        let tokens = match tokenize_meta_command(&args.command) {
            Ok(tokens) => tokens,
            Err(error) => return Ok(meta_command_failure_result(&args.command, error)),
        };
        match crate::runtime::metaagent_command_registry::execution_policy(&tokens) {
            crate::runtime::metaagent_command_registry::MetaCommandExecutionPolicy::Routed => {}
            crate::runtime::metaagent_command_registry::MetaCommandExecutionPolicy::Denied {
                message,
            } => {
                return Ok(meta_command_failure_result(
                    &args.command,
                    meta_command_error(message),
                ))
            }
            crate::runtime::metaagent_command_registry::MetaCommandExecutionPolicy::NotRouted {
                message,
            } => {
                return Ok(meta_command_failure_result(
                    &args.command,
                    meta_command_error(message),
                ))
            }
        }
        if tokens.first().map(String::as_str) == Some("prompt") {
            return self
                .dispatch_meta_prompt_command(&session, &metaagent, &args.command, &tokens[1..])
                .await;
        }
        let request = match self
            .meta_command_request(&session, &metaagent, &tokens)
            .await
        {
            Ok(request) => request,
            Err(error) => return Ok(meta_command_failure_result(&args.command, error)),
        };
        let command = meta_kernel_command(None, &metaagent, &request);
        let response = match self.dispatch(command, request).await {
            Ok(response) => response,
            Err(error) => return Ok(meta_command_failure_result(&args.command, error)),
        };
        Ok(RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "command": args.command,
                "response": response,
            }),
        })
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
            .submit_metaagent_command_prompt(session.id(), &attachment_id, target.id(), prompt)
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
            "workflow" => meta_workflow_request(session, &tokens[1..]),
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
    args: &[String],
) -> Result<LocalDaemonRequest, DaemonError> {
    match args.first().map(String::as_str) {
        Some("list" | "ls") | None => Ok(LocalDaemonRequest::ListWorkflows(ListWorkflowsRequest {
            session_id: session.id().to_string(),
        })),
        Some("new" | "create") => {
            if args.len() > 2 {
                return Err(meta_command_error("usage: workflow new [alias]"));
            }
            Ok(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
                session_id: session.id().to_string(),
                alias: args.get(1).cloned(),
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
            "usage: workflow <list|new|run|runs|cancel|resume> ...",
        )),
    }
}

fn tokenize_meta_command(input: &str) -> Result<Vec<String>, DaemonError> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaping = false;
    for ch in input.trim().chars() {
        if escaping {
            current.push(ch);
            escaping = false;
            continue;
        }
        if ch == '\\' && quote != Some('\'') {
            escaping = true;
            continue;
        }
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            } else {
                current.push(ch);
            }
            continue;
        }
        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            continue;
        }
        if ch.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(ch);
    }
    if escaping {
        current.push('\\');
    }
    if quote.is_some() {
        return Err(meta_command_error("unterminated quote"));
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
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
            caller_kind: KernelCallerKind::LocalClient,
            user_id: Some(metaagent.owner_user_id().to_string()),
            client_id: Some(metaagent_command_client_id(metaagent.id())),
            machine_id: None,
            realm_id: None,
            public_key_thumbprint: None,
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
