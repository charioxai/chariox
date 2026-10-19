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
impl AgentTaskExecution {
    /// A task with no open obligation and no wait is an ordinary turn: its end
    /// is its whole outcome, so nothing needs the owner to decide.
    pub(crate) fn is_plain_turn(&self) -> bool {
        self.wait.is_none()
            && !self
                .obligations
                .iter()
                .any(|o| matches!(o.status.as_str(), "open" | "settling" | "failed"))
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentObligation {
    pub id: String,
    pub kind: String,
    pub resource_id: Option<String>,
    pub status: String,
    pub dispatch_state: String,
}
impl AgentObligation {
    pub(crate) fn completion_source(&self) -> Option<&str> {
        self.resource_id.as_deref()
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
    pub state: String,
    pub prompt_id: Option<String>,
    pub target_prompt_id: Option<String>,
    pub provider_run_id: Option<String>,
    pub attempted_at_ms: Option<u64>,
    pub submit_epoch: Option<u64>,
}
/// MP-08 / MP-09 / MP-10 / MP-11 A03: one kernel-owned wake source. It is a
/// task obligation plus a live registration; its receipts record when the last
/// occurrence fired, was delivered and acknowledged. The ledger does not know
/// what a `kind` does: its driver renders `summary` and owns `detail`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentWake {
    pub id: String,
    pub task_id: String,
    pub room_id: String,
    pub agent_id: String,
    pub registration_id: String,
    /// Names the driver that owns the source, e.g. `timer`.
    pub kind: String,
    pub label: String,
    /// `scheduled`, `starting`, `running`, `cancelling`, `fired`, `exited`, `lost` or `cancelled`.
    pub state: String,
    pub created_at_ms: u64,
    /// Proof of life: whoever armed the source confirmed it is live.
    pub verified_at_ms: Option<u64>,
    pub next_due_ms: Option<u64>,
    pub interval_ms: Option<u64>,
    /// One redacted line for clients, rendered by the driver.
    #[serde(default)]
    pub summary: String,
    /// Driver-owned, redacted before storage; the ledger never interprets it.
    #[serde(default)]
    pub detail: serde_json::Value,
    pub fire_count: u64,
    pub missed_fires: u64,
    pub last_fired_at_ms: Option<u64>,
    pub last_sequence: Option<u64>,
    pub last_delivery: Option<String>,
    pub last_delivered_at_ms: Option<u64>,
    pub last_acknowledged_at_ms: Option<u64>,
    pub alerted_sequence: Option<u64>,
}
/// How a driver reports the end of its source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WakeEnd {
    Succeeded,
    #[allow(dead_code, reason = "reported by the terminal driver")]
    Failed,
    /// The source can no longer be observed; it is never relaunched.
    Lost,
}
#[derive(Clone)]
pub(crate) enum Operation {
    Begin {
        owner: String,
        room: String,
        agent: String,
        prompt: String,
        run: Option<String>,
        now: u64,
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
        cancelled: bool,
        now: u64,
    },
    /// Test-only injection of an arbitrary inbox occurrence.
    #[cfg(test)]
    Occur(InboxEvent),
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
    OwnerResponse {
        task: String,
        revision: u64,
        resume: bool,
        now: u64,
    },
    /// Obligation, live registration and wake record in one transaction. The
    /// driver sets the first state: `scheduled` (time-driven) or `starting`
    /// (confirmed by a driver receipt).
    ArmWake {
        task: String,
        prompt: String,
        wake: Box<AgentWake>,
    },
    /// Timer driver: proof of life for every scheduled timer.
    VerifyWakes {
        now: u64,
    },
    /// Timer driver: fires every due timer.
    FireWakes {
        now: u64,
    },
    /// A driver confirms its `starting` source is live.
    WakeVerified {
        id: String,
        now: u64,
    },
    /// A driver reports an occurrence of its source, de-duplicated on
    /// (wake, occurrence). `end` also settles the wake's obligation.
    WakeOccurred {
        id: String,
        occurrence: String,
        kind: String,
        payload: serde_json::Value,
        /// The driver's one-line rendering of the source after this occurrence.
        summary: Option<String>,
        end: Option<WakeEnd>,
        now: u64,
    },
    /// `prompt: Some` is the owning agent's turn; `None` settles a cancelled
    /// task. `pending` means the driver must still stop the source physically
    /// and will report the end.
    CancelWake {
        id: String,
        task: String,
        prompt: Option<String>,
        pending: bool,
    },
    WakeAlerted {
        id: String,
        sequence: u64,
    },
    /// Kernel teardown of a Room (`agent: None`) or agent, without owner
    /// authority: cancels its unfinished tasks, settles its scheduled wakes
    /// (other armed wakes wait for their driver) and expires its unresolved
    /// deliveries.
    RetireWakes {
        room: String,
        agent: Option<String>,
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
    Wakes(Vec<AgentWake>),
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

// An expired automatic wake may still carry an unresolved resource outcome.
// Explicit ordinary handling can settle that exact source without replaying delivery.
pub(crate) fn unresolved_source_outcome(task: &AgentTaskExecution, event: &InboxEvent) -> bool {
    task.room_id == event.room_id
        && task.agent_id == event.agent_id
        && !matches!(task.state, ExecutionState::Done | ExecutionState::Cancelled)
        && matches!(event.kind.as_str(), "source_completed" | "source_lost")
        && task.obligations.iter().any(|obligation| {
            obligation.completion_source() == Some(event.source_id.as_str())
                && matches!(obligation.status.as_str(), "settling" | "failed")
        })
}
