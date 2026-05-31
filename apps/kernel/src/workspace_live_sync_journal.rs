use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::error::DaemonError;
use crate::git_observer::WorkspaceLiveSyncChange;

#[derive(Debug, Clone, Default)]
pub(crate) struct WorkspaceLiveSyncJournal {
    inner: Arc<Mutex<WorkspaceLiveSyncJournalState>>,
}

#[derive(Debug, Clone, Default)]
struct WorkspaceLiveSyncJournalState {
    entries: Vec<WorkspaceLiveSyncJournalEntry>,
    next_sequence_by_link: BTreeMap<String, u64>,
    target_results: Vec<WorkspaceLiveSyncTargetResult>,
}

impl WorkspaceLiveSyncJournal {
    pub(crate) fn restore_from_durable_state(
        store: &crate::durable_state::DurableKernelStateStore,
    ) -> Result<Self, DaemonError> {
        let journal = Self::default();
        for event in store.load_events_after(0)? {
            journal.restore_durable_event(&event)?;
        }
        Ok(journal)
    }

    pub(crate) fn append_for_link(
        &self,
        link_id: &str,
        link_name: &str,
        change: WorkspaceLiveSyncChange,
    ) -> WorkspaceLiveSyncJournalEntry {
        let mut state = self
            .inner
            .lock()
            .expect("workspace live sync journal mutex poisoned");
        let next_sequence = state
            .next_sequence_by_link
            .entry(link_id.to_string())
            .or_insert(1);
        let entry = WorkspaceLiveSyncJournalEntry {
            sequence: *next_sequence,
            link_id: link_id.to_string(),
            link_name: link_name.to_string(),
            change,
        };
        *next_sequence += 1;
        state.entries.push(entry.clone());
        entry
    }

    pub(crate) fn record_target_results(&self, results: Vec<WorkspaceLiveSyncTargetResult>) {
        if results.is_empty() {
            return;
        }
        self.inner
            .lock()
            .expect("workspace live sync journal mutex poisoned")
            .target_results
            .extend(results);
    }

    fn restore_durable_event(
        &self,
        event: &crate::durable_state::DurableStateEvent,
    ) -> Result<(), DaemonError> {
        match event.kind.as_str() {
            "workspace_live_sync.change_recorded" => {
                let entry: WorkspaceLiveSyncJournalEntry =
                    decode_workspace_live_sync_durable_payload_field(
                        event,
                        "entry",
                        "workspace_live_sync.restore_change",
                    )?;
                self.restore_entry(entry);
            }
            "workspace_live_sync.target_results_recorded" => {
                let target_results: Vec<WorkspaceLiveSyncTargetResult> =
                    decode_workspace_live_sync_durable_payload_field(
                        event,
                        "target_results",
                        "workspace_live_sync.restore_target_results",
                    )?;
                self.record_target_results(target_results);
            }
            _ => {}
        }
        Ok(())
    }

    fn restore_entry(&self, entry: WorkspaceLiveSyncJournalEntry) {
        let mut state = self
            .inner
            .lock()
            .expect("workspace live sync journal mutex poisoned");
        let next_sequence = state
            .next_sequence_by_link
            .entry(entry.link_id.clone())
            .or_insert(1);
        *next_sequence = (*next_sequence).max(entry.sequence.saturating_add(1));
        state.entries.push(entry);
    }

    pub(crate) fn target_results_for_session(
        &self,
        session_id: &str,
    ) -> Vec<WorkspaceLiveSyncTargetResult> {
        self.inner
            .lock()
            .expect("workspace live sync journal mutex poisoned")
            .target_results
            .iter()
            .filter(|result| result.session_id == session_id)
            .cloned()
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn entries_for_session(
        &self,
        session_id: &str,
    ) -> Vec<WorkspaceLiveSyncJournalEntry> {
        self.inner
            .lock()
            .expect("workspace live sync journal mutex poisoned")
            .entries
            .iter()
            .filter(|entry| entry.change.session_id == session_id)
            .cloned()
            .collect()
    }
}

fn decode_workspace_live_sync_durable_payload_field<T>(
    event: &crate::durable_state::DurableStateEvent,
    field: &'static str,
    operation: &'static str,
) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned,
{
    let value = event
        .payload
        .get(field)
        .cloned()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation,
            message: format!(
                "durable state event {} ({}) missing payload field {field}",
                event.event_id, event.kind
            ),
        })?;
    serde_json::from_value(value).map_err(|error| DaemonError::LocalTransport {
        operation,
        message: format!(
            "durable state event {} ({}) has invalid payload field {field}: {error}",
            event.event_id, event.kind
        ),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct WorkspaceLiveSyncJournalEntry {
    pub sequence: u64,
    pub link_id: String,
    pub link_name: String,
    pub change: WorkspaceLiveSyncChange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceLiveSyncTargetResult {
    pub session_id: String,
    pub link_id: String,
    pub link_name: String,
    pub source_agent_id: String,
    pub source_worktree_path: String,
    pub target_user_id: String,
    pub target_machine_id: String,
    pub target_kernel_id: String,
    pub target_repo_root: String,
    pub path_results: Vec<WorkspaceLiveSyncPathApplyResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceLiveSyncPathApplyResult {
    pub path: String,
    pub status: WorkspaceLiveSyncApplyStatus,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceLiveSyncApplyStatus {
    Applied,
    Rebased,
    SkippedConflict,
    FailedIo,
}
