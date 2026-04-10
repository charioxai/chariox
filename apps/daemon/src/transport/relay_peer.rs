use serde::{Deserialize, Serialize};

use crate::execution_lease::ExecutionLease;

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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RelayPeerResponse {
    Pong { value: String, daemon_id: String },
    ExecutionLeaseCreated { lease: ExecutionLease },
    ExecutionLeaseDestroyed { lease_id: String },
}
