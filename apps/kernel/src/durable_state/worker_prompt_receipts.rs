use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::app::LeaseCallerBinding;
use crate::error::DaemonError;

const EVENT_KIND: &str = "remote_lease.prompt_admission_receipt";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkerPromptReceiptPhase {
    Dispatching,
    Accepted,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct WorkerPromptReceipt {
    pub(crate) home_prompt_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) worker_provider_run_id: Option<String>,
    // MP-08 / MP-10 / MP-11: private durable home/worker identity binding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) backing_prompt_id: Option<String>,
    pub(crate) execution_lease_id: String,
    pub(crate) phase: WorkerPromptReceiptPhase,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct WorkerPromptReceiptRecord {
    pub(crate) leased_agent_id: String,
    pub(crate) receipt: WorkerPromptReceipt,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) caller: Option<LeaseCallerBinding>,
}

#[derive(Debug, Clone)]
pub(crate) struct WorkerPromptReceiptStore {
    durable_state: crate::durable_state::DurableKernelStateStore,
    records: BTreeMap<(String, String), WorkerPromptReceiptRecord>,
}

impl WorkerPromptReceiptStore {
    pub(crate) fn restore(
        durable_state: crate::durable_state::DurableKernelStateStore,
    ) -> Result<Self, DaemonError> {
        let mut records: BTreeMap<(String, String), WorkerPromptReceiptRecord> = BTreeMap::new();
        for event in durable_state.load_events_by_kind(EVENT_KIND)? {
            let record: WorkerPromptReceiptRecord =
                serde_json::from_value(event.payload).map_err(|error| {
                    DaemonError::LocalTransport {
                        operation: "restore worker prompt receipt",
                        message: error.to_string(),
                    }
                })?;
            if event.subject_id.as_deref() != Some(record.leased_agent_id.as_str())
                || !receipt_record_has_identity(&record)
            {
                return Err(DaemonError::LocalTransport {
                    operation: "restore worker prompt receipt",
                    message: "durable worker prompt receipt has incomplete or conflicting identity"
                        .to_string(),
                });
            }
            let key = (
                record.leased_agent_id.clone(),
                record.receipt.home_prompt_id.clone(),
            );
            if let Some(previous) = records.get(&key) {
                validate_receipt_transition(previous, &record)?;
            }
            records.insert(key, record);
        }
        Ok(Self {
            durable_state,
            records,
        })
    }

    pub(crate) fn get(
        &self,
        leased_agent_id: &str,
        home_prompt_id: &str,
    ) -> Option<&WorkerPromptReceiptRecord> {
        self.records
            .get(&(leased_agent_id.to_string(), home_prompt_id.to_string()))
    }

    // MP-08 / MP-10 / MP-11: late worker completions retain their home identity.
    pub(crate) fn home_prompt_for_backing(
        &self,
        leased_agent_id: &str,
        execution_lease_id: &str,
        provider_run_id: &str,
        backing_prompt_id: &str,
    ) -> Option<&str> {
        let mut matches = self.records.values().filter(|record| {
            record.leased_agent_id == leased_agent_id
                && record.receipt.execution_lease_id == execution_lease_id
                && record.receipt.phase == WorkerPromptReceiptPhase::Accepted
                && record.receipt.worker_provider_run_id.as_deref() == Some(provider_run_id)
                && record.receipt.backing_prompt_id.as_deref() == Some(backing_prompt_id)
        });
        let record = matches.next()?;
        // Ambiguous records never choose an arbitrary home turn.
        matches
            .next()
            .is_none()
            .then_some(record.receipt.home_prompt_id.as_str())
    }

    pub(crate) fn backing_bindings_for_run(
        &self,
        leased_agent_id: &str,
        execution_lease_id: &str,
        provider_run_id: &str,
    ) -> Vec<(String, String)> {
        let mut bindings: BTreeMap<&str, Option<&str>> = BTreeMap::new();
        for record in self.records.values().filter(|record| {
            record.leased_agent_id == leased_agent_id
                && record.receipt.execution_lease_id == execution_lease_id
                && record.receipt.worker_provider_run_id.as_deref() == Some(provider_run_id)
                && record.receipt.phase == WorkerPromptReceiptPhase::Accepted
        }) {
            let Some(backing) = record.receipt.backing_prompt_id.as_deref() else {
                continue;
            };
            bindings
                .entry(backing)
                .and_modify(|home| *home = None)
                .or_insert(Some(&record.receipt.home_prompt_id));
        }
        bindings
            .into_iter()
            .filter_map(|(backing, home)| home.map(|home| (home.to_string(), backing.to_string())))
            .collect()
    }

