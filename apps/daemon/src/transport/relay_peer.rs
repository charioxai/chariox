use serde::{Deserialize, Serialize};

use crate::execution_lease::{ExecutionLease, LeasedAgent};
use crate::session::{PromptCancellation, PromptCompletion, PromptSubmissionOutcome};
use crate::terminal::TerminalOutputKind;

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
    },
    CompleteLeasedPrompt {
        leased_agent_id: String,
    },
    CancelLeasedPrompt {
        leased_agent_id: String,
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
