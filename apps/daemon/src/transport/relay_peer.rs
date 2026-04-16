use serde::{Deserialize, Serialize};

use crate::execution_lease::{ExecutionLease, LeasedAgent, RemoteWorkflowTurnContext};
use crate::io::WorkspaceIdentity;
use crate::session::{PromptCancellation, PromptCompletion, PromptSubmissionOutcome};
use crate::terminal::TerminalOutputKind;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayPromptAttachment {
    pub url: String,
    pub mime: String,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contents_base64: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteManagedIoContext {
    pub home_kernel_id: String,
    pub home_session_id: String,
    pub home_agent_id: String,
    pub leased_agent_id: String,
    pub worker_provider_run_id: String,
    pub worker_workspace_identity: WorkspaceIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteManagedIoArtifactState {
    pub path: String,
    pub exists: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RelayPeerRequest {
    Ping {
        value: String,
    },
    CreateExecutionLease {
        home_kernel_id: String,
        home_session_id: String,
        home_agent_id: String,
    },
    DestroyExecutionLease {
        lease_id: String,
    },
    SpawnLeasedAgent {
        lease_id: String,
        provider: String,
        model: Option<String>,
        effort: Option<String>,
    },
    DestroyLeasedAgent {
        leased_agent_id: String,
    },
    SubmitLeasedPrompt {
        leased_agent_id: String,
        prompt: String,
        #[serde(default)]
        attachments: Vec<RelayPromptAttachment>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workflow_context: Option<RemoteWorkflowTurnContext>,
    },
    CompleteLeasedPrompt {
        leased_agent_id: String,
    },
    CancelLeasedPrompt {
        leased_agent_id: String,
    },
    ForwardWorkflowRuntimeTool {
        context: RemoteWorkflowTurnContext,
        tool_name: String,
        arguments: serde_json::Value,
    },
    ForwardManagedIoRuntimeTool {
        context: RemoteManagedIoContext,
        tool_name: String,
        arguments: serde_json::Value,
        artifact_states: Vec<RemoteManagedIoArtifactState>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RelayPeerResponse {
    Pong {
        value: String,
        daemon_id: String,
    },
    ExecutionLeaseCreated {
        lease: ExecutionLease,
    },
    ExecutionLeaseDestroyed {
        lease_id: String,
    },
    LeasedAgentSpawned {
        leased_agent: LeasedAgent,
    },
    LeasedAgentDestroyed {
        leased_agent_id: String,
    },
    LeasedPromptSubmitted {
        provider_run_id: String,
        outcome: PromptSubmissionOutcome,
    },
    LeasedPromptCompleted {
        provider_run_id: Option<String>,
        completion: PromptCompletion,
    },
    LeasedPromptCancelled {
        cancellation: PromptCancellation,
    },
    WorkflowRuntimeToolHandled {
        result: crate::transport::runtime_tools::RuntimeToolResult,
    },
    ManagedIoRuntimeToolHandled {
        result: crate::transport::runtime_tools::RuntimeToolResult,
        final_artifact_states: Vec<RemoteManagedIoArtifactState>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayProjectedOutputChunk {
    pub kind: TerminalOutputKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_key: Option<String>,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayProjectedCompletion {
    pub message_id: String,
    pub completed_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RelayPeerEvent {
    LeasedRuntimeProjection {
        home_session_id: String,
        home_agent_id: String,
        provider_run_id: String,
        output_chunks: Vec<RelayProjectedOutputChunk>,
        notices: Vec<String>,
        completions: Vec<RelayProjectedCompletion>,
    },
}
