use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionStatus {
    Created,
    Active,
    Parked,
    Ended,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionExecutionMode {
    SingleAgent,
    MultiAgentWorkflow,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SchedulerState {
    Idle,
    Runnable,
    Running,
    Waiting,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KernelRestartReconciliation {
    pub cleared_active_provider_run: bool,
    pub cleared_attachment_count: usize,
    pub recoverable_prompt_count: usize,
    pub recoverable_workflow_run_count: usize,
    pub repaired_workflow_prompt_count: usize,
    pub interrupted_prompt_count: usize,
    pub stopped_workflow_run_count: usize,
}

impl KernelRestartReconciliation {
    pub fn changed(&self) -> bool {
        self.cleared_active_provider_run
            || self.cleared_attachment_count > 0
            || self.interrupted_prompt_count > 0
            || self.repaired_workflow_prompt_count > 0
            || self.stopped_workflow_run_count > 0
    }
}

impl fmt::Display for SessionExecutionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::SingleAgent => "single_agent",
            Self::MultiAgentWorkflow => "multi_agent_workflow",
        };

        write!(f, "{value}")
    }
}

impl fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Created => "created",
            Self::Active => "active",
            Self::Parked => "parked",
            Self::Ended => "ended",
        };

        write!(f, "{value}")
    }
}
