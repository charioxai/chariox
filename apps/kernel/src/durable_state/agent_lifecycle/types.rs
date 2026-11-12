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
#[serde(rename_all = "snake_case")]
pub enum TaskOwnerAction {
    BrowserAccess,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTaskExecution {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<super::goals::AgentGoal>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_action: Option<TaskOwnerAction>,
    pub obligations: Vec<AgentObligation>,
    pub wait: Option<AgentWait>,
    pub last_progress_at_ms: u64,
    pub correction_used: bool,
    pub pending_prompt_id: Option<String>,
}
impl AgentTaskExecution {
    /// Who began stopping this turn, as recorded when the cancellation began. A path that only
    /// settles a cancellation consumes this; it never makes a new owner Stop.
    pub(crate) fn recorded_cancel_initiator(&self) -> CancelInitiator {
        if self.reason == super::supervision::AGENT_CANCELLED_REASON {
            CancelInitiator::Agent
        } else {
            CancelInitiator::Owner
        }
    }
    /// The owner stopped this turn; the agent stays stopped until prompted again.
    pub(crate) fn stopped_by_owner(&self) -> bool {
        self.state == ExecutionState::Cancelled
            && self.reason == super::supervision::OWNER_CANCELLED_REASON
    }
}
/// Who stops a running turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CancelInitiator {
    Owner,
    Agent,
}
impl CancelInitiator {
    pub(crate) fn cancel_task(self, task: String, owner: String, revision: u64) -> Operation {
        match self {
            Self::Owner => Operation::CancelTask {
                task,
                owner,
                revision,
            },
            Self::Agent => Operation::CancelTaskByAgent {
                task,
                owner,
                revision,
            },
        }
    }
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
    pub(crate) fn requires_resolution(&self) -> bool {
        match self.status.as_str() {
            "open" | "settling" => true,
            // A rejected dispatch has no admitted work to reconcile.
            "failed" => self.dispatch_state != "rejected",
            _ => false,
        }
    }

