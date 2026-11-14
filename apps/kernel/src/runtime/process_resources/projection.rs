//! Session-scoped read model: only launch-owned observations reach clients.
use super::{admission, samples_for_key, ProcessSample};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum SessionProcessOwner {
    Agent(String),
    Terminal(String),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SessionProcess {
    pub owner: SessionProcessOwner,
    #[serde(flatten)]
    pub process: ProcessSample,
    /// Set on a command's own process; what it started waits with it.
    pub state: SessionProcessState,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub remote_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_refusal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate: Option<crate::local::RemoteJobEstimate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<crate::local::RemoteJobPlacement>,
}

/// A quiet process is repeated this often, so a client that stops hearing of a row can tell a
/// finished process from a feed that went silent.
const REPEAT_QUIET_PROCESS_MS: u64 = 30_000;
const MEANINGFUL_RESIDENT_SET_BYTES: u64 = 1024 * 1024;

/// Whether clients that already hold `published` learn nothing from `current`: the same processes in
/// the same state, sampled again. A row's sample time and age move on every sample and its memory
/// jitters, so those alone are not a change; memory counts from 1 MiB or 5 %, whichever is larger,
/// and a row not repeated for 30 s is sent again. `published` is the last list that was sent, so
/// a slow drift adds up against it instead of slipping through one small step at a time.
pub(crate) fn same_to_clients(published: &[SessionProcess], current: &[SessionProcess]) -> bool {
    published.len() == current.len()
        && published.iter().zip(current).all(|(before, now)| {
            let (a, b) = (&before.process, &now.process);
            let meaningful = MEANINGFUL_RESIDENT_SET_BYTES
                .max(a.resident_set_bytes.max(b.resident_set_bytes) / 20);
            before.owner == now.owner
                && before.state == now.state
                && before.remote_available == now.remote_available
                && before.remote_refusal == now.remote_refusal
                && before.estimate == now.estimate
                && before.placement == now.placement
                && a.process_id == b.process_id
                && a.pid == b.pid
                && a.command == b.command
                && a.cpu_millis == b.cpu_millis
                && a.disk_io_bytes == b.disk_io_bytes
                && a.disk_written_bytes == b.disk_written_bytes
                && a.deleted_open_bytes == b.deleted_open_bytes
                && a.outlived == b.outlived
                && a.resident_set_bytes.abs_diff(b.resident_set_bytes) < meaningful
                && b.sampled_at_ms.saturating_sub(a.sampled_at_ms) < REPEAT_QUIET_PROCESS_MS
        })
}

/// Whether a command runs or waits for room (admission).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionProcessState {
    #[default]
    Running,
    /// Stopped before it grew, in line for room for its expected need.
    Held(Waiting),
    /// Stopped while growing into a full pool or disk; continues with room.
    Paused(Waiting),
}

/// The one description of a waiting command: the row clients show, the notice
/// its agent reads and the event the room's followers get all come from it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Waiting {
    pub reason: admission::WaitReason,
    /// Place in line among commands waiting for the same reason, from 1.
    pub position: u32,
    pub need_bytes: u64,
    pub free_bytes: u64,
    pub command_line: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// When it began to wait (Unix ms).
    pub since_ms: u64,
}

/// Whom a workload's processes are listed under when no run or terminal does:
/// what an ended workload left running, or what an agent started through a
/// runtime tool.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct Listing {
    pub(super) session_id: String,
    pub(super) owner: SessionProcessOwner,
    /// The agent that opened a terminal owner, which stays after the terminal
    /// is closed: archiving that agent ends its terminals' jobs too.
    pub(super) opened_by: Option<String>,
}

pub(crate) fn for_session(
    session_id: &str,
    providers: &crate::provider::ProviderProcessServiceStore,
    terminals: &crate::room_terminal::RoomTerminalService,
) -> Vec<SessionProcess> {
    let mut rows = Vec::new();
    for (owner, key) in observed_keys_for_session(session_id, providers, terminals) {
        let states = states_for_key(&key);
        rows.extend(
            samples_for_key(&key)
                .into_iter()
                .map(|process| SessionProcess {
                    remote_available: false,
                    remote_refusal: None,
                    estimate: None,
                    placement: None,
                    owner: owner.clone(),
                    state: states.get(&process.pid).cloned().unwrap_or_default(),
                    process,
                }),
        );
    }
    rows.sort_by_key(|row| row.process.pid);
    rows.dedup_by_key(|row| row.process.pid);
    rows
}

