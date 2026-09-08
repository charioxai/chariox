use super::*;

impl SliceStore {
    pub(crate) fn upsert_saved_state_after_commit(
        &self,
        slice_ref: &str,
        saved_state: SliceSavedStateRecord,
        now_ms: u64,
        commit: impl FnOnce(&SliceSavedStateRecord, &SliceRecord) -> Result<(), DaemonError>,
    ) -> Result<SliceRecord, DaemonError> {
        let resolved = self.resolve(slice_ref)?;
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut record = state.records.get(&resolved.id).cloned().ok_or_else(|| {
            DaemonError::LocalTransport {
                operation: "slice.state.save",
                message: format!("unknown slice `{slice_ref}`"),
            }
        })?;
        record.saved_state_ref = Some(saved_state.id.clone());
        record.saved_state_status = Some(SliceSavedStateStatus::Saved);
        record.saved_state_updated_at_ms = Some(now_ms);
        record.last_operation = Some("state.save".to_string());
        record.last_operation_status = Some(SliceOperationStatus::Completed);
        record.last_error = None;
        record.last_operation_at_ms = Some(now_ms);
        record.updated_at_ms = now_ms;
        // Keep this lock through the commit, so no reader sees an unpublished
        // reference and no peer update is overwritten by a stale staged record.
        // The durable writer executes SQL only; it never calls SliceStore.
        commit(&saved_state, &record)?;
        state
            .saved_states
            .insert(saved_state.id.clone(), saved_state);
        state.records.insert(record.id.clone(), record.clone());
        Ok(record)
    }
}
