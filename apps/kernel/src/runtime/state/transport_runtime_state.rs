use super::*;

impl KernelRuntimeState {
    pub(crate) fn durable_snapshot_scheduler(
        &self,
    ) -> Option<crate::durable_snapshot::DurableSnapshotScheduler> {
        let interval_events = self
            .owned
            .config_projection
            .snapshot()
            .user_config
            .state
            .snapshot_interval_events? as u64;
        Some(crate::durable_snapshot::DurableSnapshotScheduler::new(
            self.owned.durable_state_store.clone(),
            self.owned.session_store.clone(),
            self.owned.agent_store.clone(),
            self.owned.slice_store.clone(),
            interval_events,
        ))
    }

    pub(crate) async fn pump_transport_runtime(&self) {
        self.with_app_side_effect(|app| {
            crate::app::provider_output::pump_active_prompt_outputs(app);
            crate::app::workflow_runtime::pump_workflow_watchdogs(app);
        })
        .await;
    }

    pub(crate) async fn shutdown_cleanup(&self) -> Result<(), DaemonError> {
        self.with_app_side_effect(|app| app.shutdown_cleanup())
            .await
    }

    pub(crate) fn session_snapshot_projection(
        &self,
        session_id: &str,
        last_event_id: u64,
    ) -> Result<crate::runtime::projection::SessionSnapshotProjection, DaemonError> {
        let session = self.owned.session_snapshot(session_id)?;
        let provider_run = session
            .active_provider_run_id()
            .and_then(|provider_run_id| {
                self.owned
                    .provider_store
                    .get_run(provider_run_id)
                    .ok()
                    .or_else(|| self.owned.provider_run_projection.get(provider_run_id))
            });
        let agent_activity = self.agent_activity_for_session(&session);
        Ok(crate::runtime::projection::SessionSnapshotProjection {
            metadata: crate::runtime::projection::ProjectionMetadata::new(2, last_event_id),
            session,
            provider_run,
            agent_activity,
        })
    }
}