/// The row for a command: running, or waiting with everything clients show.
pub(super) fn state_of(entry: &admission::Entry) -> SessionProcessState {
    let Some(wait) = entry.wait else {
        return SessionProcessState::Running;
    };
    let waiting = Waiting {
        reason: wait.reason(),
        position: entry.position,
        need_bytes: entry.need,
        free_bytes: entry.free,
        command_line: entry
            .seen
            .as_ref()
            .map(|seen| seen.line.clone())
            .unwrap_or_default(),
        cwd: entry
            .seen
            .as_ref()
            .and_then(|seen| seen.cwd.as_ref())
            .map(|cwd| cwd.display().to_string()),
        since_ms: entry.since_ms,
    };
    match wait {
        admission::Wait::Held(_) => SessionProcessState::Held(waiting),
        admission::Wait::Paused(_) => SessionProcessState::Paused(waiting),
    }
}

/// The waiting commands of one workload, by their own process.
pub(super) fn states_for_key(key: &str) -> std::collections::BTreeMap<u32, SessionProcessState> {
    let Some(state) = super::registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(key)
        .and_then(std::sync::Weak::upgrade)
    else {
        return Default::default();
    };
    let observation = state.lock().unwrap_or_else(|e| e.into_inner());
    observation
        .admission
        .iter()
        .filter_map(|(pid, entry)| {
            let state = state_of(entry);
            (state != SessionProcessState::Running).then_some((*pid, state))
        })
        .collect()
}

pub(crate) fn keys_for_session(
    session_id: &str,
    providers: &crate::provider::ProviderProcessServiceStore,
    terminals: &crate::room_terminal::RoomTerminalService,
) -> Vec<(SessionProcessOwner, String)> {
    let mut keys = providers
        .run_owners_for_session(session_id)
        .into_iter()
        .map(|(agent, run)| (SessionProcessOwner::Agent(agent), run))
        .collect::<Vec<_>>();
    keys.extend(terminals.process_keys(session_id));
    keys.extend(listed_keys(
        |listing| listing.session_id == session_id,
        false,
    ));
    keys
}

// Reads need only observed workloads. Command control uses keys_for_session,
// which also includes launches that have not created their process domain yet.
fn observed_keys_for_session(
    session_id: &str,
    providers: &crate::provider::ProviderProcessServiceStore,
    terminals: &crate::room_terminal::RoomTerminalService,
) -> Vec<(SessionProcessOwner, String)> {
    let observed_keys = {
        let mut registry = super::registry().lock().unwrap_or_else(|e| e.into_inner());
        registry.retain(|_, state| state.strong_count() > 0);
        registry.keys().cloned().collect::<Vec<_>>()
    };
    let mut keys = providers
        .observed_run_owners_for_session(session_id, &observed_keys)
        .into_iter()
        .map(|(agent, run)| (SessionProcessOwner::Agent(agent), run))
        .collect::<Vec<_>>();
    keys.extend(terminals.process_keys(session_id));
    keys.extend(listed_keys(
        |listing| listing.session_id == session_id,
        false,
    ));
    keys
}

#[cfg(test)]
pub(crate) fn released_keys_for_session(session_id: &str) -> Vec<(SessionProcessOwner, String)> {
    released_keys(|listing| listing.session_id == session_id)
}

/// Processes an ended workload left running (`setsid nohup job &`).
pub(super) fn released_keys(
    select: impl Fn(&Listing) -> bool,
) -> Vec<(SessionProcessOwner, String)> {
    listed_keys(select, true)
}

/// Workloads listed under an explicit owner: what an ended workload left
/// running, and processes an agent started through a runtime tool.
fn listed_keys(
    select: impl Fn(&Listing) -> bool,
    released_only: bool,
) -> Vec<(SessionProcessOwner, String)> {
    let mut registry = super::registry().lock().unwrap_or_else(|e| e.into_inner());
    registry.retain(|_, state| state.strong_count() > 0);
    registry
        .iter()
        .filter_map(|(key, state)| {
            let state = state.upgrade()?;
            let observation = state.lock().unwrap_or_else(|e| e.into_inner());
            let listing = observation.listed.as_ref()?;
            (select(listing) && (observation.released || !released_only))
                .then(|| (listing.owner.clone(), key.clone()))
        })
        .collect()
}
