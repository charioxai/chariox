//! MP-08 / MP-09 / MP-10 / MP-11 A02: shared task projection and writer commands.
use super::DaemonError;
use serde::{Deserialize, Serialize};
use std::sync::mpsc;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionState {
    Working,
    Waiting,
    Blocked,
    Done,
    Cancelled,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTaskExecution {
    pub task_id: String,
    pub room_id: String,
    #[serde(default)]
    pub owner_user_id: String,
    pub agent_id: String,
    pub prompt_id: String,
    pub provider_run_id: Option<String>,
    pub revision: u64,
    #[serde(default)]
    pub blocked_revision: u64,
    pub state: ExecutionState,
    pub reason: String,
    pub obligations: Vec<AgentObligation>,
    pub wait: Option<AgentWait>,
    pub last_progress_at_ms: u64,
    pub progress_sequence: u64,
    pub no_progress_wakes: u32,
    pub correction_used: bool,
    pub pending_prompt_id: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentObligation {
    pub id: String,
    pub kind: String,
    pub resource_id: Option<String>,
    #[serde(default)]
    pub completion_task_id: Option<String>,
    pub status: String,
    pub dispatch_state: String,
}
impl AgentObligation {
    pub(crate) fn tracks_workflow_run(&self) -> bool {
        matches!(self.kind.as_str(), "workflow" | "workflow_run")
    }
    pub(crate) fn completion_source(&self) -> Option<&str> {
        self.completion_task_id
            .as_deref()
            .or(self.resource_id.as_deref())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentWait {
    pub registration_ids: Vec<String>,
    pub deadline_ms: u64,
    pub started_at_ms: u64,
    pub inbox_cursor: u64,
    pub long_wait_notified: bool,
    #[serde(default)]
    pub last_checked_at_ms: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Registration {
    pub id: String,
    pub task_id: String,
    pub source_id: String,
    pub obligation_id: Option<String>,
    #[serde(default)]
    pub source_cursor: u64,
    pub live: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct InboxEvent {
    pub sequence: u64,
    pub room_id: String,
    pub agent_id: String,
    pub source_id: String,
    pub occurrence_id: String,
    pub kind: String,
    pub payload: serde_json::Value,
    pub urgent: bool,
    pub reply_requested: bool,
    pub state: String,
    pub prompt_id: Option<String>,
    pub target_prompt_id: Option<String>,
    pub provider_run_id: Option<String>,
    pub attempted_at_ms: Option<u64>,
    pub submit_epoch: Option<u64>,
}
pub(crate) enum Operation {
    Begin {
        owner: String,
        room: String,
        agent: String,
        prompt: String,
        run: Option<String>,
        now: u64,
    },
    BindDelegate {
        parent_task: String,
        child_task: String,
    },
    RegisterObligation {
        owner: String,
        room: String,
        agent: String,
        prompt: String,
        run: Option<String>,
        id: String,
        kind: String,
        resource: Option<String>,
        now: u64,
    },
    DispatchReceipt {
        id: String,
        accepted: bool,
        resource: Option<String>,
    },
    Subscribe {
        task: String,
        prompt: String,
        registration: Registration,
    },
    Unsubscribe {
        task: String,
        prompt: String,
        registration: String,
    },
    Yield {
        task: String,
        prompt: String,
        registrations: Vec<String>,
        cursor: u64,
        deadline: u64,
        reason: String,
        now: u64,
    },
    Block {
        task: String,
        prompt: String,
        reason: String,
    },
    Settle {
        room: String,
        agent: String,
        prompt: String,
        run: String,
        has_answer: bool,
        cancelled: bool,
        now: u64,
    },
    Occur(InboxEvent),
    Send {
        task: String,
        prompt: String,
        event: InboxEvent,
    },
    Attempt {
        room: String,
        agent: String,
        sequence: u64,
        prompt: String,
        target: Option<String>,
        run: Option<String>,
        now: u64,
    },
    Expire {
        room: String,
        agent: String,
        sequence: u64,
    },
    Defer {
        room: String,
        agent: String,
        sequence: u64,
        now: u64,
    },
    // Internal writer command, not a serialized client protocol shape.
    BindSubmission {
        room: String,
        agent: String,
        sequence: u64,
        prompt: String,
        target: Option<String>,
        run: String,
        submit_epoch: u64,
        now: u64,
    },
    Receipt {
        room: String,
        agent: String,
        sequence: u64,
        state: String,
        now: u64,
    },
    Ack {
        room: String,
        agent: String,
        sequence: u64,
        handled: bool,
        now: u64,
    },
    Progress {
        task: String,
        prompt: String,
        receipt: String,
        now: u64,
    },
    SourceOutcome {
        public_answer: Option<serde_json::Value>,
        room: String,
        source: String,
        occurrence: String,
        success: bool,
        now: u64,
    },
    Sweep {
        now: u64,
        busy_recipients: Vec<(String, String)>,
    },
    CancelTask {
        task: String,
        owner: String,
        revision: u64,
    },
    /// Rolls back an admission whose prompt submission was rejected.
    Withdraw {
        task: String,
    },
    OwnerResponse {
        task: String,
        revision: u64,
        resume: bool,
        now: u64,
    },
}
#[derive(Debug)]
pub(crate) enum Outcome {
    Task(AgentTaskExecution),
    Event(InboxEvent),
    Settled {
        task: AgentTaskExecution,
        correction: bool,
    },
    Swept(Vec<AgentTaskExecution>),
    Saved,
}
pub(crate) struct Request {
    pub(super) operation: Operation,
    pub(super) response: mpsc::Sender<Result<Outcome, DaemonError>>,
}
impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentLifecycleRequest")
            .finish_non_exhaustive()
    }
}
