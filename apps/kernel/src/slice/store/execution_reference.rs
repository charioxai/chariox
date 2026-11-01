use super::*;

impl SliceStore {
    /// Resolve friendly input only for guarded execution. Authenticated peer and
    /// token checks continue to use exact worker identity lookup.
    pub(crate) fn resolve_execution_worker_kernel_ref(
        &self,
        kernel_ref: &str,
        operation: &'static str,
    ) -> Result<Option<SliceRecord>, DaemonError> {
        let kernel_ref = kernel_ref.trim();
        if kernel_ref.is_empty() {
            return Ok(None);
        }
        let state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut matches = state.records.values().filter(|record| {
            record.worker_kernel_ref == kernel_ref
                || record.worker_kernel_id.as_deref() == Some(kernel_ref)
                || (record.worker_machine_id.as_deref() == Some(kernel_ref)
                    && record.worker_machine_id.as_deref()
                        != Some(record.owner_machine_id.as_str()))
                || format!("slice:{}", record.name) == kernel_ref
        });
        let record = matches.next();
        if matches.next().is_some() {
            return Err(DaemonError::LocalTransport {
                operation,
                message: format!("environment_slice_access_denied: ambiguous slice execution reference `{kernel_ref}`; worker reference is shared by another slice"),
            });
        }
        if record.is_none() && kernel_ref.starts_with("slice:") {
            return Err(DaemonError::LocalTransport {
                operation,
                message: format!("environment_slice_access_denied: slice execution reference `{kernel_ref}` has no local physical slice record; use a home-managed slice or an ordinary remote Kernel reference"),
            });
        }
        Ok(record.cloned())
    }
}

impl SliceStore {
    /// Resolve `agent spawn --slice` for an agent that may itself run in a
    /// slice. Such an agent learns its placement from scoped discovery as the
    /// worker kernel ID, not as the slice id or name, so accept that form too
    /// and otherwise name the slice the caller actually runs in.
    pub(crate) fn resolve_spawn_slice(
        &self,
        reference: &str,
        caller_agent_id: &str,
    ) -> Result<SliceRecord, DaemonError> {
        match self.resolve(reference) {
            Err(DaemonError::LocalTransport { message, .. })
                if message.starts_with("unknown slice") => {}
            resolved => return resolved,
        }
        if let Some(slice) = self.resolve_by_worker_kernel_ref(reference) {
            return Ok(slice);
        }
        let caller_slice = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .records
            .values()
            .find(|record| record.agent_ids.iter().any(|id| id == caller_agent_id))
            .map(|record| record.name.clone());
        let hint = match caller_slice {
            Some(name) => format!(
                "this agent runs in slice `{name}`; place the child there with `--slice {name}`"
            ),
            None => "use a slice id or name from `slice list`".to_string(),
        };
        Err(DaemonError::LocalTransport {
            operation: "slice.resolve",
            message: format!("unknown slice `{}`: {hint}", reference.trim()),
        })
    }
}
