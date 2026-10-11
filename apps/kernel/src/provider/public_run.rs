// MP-11 F7: an allowlist independent of the private launch/persistence contract.
use super::{
    AgentEndpointMode, AgentExecutionMode, AgentPermissionLevel, ControlCapability,
    ExternalProviderImportMetadata, ProviderClientInterface, ProviderRunState,
    ProviderRunTokenUsage, RuntimeProviderRun,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Public client projection. Never contains provider environment, arguments,
/// MCP credentials/configuration, resume payloads or free-form diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicProviderRun {
    id: String,
    session_id: String,
    agent_instance_id: Option<String>,
    owner_user_id: String,
    adapter_key: String,
    provider: String,
    account_profile: String,
    model: String,
    variant: Option<String>,
    usage_tokens_total: Option<u64>,
    #[serde(default, skip_serializing_if = "ProviderRunTokenUsage::is_empty")]
    usage: ProviderRunTokenUsage,
    state: ProviderRunState,
    endpoint_mode: AgentEndpointMode,
    client_interface: ProviderClientInterface,
    working_directory: Option<PathBuf>,
    structured_endpoint: Option<String>,
    provider_session_id: Option<String>,
    control_capabilities: Vec<ControlCapability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    external_provider_import: Option<ExternalProviderImportMetadata>,
    execution_mode: AgentExecutionMode,
    permission_level: AgentPermissionLevel,
    started_at_ms: u64,
    last_activity_at_ms: u64,
}

impl From<&RuntimeProviderRun> for PublicProviderRun {
    fn from(run: &RuntimeProviderRun) -> Self {
        Self {
            id: run.id().to_owned(),
            session_id: run.session_id().to_owned(),
            agent_instance_id: run.agent_instance_id().map(str::to_owned),
            owner_user_id: run.owner_user_id().to_owned(),
            adapter_key: run.adapter_key().to_owned(),
            provider: run.provider().to_owned(),
            account_profile: run.account_profile().to_owned(),
            model: run.model().to_owned(),
            variant: run.variant().map(str::to_owned),
            usage_tokens_total: run.usage_tokens_total(),
            usage: run.usage(),
            state: run.state(),
            endpoint_mode: run.endpoint_mode(),
            client_interface: run.client_interface(),
            working_directory: run.working_directory().cloned(),
            structured_endpoint: run
                .structured_endpoint()
                .filter(|value| public_endpoint(value))
                .map(str::to_owned),
            provider_session_id: run
                .provider_session_id()
                .or_else(|| run.resume_state().provider_session_id(run.adapter_key()))
                .map(str::to_owned),
            control_capabilities: run.control_capabilities().to_vec(),
            external_provider_import: run.external_provider_import().cloned(),
            execution_mode: run.execution_mode(),
            permission_level: run.permission_level(),
            started_at_ms: run.started_at_ms(),
            last_activity_at_ms: run.last_activity_at_ms(),
        }
    }
}

impl From<RuntimeProviderRun> for PublicProviderRun {
    fn from(run: RuntimeProviderRun) -> Self {
        Self::from(&run)
    }
}

impl PublicProviderRun {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
    pub fn agent_instance_id(&self) -> Option<&str> {
        self.agent_instance_id.as_deref()
    }
    pub fn owner_user_id(&self) -> &str {
        &self.owner_user_id
    }
    pub fn adapter_key(&self) -> &str {
        &self.adapter_key
    }
    pub fn provider(&self) -> &str {
        &self.provider
    }
    pub fn account_profile(&self) -> &str {
        &self.account_profile
    }
    pub fn model(&self) -> &str {
        &self.model
    }
    pub fn variant(&self) -> Option<&str> {
        self.variant.as_deref()
    }
    pub fn usage_tokens_total(&self) -> Option<u64> {
        self.usage_tokens_total
    }
    pub fn usage(&self) -> ProviderRunTokenUsage {
        self.usage
    }
    pub fn state(&self) -> ProviderRunState {
        self.state
    }
    pub fn endpoint_mode(&self) -> AgentEndpointMode {
        self.endpoint_mode
    }
    pub fn client_interface(&self) -> ProviderClientInterface {
        self.client_interface
    }
    pub fn working_directory(&self) -> Option<&PathBuf> {
        self.working_directory.as_ref()
    }
    pub fn structured_endpoint(&self) -> Option<&str> {
        self.structured_endpoint.as_deref()
    }
    pub fn provider_session_id(&self) -> Option<&str> {
        self.provider_session_id.as_deref()
    }
    pub fn control_capabilities(&self) -> &[ControlCapability] {
        &self.control_capabilities
    }
    pub fn external_provider_import(&self) -> Option<&ExternalProviderImportMetadata> {
        self.external_provider_import.as_ref()
    }
    pub fn execution_mode(&self) -> AgentExecutionMode {
        self.execution_mode
    }
    pub fn permission_level(&self) -> AgentPermissionLevel {
        self.permission_level
    }
    pub fn started_at_ms(&self) -> u64 {
        self.started_at_ms
    }
    pub fn last_activity_at_ms(&self) -> u64 {
        self.last_activity_at_ms
    }
    pub fn owned_by(&self, user_id: &str) -> bool {
        self.owner_user_id == user_id
    }
}

