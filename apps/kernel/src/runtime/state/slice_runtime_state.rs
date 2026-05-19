use super::*;

impl KernelRuntimeState {
    pub(crate) fn list_slices(&self) -> Vec<crate::slice::SliceRecord> {
        self.owned.slice_store.list()
    }

    pub(crate) async fn create_slice(
        &self,
        request: crate::local::CreateSliceRequest,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        let config = self.owned.config_projection.snapshot();
        let slice = self.owned.slice_store.create(
            &config.daemon_id,
            &config.host_machine_id,
            crate::slice::CreateSliceInput {
                name: request.name,
                backend: request.backend,
                os: request.os,
                workspace_mount: request.workspace_mount,
                worker_kernel_ref: request.worker_kernel_ref,
                display_url: request.display_url,
                now_ms: crate::session::unix_epoch_ms(),
            },
        )?;
        self.append_slice_durable_event("slice.created", &slice)?;
        Ok(slice)
    }

    pub(crate) fn resolve_slice(
        &self,
        slice_ref: &str,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        self.owned.slice_store.resolve(slice_ref)
    }

    pub(crate) fn mark_slice_starting(
        &self,
        slice_ref: &str,
        relay_endpoint: crate::slice::SliceRelayEndpoint,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        self.owned.slice_store.set_relay_endpoint(
            slice_ref,
            Some(relay_endpoint),
            crate::session::unix_epoch_ms(),
        )?;
        let slice = self.owned.slice_store.set_status(
            slice_ref,
            crate::slice::SliceStatus::Starting,
            crate::session::unix_epoch_ms(),
        )?;
        self.append_slice_durable_event("slice.updated", &slice)?;
        Ok(slice)
    }

    pub(crate) fn set_slice_status(
        &self,
        slice_ref: &str,
        status: crate::slice::SliceStatus,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        let slice = self.owned.slice_store.set_status(
            slice_ref,
            status,
            crate::session::unix_epoch_ms(),
        )?;
        self.append_slice_durable_event("slice.updated", &slice)?;
        Ok(slice)
    }

    pub(crate) fn mark_slice_running(
        &self,
        slice_ref: &str,
        worker: Option<arroba_relay::protocol::RelayKernelPresence>,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        let mut slice = self.owned.slice_store.set_status(
            slice_ref,
            crate::slice::SliceStatus::Running,
            crate::session::unix_epoch_ms(),
        )?;
        if let Some(worker) = worker {
            slice = self.owned.slice_store.set_worker_presence(
                slice_ref,
                Some(worker.kernel_id),
                Some(worker.machine_id),
                worker.available_providers,
                crate::session::unix_epoch_ms(),
            )?;
        }
        self.append_slice_durable_event("slice.updated", &slice)?;
        Ok(slice)
    }

    pub(crate) fn delete_slice(
        &self,
        slice_ref: &str,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        let slice = self.owned.slice_store.delete(slice_ref)?;
        self.append_slice_durable_event("slice.deleted", &slice)?;
        Ok(slice)
    }

    pub(crate) fn slice_display_endpoint(
        &self,
        slice_ref: &str,
    ) -> Result<crate::slice::SliceDisplayEndpoint, DaemonError> {
        self.owned.slice_store.display_endpoint(slice_ref)
    }

    fn append_slice_durable_event(
        &self,
        kind: &'static str,
        slice: &crate::slice::SliceRecord,
    ) -> Result<(), DaemonError> {
        self.owned.durable_state_store.append_event(
            kind,
            Some(slice.id.clone()),
            serde_json::json!({ "slice": slice }),
        )?;
        Ok(())
    }
}