    /// Work still running: a wait has to name a live source for it. Once an obligation has an
    /// outcome (reply arrived, cancelled, child deleted) that outcome reaches the agent by itself.
    pub(crate) fn needs_live_source(&self) -> bool {
        self.status == "open"
    }

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
    /// Only a goal wait carries a deadline; any other wait ends with its sources.
    pub deadline_ms: Option<u64>,
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
    /// The source is an agent that is neither above nor below the subscriber: its end
    /// arrives as `peer_event`, never as an obligation receipt.
    #[serde(default)]
    pub peer: bool,
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
impl InboxEvent {
    /// The outcome of a source the recipient waited on: the receipt of a source or a child's `child_finished`.
    pub(crate) fn is_source_outcome(&self) -> bool {
        matches!(
            self.kind.as_str(),
            "source_completed" | "source_lost" | "child_finished" | "peer_event"
        )
    }
    /// The outcome is about this obligation source (a child's task id also names its `child_finished`).
    pub(crate) fn reports_source(&self, source: &str) -> bool {
        self.source_id == source
            || (self.kind == "child_finished"
                && self.payload["child_task_id"].as_str() == Some(source))
    }
    /// The source did not finish its work.
    pub(crate) fn is_loss(&self) -> bool {
        self.kind == "source_lost"
            || (self.kind == "peer_event" && self.payload["success"].as_bool() == Some(false))
            || (self.kind == "child_finished" && self.payload["success"].as_bool() == Some(false))
    }
}
/// MP-08 / MP-09 / MP-10 / MP-11 A03: one kernel-owned wake source: the
/// kernel timer, a watched process, or a source owned by a wake driver. It is
/// a task obligation plus a live registration; its receipts record when the
/// last occurrence fired, was delivered and acknowledged. The ledger does not
/// know what a driver `kind` does: its driver renders `summary` and owns
/// `detail`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentWake {
    pub id: String,
    pub task_id: String,
    pub room_id: String,
    pub agent_id: String,
    pub registration_id: String,
    /// `timer`, `process`, or the kind of the driver that owns the source.
    pub kind: String,
    pub label: String,
    /// `scheduled`, `starting`, `running`, `cancelling`, `fired`, `exited`, `lost` or `cancelled`.
    pub state: String,
    pub created_at_ms: u64,
    /// Proof of life: the scheduler (timer), supervisor (process) or driver
    /// confirmed the source is live.
    pub verified_at_ms: Option<u64>,
    pub next_due_ms: Option<u64>,
    pub interval_ms: Option<u64>,
    pub command: Vec<String>,
    pub match_text: Option<String>,
    pub matched_at_ms: Option<u64>,
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
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
    Outside(super::outside::Mutation),
    SetDeadline {
        room: String,
        agent: String,
        deadline: u64,
    },
    ReachDeadline {
        room: String,
        agent: String,
        now: u64,
        stopped: bool,
    },
    LeasedProcess(super::LeasedProcessMutation),
    WorkerStop(super::WorkerStopMutation),
    CreateLeasedWake {
        task: String,
        prompt: String,
        wake: AgentWake,
        binding: super::LeasedProcessBinding,
    },
    BeginContinuation {
        task: String,
        room: String,
        agent: String,
        prompt: String,
        run: Option<String>,
        now: u64,
    },
    PrepareCatalogContinuation {
        room: String,
        agent: String,
        prompt: String,
        run: String,
        next_prompt: String,
        now: u64,
    },
    AdoptGoal {
        owner: String,
        room: String,
        agent: String,
        run: String,
        nodes: Vec<String>,
        objective: String,
        now: u64,
    },
    TimeoutGoal {
        task: String,
        prompt: String,
        now: u64,
    },
    BindGoalCheckLease {
        task: String,
        prompt: String,
        lease: super::goals::CheckLease,
    },
    RetryGoalCheck {
        task: String,
        prompt: String,
        lease: super::goals::CheckLease,
        now: u64,
    },
    BeginGoalApproval {
        task: String,
        prompt: String,
        now: u64,
    },
    AnswerGoalApproval {
        task: String,
        prompt: String,
        revision: u64,
        now: u64,
    },
    BeginOwnerWait {
        task: String,
        prompt: String,
        now: u64,
    },
    EndOwnerWait {
        task: String,
        prompt: String,
        refusal: Option<String>,
        now: u64,
    },
    StartGoal {
        task: String,
        owner: String,
        objective: String,
        minutes: u32,
        now: u64,
    },
    EvaluateGoal {
        task: String,
        prompt: String,
        result: String,
        now: u64,
    },
    ControlGoal {
        task: String,
        owner: String,
        revision: u64,
        action: String,
        value: Option<String>,
        now: u64,
    },
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
        deadline: Option<u64>,
        reason: String,
        now: u64,
    },
    Block {
        task: String,
        prompt: String,
        reason: String,
    },
    BlockForBrowserAccess {
        task: String,
        prompt: String,
        reason: String,
    },
    /// A provider turn failed or vanished: a plain task is cancelled, a task with
    /// ledger work is blocked for the owner.
    EndFailedTurn {
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
    /// The first answer to a peer request of another kernel, so a retry gets the same one.
    RememberPeerRequest {
        kernel: String,
        request_id: String,
        answer: String,
    },
    /// An agent of another kernel follows `agent` of `room` here; an older generation of the
    /// same subscription changes nothing.
    FollowPeer {
        kernel: String,
        owner: String,
        subscription: String,
        room: String,
        agent: String,
        generation: u64,
    },
    /// Ends a follow, only for the kernel that holds it and never with an older generation.
    UnfollowPeer {
        kernel: String,
        subscription: String,
        generation: u64,
    },
    /// A followed agent or its room is gone: its followers on other kernels hear `source_lost`.
    PeerSourceLost {
        room: String,
        agent: String,
        now: u64,
    },
    PeerOutboxDelivered {
        delivery: String,
    },
    /// A room agent created a peer in another room.
    RecordCreatedPeer {
        peer: super::created_peers::CreatedPeer,
    },
    /// The peer of a creator that ended is deleted, or was already gone.
    ReleaseCreatedPeer {
        peer: super::created_peers::CreatedPeer,
    },
    /// Another attempt later; outcomes past their age are dropped with it.
    PeerOutboxRetry {
        delivery: String,
        at: u64,
        now: u64,
    },
    /// An event without a sender turn on this kernel, such as a message from a peer on
    /// another kernel; de-duplicated on (room, agent, source, occurrence).
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
        /// A04: an elevated task whose correlated wakes pass deferred events.
        work: Option<String>,
        /// Last sequence the wake covers. Every other pending row up to it
        /// that the wake may deliver joins the lead row's prompt and receipt.
        through: u64,
    },
    Expire {
        room: String,
        agent: String,
        sequence: u64,
    },
    /// An inbox read returned these rows to the agent: only shown rows can be acknowledged through a range.
    MarkShown {
        room: String,
        agent: String,
        sequences: Vec<u64>,
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
    /// A handled `Ack` of a `child_finished` receipt that, in the same write, marks the
    /// reported sub-agent's `subtree` (the child first) as deleting, so a stop after the
    /// ack still leaves the deletion for the lifecycle pass.
    AckHandledChild {
        room: String,
        agent: String,
        sequence: u64,
        subtree: Vec<String>,
        now: u64,
    },
    Progress {
        task: String,
        prompt: String,
        receipt: String,
        now: u64,
    },
    DelegateUnavailable {
        room: String,
        agent: String,
        reason: DelegateGoneReason,
        now: u64,
    },
    /// A deleted agent that was never given work: its parents' spawn
    /// obligations on it end without a failure.
    ReleaseSpawn {
        room: String,
        source: String,
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
        /// A local cold start paused at a typed Vault unlock, before any provider I/O.
        vault_wait_recipients: Vec<(String, String)>,
        /// A04: (room, agent, task) of sudo-held recipients; their unrelated
        /// events wait for the window to end like busy refusals.
        held_work: Vec<(String, String, String)>,
    },
    CancelTask {
        task: String,
        owner: String,
        revision: u64,
    },
    /// [`Operation::CancelTask`] for a cancel an agent (a peer, or a parent) issues: it ends the
    /// task, not the agent, so later messages still run.
    CancelTaskByAgent {
        task: String,
        owner: String,
        revision: u64,
    },
    /// Records who is stopping a running turn whose task settles as cancelled later. The owner's
    /// Stop always wins; an agent's cancel never overwrites it.
    NoteTurnCancel {
        task: String,
        initiator: CancelInitiator,
    },
    /// Rolls back an admission whose prompt submission was rejected.
    Withdraw {
        task: String,
    },
    /// Private kernel lineage; not a serialized client contract.
    ReconcileFailedWorkflow {
        owner_id: String,
        room: String,
        run: String,
    },
    BindWorkflow {
        task: String,
        prompt: String,
        run: String,
        node: String,
    },
    /// Private owner-turn admission; never cancels resources or delivery receipts.
    RetireRestartFailure {
        task: String,
        revision: u64,
        now: u64,
    },
    OwnerResponse {
        task: String,
        revision: u64,
        resume: bool,
        now: u64,
    },
    /// A kernel timer or watched process: the ledger sets its first state.
    CreateWake {
        task: String,
        prompt: String,
        wake: AgentWake,
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
    ProcessStarted {
        id: String,
        pid: u32,
        now: u64,
    },
    ProcessMatched {
        id: String,
        line: String,
        now: u64,
    },
    /// `exit_code: None` records a lost process; it is never relaunched.
    ProcessExited {
        id: String,
        exit_code: Option<i32>,
        tail: String,
        now: u64,
    },
    /// The owner can cancel a retained timer without a running provider turn.
    CancelOwnerTimer {
        id: String,
        room: String,
        owner: String,
    },
    /// `prompt: Some` is the owning agent's turn; `None` settles a cancelled
    /// task. `pending` means the driver must still stop the source physically
    /// and will report the end; a watched process always settles on its exit.
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
    /// (processes and other driver sources on their reported end) and expires
    /// its unresolved deliveries.
    RetireWakes {
        room: String,
        agent: Option<String>,
        now: u64,
    },
}
#[derive(Debug)]
pub(crate) enum Outcome {
    Deadline(crate::local::AgentDeadline),
    LeasedProcess(Option<super::LeasedProcessReceipt>),
    Task(AgentTaskExecution),
    Event(InboxEvent),
    Settled {
        task: AgentTaskExecution,
        correction: bool,
    },
    Swept(Vec<AgentTaskExecution>),
    Wakes(Vec<AgentWake>),
    PeerFollowed(super::peer_follows::PeerFollowed),
    /// Whether the link was new, or an earlier attempt of the same spawn had written it.
    CreatedPeerRecorded {
        inserted: bool,
    },
    Saved,
}
pub(crate) struct Request {
    pub(super) enqueued_at: std::time::Instant,
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
        && event.is_source_outcome()
        && task.obligations.iter().any(|obligation| {
            obligation
                .completion_source()
                .is_some_and(|source| event.reports_source(source))
                && matches!(obligation.status.as_str(), "settling" | "failed")
        })
}

// MP-08 / MP-09 / MP-10 / MP-11: a requested reply has an explicit terminal cause.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum ReplyOutcome {
    DelegateGone {
        delegate_id: String,
        reason: DelegateGoneReason,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DelegateGoneReason {
    Deleted,
    FailedStart,
    SourceLost,
}
impl DelegateGoneReason {
    pub(crate) fn answer(self, delegate: &str) -> serde_json::Value {
        serde_json::to_value(ReplyOutcome::DelegateGone {
            delegate_id: delegate.into(),
            reason: self,
        })
        .expect("reply outcome is serializable")
    }
}
