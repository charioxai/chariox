use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::app::LeaseCallerBinding;
use crate::error::DaemonError;
use crate::execution_lease::{LeasedPromptSteerReceipt, LeasedPromptSteerReceiptPhase};

const EVENT_KIND: &str = "remote_lease.queued_prompt_steer_receipt";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct WorkerSteerReceiptRecord {
    pub(crate) leased_agent_id: String,
    pub(crate) receipt: LeasedPromptSteerReceipt,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) caller: Option<LeaseCallerBinding>,
}

#[derive(Debug, Clone)]
pub(crate) struct WorkerSteerReceiptStore {
    durable_state: crate::durable_state::DurableKernelStateStore,
    records: BTreeMap<(String, String), WorkerSteerReceiptRecord>,
}

impl WorkerSteerReceiptStore {
    pub(crate) fn restore(
        durable_state: crate::durable_state::DurableKernelStateStore,
    ) -> Result<Self, DaemonError> {
        let mut records = BTreeMap::new();
        for event in durable_state.load_events_by_kind(EVENT_KIND)? {
            let record: WorkerSteerReceiptRecord = serde_json::from_value(event.payload).map_err(
                |error| DaemonError::LocalTransport {
                    operation: "restore worker queued-steer receipt",
                    message: error.to_string(),
                },
            )?;
            if event.subject_id.as_deref() != Some(record.leased_agent_id.as_str())
                || !receipt_record_has_identity(&record)
            {
                return Err(DaemonError::LocalTransport {
                    operation: "restore worker queued-steer receipt",
                    message: "durable worker receipt has incomplete or conflicting identity"
                        .to_string(),
                });
            }
            let key = (
                record.leased_agent_id.clone(),
                record.receipt.steer_id.clone(),
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
        steer_id: &str,
    ) -> Option<&WorkerSteerReceiptRecord> {
        self.records
            .get(&(leased_agent_id.to_string(), steer_id.to_string()))
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
        mut record: WorkerSteerReceiptRecord,
    ) -> Result<WorkerSteerReceiptRecord, DaemonError> {
        if !receipt_record_has_identity(&record) {
            return Err(DaemonError::LocalTransport {
                operation: "persist worker queued-steer receipt",
                message: "worker receipt identity fields are required".to_string(),
            });
        }
        let key = (
            record.leased_agent_id.clone(),
            record.receipt.steer_id.clone(),
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
                operation: "encode worker queued-steer receipt",
                message: error.to_string(),
            })?,
        )?;
        self.records.insert(key, record.clone());
        Ok(record)
    }
}

fn receipt_record_has_identity(record: &WorkerSteerReceiptRecord) -> bool {
    !record.leased_agent_id.trim().is_empty()
        && !record.receipt.steer_id.trim().is_empty()
        && !record.receipt.target_home_prompt_id.trim().is_empty()
        && !record.receipt.worker_provider_run_id.trim().is_empty()
        && !record.receipt.execution_lease_id.trim().is_empty()
}

fn validate_receipt_transition(
    previous: &WorkerSteerReceiptRecord,
    next: &WorkerSteerReceiptRecord,
) -> Result<(), DaemonError> {
    if previous.leased_agent_id != next.leased_agent_id
        || previous.receipt.steer_id != next.receipt.steer_id
        || previous.receipt.target_home_prompt_id != next.receipt.target_home_prompt_id
        || previous.receipt.worker_provider_run_id != next.receipt.worker_provider_run_id
        || previous.receipt.execution_lease_id != next.receipt.execution_lease_id
        || previous
            .caller
            .as_ref()
            .zip(next.caller.as_ref())
            .is_some_and(|(old, new)| old != new)
    {
        return Err(DaemonError::LocalTransport {
            operation: "persist worker queued-steer receipt",
            message: "receipt identity or authenticated home binding changed".to_string(),
        });
    }
    let allowed = previous.receipt.phase == next.receipt.phase
        || previous.receipt.phase == LeasedPromptSteerReceiptPhase::Dispatching
            && matches!(
                next.receipt.phase,
                LeasedPromptSteerReceiptPhase::Accepted | LeasedPromptSteerReceiptPhase::Rejected
            );
    if !allowed {
        return Err(DaemonError::LocalTransport {
            operation: "persist worker queued-steer receipt",
            message: "worker receipt cannot change a settled outcome".to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution_lease::LeasedPromptSteerReceiptPhase;

    fn receipt(phase: LeasedPromptSteerReceiptPhase) -> WorkerSteerReceiptRecord {
        WorkerSteerReceiptRecord {
            leased_agent_id: "leased-agent-1".to_string(),
            receipt: LeasedPromptSteerReceipt {
                steer_id: "home-steer-1".to_string(),
                target_home_prompt_id: "home-prompt-1".to_string(),
                worker_provider_run_id: "worker-run-1".to_string(),
                execution_lease_id: "lease-1".to_string(),
                phase,
            },
            caller: None,
        }
    }

    #[test]
    fn worker_receipt_restore_preserves_exact_identity_and_rejection_tombstones() {
        let root = std::env::temp_dir().join(format!(
            "chariox-worker-steer-receipts-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms(),
        ));
        let path = root.join("state.db");
        let durable = crate::durable_state::DurableKernelStateStore::open(path.clone())
            .expect("durable state should open");
        let mut store = WorkerSteerReceiptStore::restore(durable.clone())
            .expect("empty receipt state should restore");
        store
            .persist(receipt(LeasedPromptSteerReceiptPhase::Dispatching))
            .expect("dispatching receipt should persist");
        store
            .persist(receipt(LeasedPromptSteerReceiptPhase::Rejected))
            .expect("rejection tombstone should persist");
        let mut wrong_run = receipt(LeasedPromptSteerReceiptPhase::Rejected);
        wrong_run.receipt.worker_provider_run_id = "other-worker-run".to_string();
        assert!(store.persist(wrong_run).is_err());
        let mut wrong_lease = receipt(LeasedPromptSteerReceiptPhase::Rejected);
        wrong_lease.receipt.execution_lease_id = "other-execution-lease".to_string();
        assert!(store.persist(wrong_lease).is_err());
        drop(store);
        drop(durable);

        let reopened = crate::durable_state::DurableKernelStateStore::open(path)
            .expect("durable state should reopen");
        let restored_store = WorkerSteerReceiptStore::restore(reopened.clone())
            .expect("worker restart should restore the receipt");
        let restored = restored_store
            .get("leased-agent-1", "home-steer-1")
            .expect("exact worker receipt should be present");
        assert_eq!(
            restored.receipt.phase,
            LeasedPromptSteerReceiptPhase::Rejected
        );
        assert_eq!(restored.receipt.worker_provider_run_id, "worker-run-1");
        assert_eq!(restored.receipt.execution_lease_id, "lease-1");
        drop(restored_store);
        drop(reopened);
        let _ = std::fs::remove_dir_all(root);
    }
}
