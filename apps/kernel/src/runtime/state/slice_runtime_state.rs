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
                display_mode: request.display_mode,
                workspace_id: request.workspace_id,
                worktree_id: request.worktree_id,
                workspace_mount: request.workspace_mount,
                worker_kernel_ref: request.worker_kernel_ref,
                display_url: request.display_url,
                provider_auth: Vec::new(),
                now_ms: crate::session::unix_epoch_ms(),
            },
        )?;
        self.append_slice_durable_event("slice.created", &slice)?;
        self.record_slice_audit_event(&slice, "create", "completed", None, None)?;
        Ok(slice)
    }

    pub(crate) fn resolve_slice(
        &self,
        slice_ref: &str,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        self.owned.slice_store.resolve(slice_ref)
    }

    pub(crate) fn begin_slice_operation(
        &self,
        slice_ref: &str,
        operation: &'static str,
    ) -> Result<crate::slice::SliceOperationGuard, DaemonError> {
        self.owned
            .slice_store
            .try_begin_operation(slice_ref, operation)
    }

    pub(crate) fn record_slice_audit_event(
        &self,
        slice: &crate::slice::SliceRecord,
        action: &'static str,
        outcome: &'static str,
        provider: Option<&str>,
        message: Option<&str>,
    ) -> Result<(), DaemonError> {
        self.owned.durable_state_store.append_event(
            "slice.audit",
            Some(slice.id.clone()),
            serde_json::json!({
                "slice_id": slice.id,
                "slice_name": slice.name,
                "action": action,
                "outcome": outcome,
                "result": outcome,
                "actor": "kernel",
                "client_type": "local_daemon",
                "provider": provider,
                "message": message,
                "redacted_error": if outcome == "failed" { message } else { None },
                "status": slice.status,
                "backend": slice.backend,
                "display_mode": slice.display_mode,
                "workspace_id": slice.workspace_id,
                "worktree_id": slice.worktree_id,
                "workspace_mount": slice.workspace_mount,
                "session_ids": slice.session_ids,
                "agent_ids": slice.agent_ids,
                "owner_kernel_id": slice.owner_kernel_id,
                "owner_machine_id": slice.owner_machine_id,
                "worker_kernel_ref": slice.worker_kernel_ref,
                "worker_kernel_id": slice.worker_kernel_id,
                "worker_machine_id": slice.worker_machine_id,
                "at_ms": crate::session::unix_epoch_ms(),
            }),
        )?;
        Ok(())
    }

    pub(crate) fn mark_slice_starting(
        &self,
        slice_ref: &str,
        relay_endpoint: crate::slice::SliceRelayEndpoint,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        self.update_slice_operation(
            slice_ref,
            "start",
            crate::slice::SliceOperationStatus::InProgress,
            None,
        )?;
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

    pub(crate) fn mark_slice_stopping(
        &self,
        slice_ref: &str,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        self.update_slice_operation(
            slice_ref,
            "stop",
            crate::slice::SliceOperationStatus::InProgress,
            None,
        )?;
        let slice = self.owned.slice_store.set_status(
            slice_ref,
            crate::slice::SliceStatus::Stopping,
            crate::session::unix_epoch_ms(),
        )?;
        self.append_slice_durable_event("slice.updated", &slice)?;
        Ok(slice)
    }

    pub(crate) fn mark_slice_operation_failed(
        &self,
        slice_ref: &str,
        operation: &'static str,
        error: &DaemonError,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        self.owned.slice_store.set_status(
            slice_ref,
            crate::slice::SliceStatus::Unhealthy,
            crate::session::unix_epoch_ms(),
        )?;
        let slice = self.owned.slice_store.set_operation_diagnostics(
            slice_ref,
            operation,
            crate::slice::SliceOperationStatus::Failed,
            Some(&error.to_string()),
            crate::session::unix_epoch_ms(),
        )?;
        self.append_slice_durable_event("slice.updated", &slice)?;
        Ok(slice)
    }

    pub(crate) fn mark_slice_operation_rejected(
        &self,
        slice_ref: &str,
        operation: &'static str,
        error: &DaemonError,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        let slice = self.update_slice_operation(
            slice_ref,
            operation,
            crate::slice::SliceOperationStatus::Failed,
            Some(&error.to_string()),
        )?;
        self.append_slice_durable_event("slice.updated", &slice)?;
        Ok(slice)
    }

    pub(crate) fn mark_slice_running(
        &self,
        slice_ref: &str,
        worker: Option<arroba_relay::protocol::RelayKernelPresence>,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        self.owned.slice_store.set_status(
            slice_ref,
            crate::slice::SliceStatus::Running,
            crate::session::unix_epoch_ms(),
        )?;
        if let Some(worker) = worker {
            self.owned.slice_store.set_worker_presence(
                slice_ref,
                Some(worker.kernel_id),
                Some(worker.machine_id),
                worker.available_providers,
                crate::session::unix_epoch_ms(),
            )?;
        }
        let slice = self.owned.slice_store.set_operation_diagnostics(
            slice_ref,
            "start",
            crate::slice::SliceOperationStatus::Completed,
            None,
            crate::session::unix_epoch_ms(),
        )?;
        self.append_slice_durable_event("slice.updated", &slice)?;
        Ok(slice)
    }

    pub(crate) fn mark_slice_stopped(
        &self,
        slice_ref: &str,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        self.owned.slice_store.set_status(
            slice_ref,
            crate::slice::SliceStatus::Stopped,
            crate::session::unix_epoch_ms(),
        )?;
        let slice = self.owned.slice_store.set_operation_diagnostics(
            slice_ref,
            "stop",
            crate::slice::SliceOperationStatus::Completed,
            None,
            crate::session::unix_epoch_ms(),
        )?;
        self.append_slice_durable_event("slice.updated", &slice)?;
        Ok(slice)
    }

    pub(crate) fn mark_slice_delete_in_progress(
        &self,
        slice_ref: &str,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        let slice = self.update_slice_operation(
            slice_ref,
            "delete",
            crate::slice::SliceOperationStatus::InProgress,
            None,
        )?;
        self.append_slice_durable_event("slice.updated", &slice)?;
        Ok(slice)
    }

    pub(crate) fn mark_slice_delete_failed(
        &self,
        slice_ref: &str,
        error: &DaemonError,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        let slice = self.update_slice_operation(
            slice_ref,
            "delete",
            crate::slice::SliceOperationStatus::Failed,
            Some(&error.to_string()),
        )?;
        self.append_slice_durable_event("slice.updated", &slice)?;
        Ok(slice)
    }

    pub(crate) fn set_slice_provider_auth(
        &self,
        slice_ref: &str,
        provider_auth: Vec<crate::slice_provider_auth::SliceProviderAuthSummary>,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        let slice = self.owned.slice_store.set_provider_auth(
            slice_ref,
            provider_auth,
            crate::session::unix_epoch_ms(),
        )?;
        self.append_slice_durable_event("slice.updated", &slice)?;
        Ok(slice)
    }

    pub(crate) fn set_slice_provider_auth_alias(
        &self,
        slice_ref: &str,
        provider: &str,
        alias: Option<&str>,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        let slice = self.owned.slice_store.set_provider_auth_alias(
            slice_ref,
            provider,
            alias,
            crate::session::unix_epoch_ms(),
        )?;
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

    fn update_slice_operation(
        &self,
        slice_ref: &str,
        operation: &'static str,
        status: crate::slice::SliceOperationStatus,
        error: Option<&str>,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        self.owned.slice_store.set_operation_diagnostics(
            slice_ref,
            operation,
            status,
            error,
            crate::session::unix_epoch_ms(),
        )
    }

    pub(crate) fn slice_display_endpoint(
        &self,
        slice_ref: &str,
    ) -> Result<crate::slice::SliceDisplayEndpoint, DaemonError> {
        self.owned.slice_store.display_endpoint(slice_ref)
    }

    pub(crate) fn list_slice_audit_events(
        &self,
        slice_ref: &str,
        limit: Option<usize>,
    ) -> Result<Vec<crate::durable_state::DurableStateEvent>, DaemonError> {
        let slice = self.resolve_slice(slice_ref)?;
        let limit = limit.unwrap_or(50);
        self.owned
            .durable_state_store
            .load_subject_events_by_kind(&slice.id, "slice.audit", limit)
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
