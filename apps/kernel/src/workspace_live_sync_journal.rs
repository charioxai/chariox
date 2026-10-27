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
        for kind in [
            "workspace_live_sync.change_recorded",
            "workspace_live_sync.target_results_recorded",
        ] {
            for event in store.load_events_by_kind(kind)? {
                journal.restore_durable_event(&event)?;
            }
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

pub(crate) fn workspace_live_sync_notice_messages(
    change: &WorkspaceLiveSyncChange,
    target_results: &[WorkspaceLiveSyncTargetResult],
) -> Vec<String> {
    if target_results.is_empty() {
        return Vec::new();
    }
    let mode_label = if matches!(
        change.status_fingerprint.as_str(),
        "managed_workspace_live_sync" | "remote_managed_workspace_live_sync"
    ) {
        "managed"
    } else {
        "tracked turn"
    };
    let mut applied_targets = 0usize;
    let mut rebased_count = 0usize;
    let mut conflict_count = 0usize;
    let mut failed_count = 0usize;
    let mut target_details = Vec::new();
    let mut notices = Vec::new();
    for target_result in target_results {
        let mut target_has_applied = false;
        for path_result in &target_result.path_results {
            match path_result.status {
                WorkspaceLiveSyncApplyStatus::Applied => {
                    target_has_applied = true;
                    target_details.push(workspace_live_sync_target_detail(
                        target_result,
                        path_result,
                        "applied",
                    ));
                }
                WorkspaceLiveSyncApplyStatus::Rebased => {
                    target_has_applied = true;
                    rebased_count += 1;
                    target_details.push(workspace_live_sync_target_detail(
                        target_result,
                        path_result,
                        "rebased",
                    ));
                }
                WorkspaceLiveSyncApplyStatus::SkippedConflict => {
                    conflict_count += 1;
                    target_details.push(workspace_live_sync_target_detail(
                        target_result,
                        path_result,
                        "conflict",
                    ));
                    notices.push(format!(
                        "Workspace live sync conflict: source agent `{}` changed `{}` but target user `{}` worktree `{}` could not apply it: {}. Next action: assign a resolver agent to reread and reconcile the target worktree.",
                        change.agent_id,
                        path_result.path,
                        target_result.target_user_id,
                        target_result.target_repo_root,
                        path_result.message
                    ));
                }
                WorkspaceLiveSyncApplyStatus::FailedIo => {
                    failed_count += 1;
                    target_details.push(workspace_live_sync_target_detail(
                        target_result,
                        path_result,
                        "failed_io",
                    ));
                    notices.push(format!(
                        "Workspace live sync failed: source agent `{}` changed `{}` but target user `{}` worktree `{}` could not apply it: {}. Next action: verify the target worktree is attached and writable, then ask a resolver agent to recheck and re-edit if needed.",
                        change.agent_id,
                        path_result.path,
                        target_result.target_user_id,
                        target_result.target_repo_root,
                        path_result.message
                    ));
                }
            }
        }
        if target_has_applied {
            applied_targets += 1;
        }
    }
    let next_action = if conflict_count > 0 || failed_count > 0 {
        "review the listed conflict/failure notices and assign a resolver agent where needed"
    } else {
        "none"
    };
    notices.push(format!(
        "Workspace live sync {} summary: source agent `{}` changed {} path{}; applied to {} target{}; rebased={}; conflicts={}; failed_io={}; target results: {}; Next action: {}.",
        mode_label,
        change.agent_id,
        change.changed_paths.len(),
        if change.changed_paths.len() == 1 { "" } else { "s" },
        applied_targets,
        if applied_targets == 1 { "" } else { "s" },
        rebased_count,
        conflict_count,
        failed_count,
        workspace_live_sync_target_details_summary(&target_details),
        next_action
    ));
    notices
}

fn workspace_live_sync_target_detail(
    target_result: &WorkspaceLiveSyncTargetResult,
    path_result: &WorkspaceLiveSyncPathApplyResult,
    status: &str,
) -> String {
    format!(
        "target user `{}` worktree `{}` path `{}` {}",
        target_result.target_user_id, target_result.target_repo_root, path_result.path, status
    )
}

fn workspace_live_sync_target_details_summary(details: &[String]) -> String {
    const MAX_DETAILS: usize = 6;
    if details.is_empty() {
        return "none".to_string();
    }
    let mut shown = details
        .iter()
        .take(MAX_DETAILS)
        .cloned()
        .collect::<Vec<_>>();
    if details.len() > MAX_DETAILS {
        shown.push(format!("{} more", details.len() - MAX_DETAILS));
    }
    shown.join("; ")
}
