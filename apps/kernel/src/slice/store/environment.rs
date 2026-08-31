use super::*;

impl SliceStore {
    pub(crate) fn environment_slice(&self, session_id: &str) -> Option<SliceRecord> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .records
            .values()
            .find(|slice| slice.environment_session_id.as_deref() == Some(session_id))
            .cloned()
    }

    /// Commit the reservation before publishing it. Claims across Room lanes
    /// share this lock, so a physical profile can never have two owners.
    pub(crate) fn bind_environment(
        &self,
        session_id: &str,
        slice_ref: &str,
        now_ms: u64,
        persist: impl FnOnce(&SliceRecord) -> Result<(), DaemonError>,
    ) -> Result<SliceRecord, DaemonError> {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut matches = state
            .records
            .values()
            .filter(|slice| slice.id == slice_ref.trim() || slice.name == slice_ref.trim());
        let slice = matches
            .next()
            .ok_or_else(|| binding_error("unknown slice"))?;
        if matches.next().is_some() {
            return Err(binding_error("ambiguous slice reference"));
        }
        if slice.display_mode != super::super::SliceDisplayMode::Headed {
            return Err(binding_error("Room Environment requires a headed slice"));
        }
        if state.records.values().any(|other| {
            other.id != slice.id
                && worker_refs(other).any(|left| worker_refs(slice).any(|right| left == right))
        }) {
            return Err(binding_error(
                "slice worker reference is shared by another slice",
            ));
        }
        if state.records.values().any(|other| {
            other.environment_session_id.as_deref() == Some(session_id) && other.id != slice.id
        }) {
            return Err(binding_error(
                "Room already has a different Environment slice",
            ));
        }
        if slice
            .environment_session_id
            .as_deref()
            .is_some_and(|owner| owner != session_id)
            || slice
                .session_id
                .as_deref()
                .is_some_and(|owner| owner != session_id)
            || slice.session_ids.iter().any(|owner| owner != session_id)
        {
            return Err(binding_error("slice belongs to another Room"));
        }
        if slice.environment_session_id.as_deref() == Some(session_id) {
            return Ok(slice.clone());
        }
        if state.active_operations.contains_key(&slice.id) {
            return Err(binding_error(
                "slice operation in progress; retry after it completes",
            ));
        }
        let mut updated = slice.clone();
        updated.environment_session_id = Some(session_id.to_string());
        updated.updated_at_ms = now_ms;
        persist(&updated)?;
        state.records.insert(updated.id.clone(), updated.clone());
        Ok(updated)
    }
}

fn binding_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "environment.slice.bind",
        message: format!("environment_slice_binding_rejected: {message}"),
    }
}

fn worker_refs(slice: &SliceRecord) -> impl Iterator<Item = &str> {
    // These are worker routing identities, not the Docker host identity.
    // Local Docker assigns worker machine_id = "slice:<slice.id>"; co-located
    // containers share owner_machine_id instead. A duplicate worker machine ID
    // is ambiguous because agent placement also accepts it as a target.
    std::iter::once(slice.worker_kernel_ref.as_str())
        .chain(slice.worker_kernel_id.as_deref())
        .chain(slice.worker_machine_id.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}
