use super::*;

impl KernelRuntimeState {
    pub(crate) fn provider_runs_for_external_session_attachment(
        &self,
    ) -> Vec<crate::provider::RuntimeProviderRun> {
        self.owned.provider_store.list_runs()
    }

    /// The read handler schedules a selection sync and publishes the run, so a caller who may
    /// not see the run is refused before it runs (the projected read applies the same check).
    pub(crate) fn ensure_provider_run_readable(
        &self,
        provider_run_id: &str,
        caller_user_id: &str,
        authenticated_user_id: &str,
    ) -> Result<(), DaemonError> {
        let provider_run = match self.owned.provider_store.get_run(provider_run_id) {
            Ok(provider_run) => provider_run,
            // A missing run is the handler's `provider run not found`.
            Err(DaemonError::ProviderRunNotFound { .. }) => return Ok(()),
            Err(error) => return Err(error),
        };
        crate::runtime::provider_run_control::ensure_provider_run_visible_to_user(
            &provider_run,
            caller_user_id,
            authenticated_user_id,
        )
    }

    pub(crate) fn provider_run_response(
        &self,
        request: crate::local::GetProviderRunRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let mut provider_run = self
            .owned
            .provider_store
            .get_run(&request.provider_run_id)?;
        let session = self
            .owned
            .session_store
            .get_session(provider_run.session_id())?;
        if provider_run_selection_sync_allowed(&session) {
            self.owned
                .provider_store
                .apply_finished_provider_run_selection_sync_jobs();
            self.owned
                .provider_store
                .enqueue_run_selection_sync(&request.provider_run_id)?;
            provider_run = self
                .owned
                .provider_store
                .get_run(&request.provider_run_id)?;
        }
        self.owned
            .provider_run_projection
            .update(provider_run.clone());
        Ok(LocalDaemonResponse::ProviderRun {
            provider_run: provider_run.into(),
        })
    }

    pub(crate) fn update_provider_run_selection_response(
        &self,
        caller_user_id: &str,
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
        // The selection is the model the run's owner pays for, so only who acts for that owner
        // changes it (the `ensure_agent_owner` rule).
        if !self
            .owned
            .acts_for_owner(run.session_id(), caller_user_id, run.owner_user_id())
        {
            return Err(DaemonError::OwnershipAccessDenied {
                user_id: caller_user_id.to_string(),
                owner_user_id: run.owner_user_id().to_string(),
                resource: format!("provider run `{}`", run.id()),
                operation: "update provider run selection",
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
        Ok(LocalDaemonResponse::ProviderRunSelectionUpdated {
            provider_run: provider_run.into(),
        })
    }
}

fn provider_run_selection_sync_allowed(session: &crate::session::RuntimeSession) -> bool {
    provider_run_selection_sync_allowed_for_state(
        session.is_hidden(),
        !session.workflow_publications().is_empty(),
    )
}

fn provider_run_selection_sync_allowed_for_state(
    is_hidden: bool,
    has_workflow_publications: bool,
) -> bool {
    !(is_hidden && has_workflow_publications)
}

#[cfg(test)]
mod tests {
    use super::provider_run_selection_sync_allowed_for_state;

    #[test]
    fn publication_runtime_reads_do_not_schedule_provider_selection_writes() {
        assert!(!provider_run_selection_sync_allowed_for_state(true, true));
        assert!(provider_run_selection_sync_allowed_for_state(false, true));
        assert!(provider_run_selection_sync_allowed_for_state(true, false));
        assert!(provider_run_selection_sync_allowed_for_state(false, false));
    }
}
