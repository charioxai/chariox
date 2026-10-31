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
}

/// Whether a command runs or waits for room (admission).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionProcessState {
    #[default]
    Running,
    /// Stopped before it grew, in line for room for its expected need.
    Held {
        reason: admission::WaitReason,
        /// Place in line among commands waiting for the same reason, from 1.
        position: u32,
        need_bytes: u64,
        free_bytes: u64,
        command_line: String,
    },
    /// Stopped while growing into a full pool or disk; continues with room.
    Paused {
        reason: admission::WaitReason,
        command_line: String,
    },
}

/// Whom a workload's processes are listed under when no run or terminal does:
/// what an ended workload left running, or what an agent started through a
/// runtime tool.
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
    for (owner, key) in keys_for_session(session_id, providers, terminals) {
        let states = states_for_key(&key);
        rows.extend(
            samples_for_key(&key)
                .into_iter()
                .map(|process| SessionProcess {
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
            let command_line = entry
                .seen
                .as_ref()
                .map(|seen| seen.line.clone())
                .unwrap_or_default();
            let state = match entry.wait? {
                admission::Wait::Held(reason) => SessionProcessState::Held {
                    reason,
                    position: entry.position,
                    need_bytes: entry.need,
                    free_bytes: entry.free,
                    command_line,
                },
                admission::Wait::Paused(reason) => SessionProcessState::Paused {
                    reason,
                    command_line,
                },
            };
            Some((*pid, state))
        })
        .collect()
}

pub(crate) fn keys_for_session(
    session_id: &str,
    providers: &crate::provider::ProviderProcessServiceStore,
    terminals: &crate::room_terminal::RoomTerminalService,
) -> Vec<(SessionProcessOwner, String)> {
    let mut keys = providers
        .list_runs()
        .into_iter()
        .filter(|run| run.session_id() == session_id)
        .filter_map(|run| {
            run.agent_instance_id().map(|agent| {
                (
                    SessionProcessOwner::Agent(agent.to_string()),
                    run.id().to_string(),
                )
            })
        })
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
