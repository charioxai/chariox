//! Saved-state and active-reference publication share one existing writer
//! transaction, so a failed reference write cannot leave a completed checkpoint.
use super::{rand_suffix, unix_epoch_ms, DurableKernelStateStore, DurableWriteOperation};
use crate::error::DaemonError;
use crate::slice::{SliceRecord, SliceSavedStateRecord};

#[derive(Debug)]
pub(super) struct SavedSliceStateWrite {
    event_id: String,
    timestamp_ms: u64,
    state_id: String,
    slice_id: String,
    state_json: String,
    slice_json: String,
}

impl DurableKernelStateStore {
    pub(crate) fn append_saved_slice_state(
        &self,
        state: &SliceSavedStateRecord,
        slice: &SliceRecord,
    ) -> Result<(), DaemonError> {
        let timestamp_ms = unix_epoch_ms();
        let encode = |value| {
            serde_json::to_string(&value).map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.encode_saved_slice_state",
                message: error.to_string(),
            })
        };
        self.writer
            .execute(DurableWriteOperation::SliceSavedState(Box::new(
                SavedSliceStateWrite {
                    event_id: format!("state_evt_{timestamp_ms}_{}", rand_suffix()),
                    timestamp_ms,
                    state_id: state.id.clone(),
                    slice_id: slice.id.clone(),
                    state_json: encode(serde_json::json!({ "state": state }))?,
                    slice_json: encode(serde_json::json!({ "slice": slice }))?,
                },
            )))?;
        Ok(())
    }
}

pub(super) fn write(
    transaction: &rusqlite::Transaction<'_>,
    write: &SavedSliceStateWrite,
) -> rusqlite::Result<u64> {
    for (suffix, kind, subject, payload) in [
        (
            "state",
            "slice.state.saved",
            &write.state_id,
            &write.state_json,
        ),
        (
            "reference",
            "slice.updated",
            &write.slice_id,
            &write.slice_json,
        ),
    ] {
        transaction.execute(
            "INSERT INTO durable_state_events (event_id, kind, subject_id, timestamp_ms, payload_json) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![format!("{}_{suffix}", write.event_id), kind, subject, write.timestamp_ms as i64, payload],
        )?;
    }
    Ok(transaction.last_insert_rowid().max(0) as u64)
}
