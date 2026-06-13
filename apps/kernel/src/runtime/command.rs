use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::local::LocalDaemonRequest;
use crate::session::unix_epoch_ms;

mod caller;
mod local_request_metadata;

pub(crate) use caller::command_caller_user_id;
pub use caller::{KernelCaller, KernelCallerKind, KernelCommandSource};

use local_request_metadata::local_request_metadata;

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
        let payload = local_request_payload(request);
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

fn local_request_payload(request: &LocalDaemonRequest) -> Value {
    match request {
        LocalDaemonRequest::SetCredentialSecret(request) => serde_json::json!({
            "SetCredentialSecret": {
                "session_id": request.session_id,
                "agent_id": request.agent_id,
                "key": request.key,
                "value": "[redacted]"
            }
        }),
        _ => serde_json::to_value(request).unwrap_or(Value::Null),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::attachment::ClientCapabilityLevel;
    use crate::local::{
        AliasSessionRequest, AttachToSessionRequest, DestroyAgentRequest, EndSessionRequest,
        FocusAgentRequest, GetDaemonHealthRequest, LocalDaemonRequest, PollRuntimeNoticesRequest,
        SetCredentialSecretRequest, SpawnAgentRequest, SubmitPromptRequest,
        UpdateSessionConfigRequest,
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
                provider: Some("claude-code".to_string()),
                model: None,
                effort: None,
                execution_mode: None,
                permission_level: None,
                worktree_id: None,
                kernel_ref: None,
                slice_ref: None,
                worktree_placement: None,
                metaagent: false,
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
    fn redacts_credential_secret_payloads() {
        let command = KernelCommand::from_local_request(
            "credential-1",
            None,
            None,
            &LocalDaemonRequest::SetCredentialSecret(SetCredentialSecretRequest {
                session_id: None,
                agent_id: None,
                key: "github-token".to_string(),
                value: "super-secret".to_string(),
            }),
        );

        assert_eq!(command.command_type, "credential.secret.set");
        assert_eq!(
            command.payload["SetCredentialSecret"]["key"],
            "github-token"
        );
        assert_eq!(
            command.payload["SetCredentialSecret"]["value"],
            "[redacted]"
        );
        assert!(!serde_json::to_string(&command.payload)
            .unwrap()
            .contains("super-secret"));
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
                expires_at_ms: 20,
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
