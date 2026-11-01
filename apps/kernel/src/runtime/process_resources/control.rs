use super::*;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProcessControlTarget {
    Agent { agent_id: String },
    Process { process_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessControlOutcome {
    OutsideEnd,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProcessControlResult {
    pub outcome: ProcessControlOutcome,
    pub process_ids: Vec<String>,
}

pub(super) enum WatchCommand {
    Shutdown,
    /// The owner ended; jobs that left it keep running under the watch until they end.
    Release,
    /// The memory policy chose this command (the root: the whole workload).
    MemoryStop {
        pid: u32,
        limit_bytes: u64,
        notice: String,
    },
    Stop {
        process_id: Option<String>,
        reply: mpsc::SyncSender<Result<Vec<String>, String>>,
    },
    /// Admission changed one of its commands: apply it now.
    Apply,
    /// Start now: the user admits this waiting command whatever the room.
    StartNow {
        process_id: String,
        reply: mpsc::SyncSender<Result<Vec<String>, String>>,
    },
}

pub(crate) fn stop_for_key(key: &str, process_id: Option<String>) -> Result<Vec<String>, String> {
    ask(key, |reply| WatchCommand::Stop { process_id, reply })
}

pub(crate) fn start_now_for_key(key: &str, process_id: String) -> Result<Vec<String>, String> {
    ask(key, |reply| WatchCommand::StartNow { process_id, reply })
}

fn ask(
    key: &str,
    command: impl FnOnce(mpsc::SyncSender<Result<Vec<String>, String>>) -> WatchCommand,
) -> Result<Vec<String>, String> {
    let state = registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(key)
        .and_then(std::sync::Weak::upgrade)
        .ok_or_else(|| "the process already ended; refresh Health".to_string())?;
    let sender = state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .control
        .clone()
        .ok_or_else(|| "process monitoring ended; refresh Health".to_string())?;
    let (reply, response) = mpsc::sync_channel(1);
    sender
        .send(command(reply))
        .map_err(|_| "process monitoring ended; refresh Health".to_string())?;
    response.recv_timeout(Duration::from_secs(3)).map_err(|_| {
        "the process control did not finish within its deadline; refresh Health".to_string()
    })?
}

pub(crate) fn outside_ended_for_key(key: &str) -> bool {
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(key)
        .and_then(std::sync::Weak::upgrade)
        .is_some_and(|state| {
            state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .outside_ended
        })
}

/// An owner that is gone for good, with the start sequence its end follows.
struct OwnerEnd {
    after: u64,
    session_id: Option<String>,
    owner: Option<SessionProcessOwner>,
}

static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static LIVE: Mutex<std::collections::BTreeSet<u64>> = Mutex::new(std::collections::BTreeSet::new());
static OWNER_ENDS: Mutex<Vec<OwnerEnd>> = Mutex::new(Vec::new());

pub(super) fn domain_started() -> u64 {
    let started = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    LIVE.lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(started);
    started
}

pub(super) fn domain_dropped(started: u64) {
    LIVE.lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&started);
}

/// Whether the owner of a workload that started at `started` is gone for good.
/// Asked under the registry lock, so no end slips between it and the release.
pub(super) fn owner_ended(
    started: u64,
    session_id: &str,
    owner: &SessionProcessOwner,
    opened_by: Option<&str>,
) -> bool {
    OWNER_ENDS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .any(|end| {
            started < end.after
                && end.session_id.as_deref().is_none_or(|id| id == session_id)
                && end
                    .owner
                    .as_ref()
                    .is_none_or(|ended| belongs_to(ended, owner, opened_by))
        })
}

/// Whether work of `owner` (a terminal opened by `opened_by`) is the work of
/// `ended`: an agent's own, and that of the Room terminals it opened.
fn belongs_to(
    ended: &SessionProcessOwner,
    owner: &SessionProcessOwner,
    opened_by: Option<&str>,
) -> bool {
    ended == owner
        || matches!(ended, SessionProcessOwner::Agent(agent_id) if opened_by == Some(agent_id))
}

/// The owner is gone for good (its Room ended or was deleted, the agent was
/// deleted, the kernel stops), unlike a restart: what it left running ends,
/// and so do the jobs of its workloads that are removed only afterwards.
pub(crate) fn end_owner(session_id: Option<&str>, owner: Option<&SessionProcessOwner>) {
    {
        let _registry = registry().lock().unwrap_or_else(|e| e.into_inner());
        let oldest = LIVE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .first()
            .copied();
        let mut ends = OWNER_ENDS.lock().unwrap_or_else(|e| e.into_inner());
        // An end concerns only workloads that started before it.
        ends.retain(|end| oldest.is_some_and(|oldest| oldest < end.after));
        ends.push(OwnerEnd {
            after: SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
            session_id: session_id.map(str::to_string),
            owner: owner.cloned(),
        });
    }
    stop_released(session_id, owner);
}

/// The owner of what an ended workload left running stops its work (or is
/// gone, through `end_owner`): those processes end too. An agent's work
/// includes its Room terminals' jobs, even once the terminals are closed.
pub(crate) fn stop_released(session_id: Option<&str>, owner: Option<&SessionProcessOwner>) {
    let keys = projection::released_keys(|released| {
        session_id.is_none_or(|id| id == released.session_id)
            && owner.is_none_or(|ended| {
                belongs_to(ended, &released.owner, released.opened_by.as_deref())
            })
    });
    for (_, key) in keys {
        if let Err(error) = stop_for_key(&key, None) {
            crate::logging::error_with_fields(
                "process_resources.released",
                "a released process could not be stopped with its owner",
                serde_json::json!({"error": error}),
            );
        }
    }
}