    pub(crate) fn caller_for_leased_agent(
        &self,
        leased_agent_id: &str,
    ) -> Option<LeaseCallerBinding> {
        let mut caller: Option<LeaseCallerBinding> = None;
        for record in self
            .records
            .values()
            .filter(|record| record.leased_agent_id == leased_agent_id)
        {
            let Some(record_caller) = record.caller.as_ref() else {
                continue;
            };
            if caller
                .as_ref()
                .is_some_and(|current| current != record_caller)
            {
                return None;
            }
            caller = Some(record_caller.clone());
        }
        caller
    }

    pub(crate) fn persist(
        &mut self,
        mut record: WorkerPromptReceiptRecord,
    ) -> Result<WorkerPromptReceiptRecord, DaemonError> {
        if !receipt_record_has_identity(&record) {
            return Err(DaemonError::LocalTransport {
                operation: "persist worker prompt receipt",
                message: "worker prompt receipt identity fields are required".to_string(),
            });
        }
        let key = (
            record.leased_agent_id.clone(),
            record.receipt.home_prompt_id.clone(),
        );
        if let Some(previous) = self.records.get(&key) {
            validate_receipt_transition(previous, &record)?;
            if record.caller.is_none() {
                record.caller = previous.caller.clone();
            }
            if previous == &record {
                return Ok(record);
            }
        }
        self.durable_state.append_event(
            EVENT_KIND,
            Some(record.leased_agent_id.clone()),
            serde_json::to_value(&record).map_err(|error| DaemonError::LocalTransport {
                operation: "encode worker prompt receipt",
                message: error.to_string(),
            })?,
        )?;
        self.records.insert(key, record.clone());
        Ok(record)
    }
}

fn receipt_record_has_identity(record: &WorkerPromptReceiptRecord) -> bool {
    let receipt = &record.receipt;
    !record.leased_agent_id.trim().is_empty()
        && !receipt.home_prompt_id.trim().is_empty()
        && !receipt.execution_lease_id.trim().is_empty()
        && receipt
            .worker_provider_run_id
            .as_deref()
            .is_none_or(|run_id| !run_id.trim().is_empty())
        && receipt
            .backing_prompt_id
            .as_deref()
            .is_none_or(|id| !id.trim().is_empty())
        && (receipt.phase != WorkerPromptReceiptPhase::Accepted
            || receipt.worker_provider_run_id.is_some())
}

fn validate_receipt_transition(
    previous: &WorkerPromptReceiptRecord,
    next: &WorkerPromptReceiptRecord,
) -> Result<(), DaemonError> {
    if previous.leased_agent_id != next.leased_agent_id
        || previous.receipt.home_prompt_id != next.receipt.home_prompt_id
        || previous.receipt.execution_lease_id != next.receipt.execution_lease_id
        || previous
            .caller
            .as_ref()
            .zip(next.caller.as_ref())
            .is_some_and(|(old, new)| old != new)
    {
        return Err(receipt_transition_error(
            "worker prompt receipt binding identity or authenticated home changed",
        ));
    }

    if previous.receipt.phase != WorkerPromptReceiptPhase::Dispatching {
        if previous == next {
            return Ok(());
        }
        return Err(receipt_transition_error(
            "worker prompt receipt cannot change a settled outcome",
        ));
    }

    let allowed_phase = next.receipt.phase == WorkerPromptReceiptPhase::Dispatching
        || matches!(
            next.receipt.phase,
            WorkerPromptReceiptPhase::Accepted | WorkerPromptReceiptPhase::Rejected
        );
    let run_identity_is_monotonic = match (
        previous.receipt.worker_provider_run_id.as_deref(),
        next.receipt.worker_provider_run_id.as_deref(),
    ) {
        (Some(previous), Some(next)) => previous == next,
        (Some(_), None) => false,
        (None, _) => true,
    };
    let backing_identity_is_monotonic = previous
        .receipt
        .backing_prompt_id
        .as_deref()
        .is_none_or(|old| next.receipt.backing_prompt_id.as_deref() == Some(old));
    if !allowed_phase || !run_identity_is_monotonic || !backing_identity_is_monotonic {
        return Err(receipt_transition_error(
            "worker prompt receipt cannot change a settled outcome or provider-run identity",
        ));
    }
    Ok(())
}