fn public_endpoint(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|url| {
        matches!(url.scheme(), "http" | "https" | "ws" | "wss")
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn private_run() -> RuntimeProviderRun {
        let run = RuntimeProviderRun::from_control_capability_inference(
            "run-public",
            "session-public".into(),
            Some("agent-public".into()),
            "codex".into(),
        );
        let mut value = serde_json::to_value(run).unwrap();
        for name in ["pty_env", "provider_config_overrides"] {
            value[name] = serde_json::json!({"FIXTURE_VALUE": "mp11-private-sentinel"});
        }
        for name in [
            "runtime_mcp_auth_token",
            "runtime_mcp_server_url",
            "terminal_diagnostic",
        ] {
            value[name] = serde_json::json!("mp11-private-sentinel");
        }
        value["pty_args"] = serde_json::json!(["mp11-private-sentinel"]);
        value["structured_endpoint"] = serde_json::json!("http://127.0.0.1:1234");
        value["started_at_ms"] = serde_json::json!(1);
        value["last_activity_at_ms"] = serde_json::json!(2);
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn mp11_f7_public_provider_run_protocol_435_snapshot() {
        assert_eq!(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, 490);
        let run = private_run();
        let dto = PublicProviderRun::from(&run);
        let value = serde_json::to_value(&dto).unwrap();
        let expected = serde_json::json!({
            "id":"run-public", "session_id":"session-public", "agent_instance_id":"agent-public",
            "owner_user_id":"local", "adapter_key":"codex", "provider":"codex",
            "account_profile":"default", "model":"", "variant":null,
            "usage_tokens_total":null, "state":"Starting", "endpoint_mode":"Managed",
            "client_interface":"chariox", "working_directory":null,
            "structured_endpoint":"http://127.0.0.1:1234", "provider_session_id":null,
            "execution_mode":"build", "permission_level":"yolo",
            "control_capabilities": [
                {"operation":"interrupt_turn", "mode":"native"},
                {"operation":"cancel_prompt", "mode":"native"},
                {"operation":"ack_workflow_turn", "mode":"mcp"},
                {"operation":"validate_workflow_handoff", "mode":"mcp"}
            ],
            "started_at_ms":1, "last_activity_at_ms":2
        });
        // Compare the complete key set; the hash pins the actual protocol shape.
        assert_eq!(value, expected);
        assert!(!serde_json::to_string(&value)
            .unwrap()
            .contains("mp11-private-sentinel"));
        assert_eq!(
            serde_json::from_value::<PublicProviderRun>(value.clone()).unwrap(),
            dto
        );
        let hash = format!("{:x}", Sha256::digest(serde_json::to_vec(&value).unwrap()));
        assert_eq!(
            hash,
            "2f41df5dd906f355393e51deb9cec3546118423730cc67eb445c34cae2f3487c"
        );
        // Private persistence and peer launch serialization keep their credentials.
        assert!(serde_json::to_string(&run)
            .unwrap()
            .contains("mp11-private-sentinel"));
        assert_eq!(
            serde_json::from_value::<RuntimeProviderRun>(serde_json::to_value(&run).unwrap())
                .unwrap(),
            run
        );
    }

    #[test]
    fn mp11_f7_public_run_responses_and_events_never_emit_launch_secrets() {
        use crate::local::{LocalDaemonResponse, ProviderRunBatchLaunchResult};
        let dto = PublicProviderRun::from(private_run());
        let responses = [
            LocalDaemonResponse::ProviderRun {
                provider_run: dto.clone(),
            },
            LocalDaemonResponse::ProviderRunLaunched {
                provider_run: dto.clone(),
            },
            LocalDaemonResponse::ProviderRunLaunchAccepted {
                provider_run: dto.clone(),
            },
            LocalDaemonResponse::ProviderRunSelectionUpdated {
                provider_run: dto.clone(),
            },
            LocalDaemonResponse::ProviderRunsLaunchAccepted {
                provider_runs: vec![ProviderRunBatchLaunchResult {
                    index: 0,
                    agent_id: Some("agent-public".into()),
                    provider_run: dto.clone(),
                    reused: false,
                }],
                failures: vec![],
            },
        ];
        for response in responses {
            let value = serde_json::to_value(&response).unwrap();
            assert!(!serde_json::to_string(&value)
                .unwrap()
                .contains("mp11-private-sentinel"));
            assert_eq!(
                serde_json::from_value::<LocalDaemonResponse>(value).unwrap(),
                response
            );
        }
        let event = crate::transport::kernel_protocol::KernelEvent::ProviderRunChanged {
            session_id: "session-public".into(),
            provider_run: Some(dto),
        };
        let value = serde_json::to_value(&event).unwrap();
        assert!(!serde_json::to_string(&value)
            .unwrap()
            .contains("mp11-private-sentinel"));
        assert!(
            serde_json::from_value::<crate::transport::kernel_protocol::KernelEvent>(value).is_ok()
        );
    }

    #[test]
    fn mp11_f7_native_endpoint_rejects_embedded_credentials() {
        for endpoint in [
            "http://user:mp11-private-sentinel@localhost",
            "http://localhost?token=mp11-private-sentinel",
            "http://localhost#mp11-private-sentinel",
            "invalid",
        ] {
            let mut value = serde_json::to_value(private_run()).unwrap();
            value["structured_endpoint"] = serde_json::json!(endpoint);
            let run: RuntimeProviderRun = serde_json::from_value(value).unwrap();
            assert!(PublicProviderRun::from(run).structured_endpoint().is_none());
        }
        assert_eq!(
            PublicProviderRun::from(private_run()).structured_endpoint(),
            Some("http://127.0.0.1:1234")
        );
    }
}
