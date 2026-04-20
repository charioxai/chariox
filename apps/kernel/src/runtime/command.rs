use serde::{Deserialize, Serialize};
use serde_json::Value;

use arroba_relay::auth::RelaySubjectKind;
use arroba_relay::protocol::RelayCallerIdentity;

use crate::local::LocalDaemonRequest;
use crate::session::unix_epoch_ms;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelCommandSource {
    LocalCli,
    LocalIpc,
    RelayClient,
    RelayPeer,
    DaemonBackground,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelCallerKind {
    LocalClient,
    RemoteClient,
    RemoteKernel,
    HostedService,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelCaller {
    pub caller_id: String,
    pub caller_kind: KernelCallerKind,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub machine_id: Option<String>,
    #[serde(default)]
    pub realm_id: Option<String>,
    #[serde(default)]
    pub public_key_thumbprint: Option<String>,
}

impl Default for KernelCaller {
    fn default() -> Self {
        Self::for_source(&KernelCommandSource::LocalCli)
    }
}

impl KernelCaller {
    pub fn for_source(source: &KernelCommandSource) -> Self {
        let (caller_id, caller_kind) = match source {
            KernelCommandSource::LocalCli => ("local-cli", KernelCallerKind::LocalClient),
            KernelCommandSource::LocalIpc => ("local-ipc", KernelCallerKind::LocalClient),
            KernelCommandSource::RelayClient => {
                ("relay-client-unverified", KernelCallerKind::RemoteClient)
            }
            KernelCommandSource::RelayPeer => {
                ("relay-peer-unverified", KernelCallerKind::RemoteKernel)
            }
            KernelCommandSource::DaemonBackground => {
                ("daemon-background", KernelCallerKind::LocalClient)
            }
        };
        Self {
            caller_id: caller_id.to_string(),
            caller_kind,
            user_id: None,
            client_id: None,
            machine_id: None,
            realm_id: None,
            public_key_thumbprint: None,
        }
    }

    pub fn from_relay_identity(identity: RelayCallerIdentity) -> Self {
        let (caller_kind, client_id, machine_id) = match identity.subject_kind {
            RelaySubjectKind::Client => (
                KernelCallerKind::RemoteClient,
                Some(identity.subject.clone()),
                None,
            ),
            RelaySubjectKind::Kernel | RelaySubjectKind::Machine => (
                KernelCallerKind::RemoteKernel,
                None,
                Some(identity.subject.clone()),
            ),
            RelaySubjectKind::Service => (KernelCallerKind::HostedService, None, None),
        };
        Self {
            caller_id: identity.subject,
            caller_kind,
            user_id: identity.user_id,
            client_id,
            machine_id,
            realm_id: Some(identity.realm_id),
            public_key_thumbprint: identity.public_key_thumbprint,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelCommandPriority {
    Interactive,
    Normal,
    Background,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KernelCommand {
    pub command_id: String,
    pub command_type: String,
    pub submitted_at_ms: u64,
    pub source: KernelCommandSource,
    #[serde(default)]
    pub caller: KernelCaller,
    pub session_id: Option<String>,
    pub attachment_id: Option<String>,
    pub agent_id: Option<String>,
    pub provider_run_id: Option<String>,
    pub workflow_run_id: Option<String>,
    pub node_run_id: Option<String>,
    pub idempotency_key: Option<String>,
    pub causation_id: Option<String>,
    pub correlation_id: String,
    pub priority: KernelCommandPriority,
    pub payload: Value,
}

impl KernelCommand {
    pub fn from_local_request(
        command_id: impl Into<String>,
        correlation_id: Option<String>,
        causation_id: Option<String>,
        request: &LocalDaemonRequest,
    ) -> Self {
        Self::from_local_request_with_source(
            command_id,
            KernelCommandSource::LocalCli,
            correlation_id,
            causation_id,
            request,
        )
    }

    pub fn from_local_request_with_source(
        command_id: impl Into<String>,
        source: KernelCommandSource,
        correlation_id: Option<String>,
        causation_id: Option<String>,
        request: &LocalDaemonRequest,
    ) -> Self {
        let caller = KernelCaller::for_source(&source);
        Self::from_local_request_with_caller(
            command_id,
            source,
            caller,
            correlation_id,
            causation_id,
            request,
        )
    }

    pub fn from_local_request_with_caller(
        command_id: impl Into<String>,
        source: KernelCommandSource,
        caller: KernelCaller,
        correlation_id: Option<String>,
        causation_id: Option<String>,
        request: &LocalDaemonRequest,
    ) -> Self {
        let command_id = command_id.into();
        let payload = serde_json::to_value(request).unwrap_or(Value::Null);
        let metadata = local_request_metadata(request);
        Self {
            command_id: command_id.clone(),
            command_type: metadata.command_type.to_string(),
            submitted_at_ms: unix_epoch_ms(),
            source,
            caller,
            session_id: metadata.session_id,
            attachment_id: metadata.attachment_id,
            agent_id: metadata.agent_id,
            provider_run_id: metadata.provider_run_id,
            workflow_run_id: metadata.workflow_run_id,
            node_run_id: metadata.node_run_id,
            idempotency_key: None,
            causation_id,
            correlation_id: correlation_id.unwrap_or(command_id),
            priority: metadata.priority,
            payload,
        }
    }
}

#[derive(Debug)]
struct LocalRequestMetadata {
    command_type: &'static str,
    priority: KernelCommandPriority,
    session_id: Option<String>,
    attachment_id: Option<String>,
    agent_id: Option<String>,
    provider_run_id: Option<String>,
    workflow_run_id: Option<String>,
    node_run_id: Option<String>,
}

impl LocalRequestMetadata {
    fn new(command_type: &'static str, priority: KernelCommandPriority) -> Self {
        Self {
            command_type,
            priority,
            session_id: None,
            attachment_id: None,
            agent_id: None,
            provider_run_id: None,
            workflow_run_id: None,
            node_run_id: None,
        }
    }

    fn session(mut self, session_id: &str) -> Self {
        self.session_id = Some(session_id.to_string());
        self
    }

    fn attachment(mut self, attachment_id: &str) -> Self {
        self.attachment_id = Some(attachment_id.to_string());
        self
    }

    fn agent(mut self, agent_id: &str) -> Self {
        self.agent_id = Some(agent_id.to_string());
        self
    }

    fn provider_run(mut self, provider_run_id: &str) -> Self {
        self.provider_run_id = Some(provider_run_id.to_string());
        self
    }

    fn workflow_run(mut self, workflow_run_id: &str) -> Self {
        self.workflow_run_id = Some(workflow_run_id.to_string());
        self
    }
}

fn local_request_metadata(request: &LocalDaemonRequest) -> LocalRequestMetadata {
    use KernelCommandPriority::{Background, Interactive, Normal};

    match request {
        LocalDaemonRequest::CreateSession(_) => {
            LocalRequestMetadata::new("session.create", Interactive)
        }
        LocalDaemonRequest::AttachToSession(request) => {
            LocalRequestMetadata::new("session.attach", Interactive).session(&request.session_id)
        }
        LocalDaemonRequest::DetachFromSession(request) => {
            LocalRequestMetadata::new("session.detach", Interactive)
                .attachment(&request.attachment_id)
        }
        LocalDaemonRequest::SubmitPrompt(request) => {
            let mut metadata = LocalRequestMetadata::new("prompt.submit", Interactive)
                .session(&request.session_id)
                .attachment(&request.attachment_id);
            if let Some(agent_id) = request.target_agent_id.as_deref() {
                metadata = metadata.agent(agent_id);
            }
            metadata
        }
        LocalDaemonRequest::CancelActivePrompt(request) => {
            LocalRequestMetadata::new("prompt.cancel", Interactive)
                .session(&request.session_id)
                .attachment(&request.attachment_id)
        }
        LocalDaemonRequest::ResizeTerminal(request) => {
            LocalRequestMetadata::new("terminal.resize", Interactive).session(&request.session_id)
        }
        LocalDaemonRequest::PollRuntimeNotices(request) => {
            LocalRequestMetadata::new("runtime_notice.poll", Interactive)
                .session(&request.session_id)
                .attachment(&request.attachment_id)
        }
        LocalDaemonRequest::UpdateSessionConfig(request) => {
            LocalRequestMetadata::new("session.config.update", Interactive)
                .session(&request.session_id)
                .attachment(&request.attachment_id)
        }
        LocalDaemonRequest::AliasSession(request) => {
            LocalRequestMetadata::new("session.alias", Interactive).session(&request.session_id)
        }
        LocalDaemonRequest::FocusAgent(request) => {
            LocalRequestMetadata::new("agent.focus", Interactive)
                .session(&request.session_id)
                .agent(&request.agent_id)
        }
        LocalDaemonRequest::CycleAgentFocus(request) => {
            LocalRequestMetadata::new("agent.cycle_focus", Interactive).session(&request.session_id)
        }
        LocalDaemonRequest::GrantAgentCapability(request) => {
            LocalRequestMetadata::new("agent.capability.grant", Interactive)
                .agent(&request.agent_ref)
        }
        LocalDaemonRequest::RevokeAgentCapability(request) => {
            LocalRequestMetadata::new("agent.capability.revoke", Interactive)
                .agent(&request.agent_ref)
        }
        LocalDaemonRequest::EndSession(request) => {
            LocalRequestMetadata::new("session.end", Interactive).session(&request.session_id)
        }
        LocalDaemonRequest::DeleteSession(request) => {
            LocalRequestMetadata::new("session.delete", Interactive).session(&request.session_ref)
        }
        LocalDaemonRequest::SpawnAgent(request) => {
            LocalRequestMetadata::new("agent.spawn", Interactive).session(&request.session_id)
        }
        LocalDaemonRequest::DestroyAgent(request) => {
            LocalRequestMetadata::new("agent.destroy", Interactive)
                .session(&request.session_id)
                .agent(&request.agent_id)
        }
        LocalDaemonRequest::GetSessionHistory(request) => {
            let mut metadata = LocalRequestMetadata::new("session.history.get", Background)
                .session(&request.session_id);
            if let Some(agent_id) = request.agent_id.as_deref() {
                metadata = metadata.agent(agent_id);
            }
            metadata
        }
        LocalDaemonRequest::GetDaemonHealth(_) => {
            LocalRequestMetadata::new("daemon.health.get", Normal)
        }
        LocalDaemonRequest::GetProviderRun(request) => {
            LocalRequestMetadata::new("provider_run.get", Normal)
                .provider_run(&request.provider_run_id)
        }
        LocalDaemonRequest::CancelWorkflowRun(request) => {
            LocalRequestMetadata::new("workflow_run.cancel", Normal)
                .session(&request.session_id)
                .workflow_run(&request.workflow_run_ref)
        }
        LocalDaemonRequest::ResumeWorkflowRun(request) => {
            LocalRequestMetadata::new("workflow_run.resume", Normal)
                .session(&request.session_id)
                .workflow_run(&request.workflow_run_ref)
        }
        _ => LocalRequestMetadata::new(local_request_command_type(request), Normal),
    }
}

fn local_request_command_type(request: &LocalDaemonRequest) -> &'static str {
    match request {
        LocalDaemonRequest::CreateSession(_) => "session.create",
        LocalDaemonRequest::LaunchProviderRun(_) => "provider_run.launch",
        LocalDaemonRequest::ListSessionMembers(_) => "session.members.list",
        LocalDaemonRequest::CreateSessionInvite(_) => "session.invite.create",
        LocalDaemonRequest::JoinSessionInvite(_) => "session.invite.join",
        LocalDaemonRequest::RevokeSessionInvite(_) => "session.invite.revoke",
        LocalDaemonRequest::ListSessions(_) => "session.list",
        LocalDaemonRequest::ResolveSession(_) => "session.resolve",
        LocalDaemonRequest::GetSessionState(_) => "session.state.get",
        LocalDaemonRequest::GetProviderCatalog(_) => "provider.catalog.get",
        LocalDaemonRequest::GetProviderCommandCatalogs(_) => "provider.command_catalogs.get",
        LocalDaemonRequest::InstallMcpServer(_) => "mcp.install",
        LocalDaemonRequest::UpdateMcpServer(_) => "mcp.update",
        LocalDaemonRequest::UninstallMcpServer(_) => "mcp.uninstall",
        LocalDaemonRequest::ImportMcpServers(_) => "mcp.import",
        LocalDaemonRequest::GetMcpServer(_) => "mcp.get",
        LocalDaemonRequest::ListMcpServers(_) => "mcp.list",
        LocalDaemonRequest::InstallSkill(_) => "skill.install",
        LocalDaemonRequest::UpdateSkill(_) => "skill.update",
        LocalDaemonRequest::UninstallSkill(_) => "skill.uninstall",
        LocalDaemonRequest::ImportSkills(_) => "skill.import",
        LocalDaemonRequest::GetSkill(_) => "skill.get",
        LocalDaemonRequest::ListSkills(_) => "skill.list",
        LocalDaemonRequest::RelayStatus(_) => "relay.status",
        LocalDaemonRequest::ConfigureRelay(_) => "relay.configure",
        LocalDaemonRequest::GetUserConfig(_) => "config.get",
        LocalDaemonRequest::SetUserConfigValue(_) => "config.set",
        LocalDaemonRequest::UnsetUserConfigValue(_) => "config.unset",
        LocalDaemonRequest::ListRemoteMachines(_) => "remote_machine.list",
        LocalDaemonRequest::ListRemoteMachineKernels(_) => "remote_machine.kernel.list",
        LocalDaemonRequest::ApproveRemoteMachine(_) => "remote_machine.approve",
        LocalDaemonRequest::ForgetRemoteMachine(_) => "remote_machine.forget",
        LocalDaemonRequest::RenameRemoteMachine(_) => "remote_machine.rename",
        LocalDaemonRequest::CreatePairingInvite(_) => "pairing_invite.create",
        LocalDaemonRequest::JoinPairingInvite(_) => "pairing_invite.join",
        LocalDaemonRequest::ListPairedClients(_) => "paired_client.list",
        LocalDaemonRequest::RecordPairedClient(_) => "paired_client.record",
        LocalDaemonRequest::RevokePairedClient(_) => "paired_client.revoke",
        LocalDaemonRequest::GetProviderAuthStatus(_) => "provider.auth_status.get",
        LocalDaemonRequest::StartProviderLogin(_) => "provider.login.start",
        LocalDaemonRequest::LogoutProvider(_) => "provider.logout",
        LocalDaemonRequest::ListProviderProcesses(_) => "provider_process.list",
        LocalDaemonRequest::TeardownProviderProcesses(_) => "provider_process.teardown",
        LocalDaemonRequest::QueryHistory(_) => "history.query",
        LocalDaemonRequest::SearchHistory(_) => "history.search",
        LocalDaemonRequest::PollRuntimeNotices(_) => "runtime_notice.poll",
        LocalDaemonRequest::CompletePrompt(_) => "prompt.complete",
        LocalDaemonRequest::UpdateSessionConfig(_) => "session.config.update",
        LocalDaemonRequest::PumpTerminalOutput(_) => "terminal.output.poll",
        LocalDaemonRequest::RunShellCommand(_) => "capability.shell.run",
        LocalDaemonRequest::ReadDirectoryTree(_) => "capability.dir.tree",
        LocalDaemonRequest::ReadFile(_) => "capability.file.read",
        LocalDaemonRequest::EditFile(_) => "capability.file.edit",
        LocalDaemonRequest::InspectGit(_) => "capability.git.inspect",
        LocalDaemonRequest::CaptureScreenshot(_) => "capability.screenshot.capture",
        LocalDaemonRequest::StoreTransferredFile(_) => "capability.file.store_transferred",
        LocalDaemonRequest::AliasSession(_) => "session.alias",
        LocalDaemonRequest::SpawnAgent(_) => "agent.spawn",
        LocalDaemonRequest::MoveAgentToRemote(_) => "agent.move_remote",
        LocalDaemonRequest::DestroyAgent(_) => "agent.destroy",
        LocalDaemonRequest::GrantAgentCapability(_) => "agent.capability.grant",
        LocalDaemonRequest::RevokeAgentCapability(_) => "agent.capability.revoke",
        LocalDaemonRequest::ListAgents(_) => "agent.list",
        LocalDaemonRequest::CreateWorkflow(_) => "workflow.create",
        LocalDaemonRequest::AliasWorkflow(_) => "workflow.alias",
        LocalDaemonRequest::ListWorkflows(_) => "workflow.list",
        LocalDaemonRequest::ResolveWorkflow(_) => "workflow.resolve",
        LocalDaemonRequest::CreateWorkflowEndpoint(_) => "workflow_endpoint.create",
        LocalDaemonRequest::AliasWorkflowEndpoint(_) => "workflow_endpoint.alias",
        LocalDaemonRequest::BindWorkflowEndpoint(_) => "workflow_endpoint.bind",
        LocalDaemonRequest::AddWorkflowNode(_) => "workflow_node.add",
        LocalDaemonRequest::RemoveWorkflowNode(_) => "workflow_node.remove",
        LocalDaemonRequest::UpdateWorkflowNodeInstructions(_) => {
            "workflow_node.instructions.update"
        }
        LocalDaemonRequest::SetWorkflowNodeCanCompleteRun(_) => {
            "workflow_node.can_complete_run.set"
        }
        LocalDaemonRequest::SetWorkflowNodeCanEmitIntermediateOutput(_) => {
            "workflow_node.can_emit_intermediate_output.set"
        }
        LocalDaemonRequest::SetWorkflowNodeIntermediateOutputSchema(_) => {
            "workflow_node.intermediate_output_schema.set"
        }
        LocalDaemonRequest::SetWorkflowNodeMaxTurns(_) => "workflow_node.max_turns.set",
        LocalDaemonRequest::AddWorkflowEdge(_) => "workflow_edge.add",
        LocalDaemonRequest::RemoveWorkflowEdge(_) => "workflow_edge.remove",
        LocalDaemonRequest::SetWorkflowRunOutputSchema(_) => "workflow.run_output_schema.set",
        LocalDaemonRequest::SetWorkflowIntermediateOutputSchema(_) => {
            "workflow.intermediate_output_schema.set"
        }
        LocalDaemonRequest::SetWorkflowFlushContext(_) => "workflow.flush_context.set",
        LocalDaemonRequest::SetWorkflowLaunchPolicy(_) => "workflow.launch_policy.set",
        LocalDaemonRequest::InvokeWorkflowEndpoint(_) => "workflow_endpoint.invoke",
        LocalDaemonRequest::ListWorkflowRuns(_) => "workflow_run.list",
        LocalDaemonRequest::GetWorkflowRun(_) => "workflow_run.get",
        LocalDaemonRequest::AckWorkflowTurn(_) => "workflow_turn.ack",
        LocalDaemonRequest::ValidateWorkflowOutput(_) => "workflow_output.validate",
        LocalDaemonRequest::CancelWorkflowRun(_) => "workflow_run.cancel",
        LocalDaemonRequest::ResumeWorkflowRun(_) => "workflow_run.resume",
        LocalDaemonRequest::ClearQueuedWorkflowLaunches(_) => "workflow_launch_queue.clear",
        LocalDaemonRequest::RemoveQueuedWorkflowLaunch(_) => "workflow_launch_queue.remove",
        LocalDaemonRequest::ListQueuedWorkflowLaunches(_) => "workflow_launch_queue.list",
        LocalDaemonRequest::CreateWorkflowWatchdog(_) => "workflow_watchdog.create",
        LocalDaemonRequest::RemoveWorkflowWatchdog(_) => "workflow_watchdog.remove",
        LocalDaemonRequest::SetWorkflowWatchdogEnabled(_) => "workflow_watchdog.enabled.set",
        LocalDaemonRequest::ListWorkflowWatchdogs(_) => "workflow_watchdog.list",
        LocalDaemonRequest::AttachToSession(_)
        | LocalDaemonRequest::DetachFromSession(_)
        | LocalDaemonRequest::SubmitPrompt(_)
        | LocalDaemonRequest::CancelActivePrompt(_)
        | LocalDaemonRequest::ResizeTerminal(_)
        | LocalDaemonRequest::FocusAgent(_)
        | LocalDaemonRequest::CycleAgentFocus(_)
        | LocalDaemonRequest::EndSession(_)
        | LocalDaemonRequest::DeleteSession(_)
        | LocalDaemonRequest::GetDaemonHealth(_)
        | LocalDaemonRequest::GetSessionHistory(_)
        | LocalDaemonRequest::GetProviderRun(_) => unreachable!("handled by metadata matcher"),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::attachment::ClientCapabilityLevel;
    use crate::local::{
        AliasSessionRequest, AttachToSessionRequest, DestroyAgentRequest, EndSessionRequest,
        FocusAgentRequest, GetDaemonHealthRequest, LocalDaemonRequest, PollRuntimeNoticesRequest,
        SpawnAgentRequest, SubmitPromptRequest, UpdateSessionConfigRequest,
    };
    use crate::runtime::command::{
        KernelCaller, KernelCallerKind, KernelCommand, KernelCommandPriority, KernelCommandSource,
    };
    use crate::session::CreateSessionRequest;
    use arroba_relay::auth::RelaySubjectKind;
    use arroba_relay::protocol::RelayCallerIdentity;

    #[test]
    fn normalizes_prompt_submit_to_interactive_kernel_command() {
        let command = KernelCommand::from_local_request(
            "cmd-1",
            Some("corr-1".to_string()),
            None,
            &LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: "session-1".to_string(),
                attachment_id: "attachment-1".to_string(),
                target_agent_id: Some("agent-1".to_string()),
                prompt: "hello".to_string(),
                attachments: Vec::new(),
            }),
        );

        assert_eq!(command.command_id, "cmd-1");
        assert_eq!(command.command_type, "prompt.submit");
        assert_eq!(command.correlation_id, "corr-1");
        assert_eq!(command.priority, KernelCommandPriority::Interactive);
        assert_eq!(command.session_id.as_deref(), Some("session-1"));
        assert_eq!(command.attachment_id.as_deref(), Some("attachment-1"));
        assert_eq!(command.agent_id.as_deref(), Some("agent-1"));
    }

    #[test]
    fn normalizes_attach_and_focus_as_interactive_commands() {
        let create = KernelCommand::from_local_request(
            "create-1",
            None,
            None,
            &LocalDaemonRequest::CreateSession(CreateSessionRequest::new("workspace", "worktree")),
        );
        let attach = KernelCommand::from_local_request(
            "attach-1",
            None,
            None,
            &LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
                session_id: "session-1".to_string(),
                client_id: "cli-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            }),
        );
        let focus = KernelCommand::from_local_request(
            "focus-1",
            None,
            None,
            &LocalDaemonRequest::FocusAgent(FocusAgentRequest {
                session_id: "session-1".to_string(),
                agent_id: "agent-2".to_string(),
            }),
        );

        assert_eq!(create.command_type, "session.create");
        assert_eq!(create.priority, KernelCommandPriority::Interactive);
        assert_eq!(create.session_id.as_deref(), None);
        assert_eq!(attach.command_type, "session.attach");
        assert_eq!(attach.priority, KernelCommandPriority::Interactive);
        assert_eq!(attach.correlation_id, "attach-1");
        assert_eq!(focus.command_type, "agent.focus");
        assert_eq!(focus.priority, KernelCommandPriority::Interactive);
        assert_eq!(focus.agent_id.as_deref(), Some("agent-2"));
    }

    #[test]
    fn normalizes_end_session_as_interactive_command() {
        let command = KernelCommand::from_local_request(
            "end-1",
            None,
            None,
            &LocalDaemonRequest::EndSession(EndSessionRequest {
                session_id: "session-1".to_string(),
            }),
        );

        assert_eq!(command.command_type, "session.end");
        assert_eq!(command.priority, KernelCommandPriority::Interactive);
        assert_eq!(command.session_id.as_deref(), Some("session-1"));
    }

    #[test]
    fn normalizes_session_runtime_commands_as_interactive_commands() {
        let notice = KernelCommand::from_local_request(
            "notice-1",
            None,
            None,
            &LocalDaemonRequest::PollRuntimeNotices(PollRuntimeNoticesRequest {
                session_id: "session-1".to_string(),
                attachment_id: "attachment-1".to_string(),
            }),
        );
        let config = KernelCommand::from_local_request(
            "config-1",
            None,
            None,
            &LocalDaemonRequest::UpdateSessionConfig(UpdateSessionConfigRequest {
                session_id: "session-1".to_string(),
                attachment_id: "attachment-1".to_string(),
                values: BTreeMap::from([("theme".to_string(), "compact".to_string())]),
                requires_idle: false,
            }),
        );
        let alias = KernelCommand::from_local_request(
            "alias-1",
            None,
            None,
            &LocalDaemonRequest::AliasSession(AliasSessionRequest {
                session_id: "session-1".to_string(),
                alias: "review".to_string(),
            }),
        );
        let spawn = KernelCommand::from_local_request(
            "spawn-1",
            None,
            None,
            &LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                session_id: "session-1".to_string(),
                alias: Some("reviewer".to_string()),
                provider: "claude-code".to_string(),
                model: None,
                effort: None,
                worktree_id: None,
                machine_ref: None,
                worktree_placement: None,
            }),
        );
        let destroy = KernelCommand::from_local_request(
            "destroy-1",
            None,
            None,
            &LocalDaemonRequest::DestroyAgent(DestroyAgentRequest {
                session_id: "session-1".to_string(),
                agent_id: "agent-2".to_string(),
            }),
        );

        assert_eq!(notice.command_type, "runtime_notice.poll");
        assert_eq!(notice.priority, KernelCommandPriority::Interactive);
        assert_eq!(notice.session_id.as_deref(), Some("session-1"));
        assert_eq!(notice.attachment_id.as_deref(), Some("attachment-1"));
        assert_eq!(config.command_type, "session.config.update");
        assert_eq!(config.priority, KernelCommandPriority::Interactive);
        assert_eq!(config.session_id.as_deref(), Some("session-1"));
        assert_eq!(config.attachment_id.as_deref(), Some("attachment-1"));
        assert_eq!(alias.command_type, "session.alias");
        assert_eq!(alias.priority, KernelCommandPriority::Interactive);
        assert_eq!(alias.session_id.as_deref(), Some("session-1"));
        assert_eq!(spawn.command_type, "agent.spawn");
        assert_eq!(spawn.priority, KernelCommandPriority::Interactive);
        assert_eq!(spawn.session_id.as_deref(), Some("session-1"));
        assert_eq!(destroy.command_type, "agent.destroy");
        assert_eq!(destroy.priority, KernelCommandPriority::Interactive);
        assert_eq!(destroy.session_id.as_deref(), Some("session-1"));
        assert_eq!(destroy.agent_id.as_deref(), Some("agent-2"));
    }

    #[test]
    fn normalizes_daemon_health_as_normal_command() {
        let command = KernelCommand::from_local_request(
            "health-1",
            None,
            None,
            &LocalDaemonRequest::GetDaemonHealth(GetDaemonHealthRequest),
        );

        assert_eq!(command.command_type, "daemon.health.get");
        assert_eq!(command.priority, KernelCommandPriority::Normal);
    }

    #[test]
    fn can_normalize_local_ipc_commands_with_ipc_source() {
        let request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: "session-1".to_string(),
            attachment_id: "attachment-1".to_string(),
            target_agent_id: Some("agent-1".to_string()),
            prompt: "hello".to_string(),
            attachments: Vec::new(),
        });
        let command = KernelCommand::from_local_request_with_source(
            "ipc-1",
            KernelCommandSource::LocalIpc,
            None,
            None,
            &request,
        );

        assert_eq!(command.source, KernelCommandSource::LocalIpc);
        assert_eq!(command.caller.caller_kind, KernelCallerKind::LocalClient);
        assert_eq!(command.caller.caller_id, "local-ipc");
        assert_eq!(command.command_type, "prompt.submit");
        assert_eq!(command.priority, KernelCommandPriority::Interactive);
        assert_eq!(command.correlation_id, "ipc-1");
    }

    #[test]
    fn relay_identity_becomes_kernel_command_caller() {
        let request = LocalDaemonRequest::GetDaemonHealth(GetDaemonHealthRequest);
        let command = KernelCommand::from_local_request_with_caller(
            "relay-1",
            KernelCommandSource::RelayClient,
            KernelCaller::from_relay_identity(RelayCallerIdentity {
                realm_id: "realm-1".to_string(),
                subject: "client-1".to_string(),
                subject_kind: RelaySubjectKind::Client,
                token_id: Some("token-1".to_string()),
                user_id: Some("user-1".to_string()),
                public_key_thumbprint: Some("thumbprint-1".to_string()),
            }),
            None,
            None,
            &request,
        );

        assert_eq!(command.source, KernelCommandSource::RelayClient);
        assert_eq!(command.caller.caller_kind, KernelCallerKind::RemoteClient);
        assert_eq!(command.caller.caller_id, "client-1");
        assert_eq!(command.caller.user_id.as_deref(), Some("user-1"));
        assert_eq!(command.caller.client_id.as_deref(), Some("client-1"));
        assert_eq!(command.caller.realm_id.as_deref(), Some("realm-1"));
        assert_eq!(
            command.caller.public_key_thumbprint.as_deref(),
            Some("thumbprint-1")
        );
    }
}
