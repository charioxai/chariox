use super::*;

impl KernelRuntimeState {
    pub(crate) fn provider_runs_for_external_session_attachment(
        &self,
    ) -> Vec<crate::provider::RuntimeProviderRun> {
        self.owned.provider_store.list_runs()
    }

    pub(crate) fn provider_run_response(
        &self,
        request: crate::local::GetProviderRunRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.owned
            .provider_store
            .apply_finished_provider_run_selection_sync_jobs();
        self.owned
            .provider_store
            .enqueue_run_selection_sync(&request.provider_run_id)?;
        let provider_run = self
            .owned
            .provider_store
            .get_run(&request.provider_run_id)?;
        self.owned
            .provider_run_projection
            .update(provider_run.clone());
        Ok(LocalDaemonResponse::ProviderRun { provider_run })
    }

    pub(crate) fn update_provider_run_selection_response(
        &self,
        request: crate::local::UpdateProviderRunSelectionRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let run = self
            .owned
            .provider_store
            .get_run(&request.provider_run_id)?;
        if run.session_id() != request.session_id {
            return Err(DaemonError::ProviderRunNotInSession {
                session_id: request.session_id,
                provider_run_id: request.provider_run_id,
            });
        }
        let provider_run = self.owned.provider_store.update_run_selection(
            &request.provider_run_id,
            request.model,
            request.variant,
            request.clear_variant,
        )?;
        self.owned
            .provider_run_projection
            .update(provider_run.clone());
        Ok(LocalDaemonResponse::ProviderRunSelectionUpdated { provider_run })
    }
}
