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
