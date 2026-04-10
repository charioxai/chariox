use serde::{Deserialize, Serialize};

use crate::session::unix_epoch_ms;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionLease {
    pub id: String,
    pub home_kernel_id: String,
    pub home_session_id: String,
    pub home_agent_id: String,
    pub worker_kernel_id: String,
    pub machine_id: String,
    pub created_at_ms: u64,
    pub last_heartbeat_at_ms: u64,
}

impl ExecutionLease {
    pub fn new(
        id: String,
        home_kernel_id: String,
        home_session_id: String,
        home_agent_id: String,
        worker_kernel_id: String,
        machine_id: String,
    ) -> Self {
        let now = unix_epoch_ms();
        Self {
            id,
            home_kernel_id,
            home_session_id,
            home_agent_id,
            worker_kernel_id,
            machine_id,
            created_at_ms: now,
            last_heartbeat_at_ms: now,
        }
    }
}