fn receipt_transition_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "persist worker prompt receipt",
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receipt(phase: WorkerPromptReceiptPhase) -> WorkerPromptReceiptRecord {
        WorkerPromptReceiptRecord {
            leased_agent_id: "leased-agent-1".to_string(),
            receipt: WorkerPromptReceipt {
                home_prompt_id: "home-prompt-1".to_string(),
                backing_prompt_id: None,
                worker_provider_run_id: None,
                execution_lease_id: "lease-1".to_string(),
                phase,
            },
            caller: None,
        }
    }

    #[test]
    fn worker_prompt_receipts_survive_restart_and_fence_settled_admission() {
        let root = std::env::temp_dir().join(format!(
            "chariox-worker-prompt-receipts-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms(),
        ));
        let path = root.join("state.db");
        let durable = crate::durable_state::DurableKernelStateStore::open(path.clone())
            .expect("durable state should open");
        let mut store = WorkerPromptReceiptStore::restore(durable.clone())
            .expect("empty receipt state should restore");
        store
            .persist(receipt(WorkerPromptReceiptPhase::Dispatching))
            .expect("dispatching receipt should persist");
        let mut accepted = receipt(WorkerPromptReceiptPhase::Accepted);
        accepted.receipt.worker_provider_run_id = Some("worker-run-1".to_string());
        accepted.receipt.backing_prompt_id = Some("backing-prompt-1".into());
        store
            .persist(accepted.clone())
            .expect("acceptance should persist");
        assert!(store
            .persist(receipt(WorkerPromptReceiptPhase::Rejected))
            .is_err());
        let mut changed_backing = accepted.clone();
        changed_backing.receipt.backing_prompt_id = Some("backing-prompt-2".into());
        assert!(store.persist(changed_backing).is_err());
        assert_eq!(
            store.home_prompt_for_backing(
                "leased-agent-1",
                "lease-1",
                "worker-run-1",
                "backing-prompt-1"
            ),
            Some("home-prompt-1")
        );
        assert!(store
            .home_prompt_for_backing(
                "leased-agent-other",
                "lease-1",
                "worker-run-1",
                "backing-prompt-1"
            )
            .is_none());
        assert!(store
            .home_prompt_for_backing(
                "leased-agent-1",
                "lease-other",
                "worker-run-1",
                "backing-prompt-1"
            )
            .is_none());
        assert!(store
            .home_prompt_for_backing(
                "leased-agent-1",
                "lease-1",
                "worker-run-other",
                "backing-prompt-1"
            )
            .is_none());
        assert_eq!(
            store.backing_bindings_for_run("leased-agent-1", "lease-1", "worker-run-1"),
            vec![("home-prompt-1".into(), "backing-prompt-1".into())]
        );
        assert!(store
            .backing_bindings_for_run("leased-agent-1", "lease-other", "worker-run-1")
            .is_empty());
        let mut changed_lease = accepted.clone();
        changed_lease.receipt.execution_lease_id = "lease-2".to_string();
        assert!(store.persist(changed_lease).is_err());
        drop(store);
        drop(durable);

        let reopened = crate::durable_state::DurableKernelStateStore::open(path)
            .expect("durable state should reopen");
        let mut restored = WorkerPromptReceiptStore::restore(reopened.clone())
            .expect("accepted receipt should restore");
        assert_eq!(
            restored.get("leased-agent-1", "home-prompt-1"),
            Some(&accepted)
        );
        assert!(restored
            .persist(receipt(WorkerPromptReceiptPhase::Dispatching))
            .is_err());
        drop(restored);
        drop(reopened);
        let _ = std::fs::remove_dir_all(root);
    }
}
