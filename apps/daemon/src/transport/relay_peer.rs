use serde::{Deserialize, Serialize};

use crate::execution_lease::{ExecutionLease, LeasedAgent};
use crate::session::{PromptCompletion, PromptSubmissionOutcome};

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
        completion: PromptCompletion,
    },
}
