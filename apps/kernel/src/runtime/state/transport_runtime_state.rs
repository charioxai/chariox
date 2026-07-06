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
            self.owned.metaagent_events.clone(),
            interval_events,
        ))
    }

    pub(crate) async fn pump_transport_runtime(&self) {
        let pumped_provider_run_ids = self
            .with_app_side_effect(|app| {
                let pumped_provider_run_ids =
                    crate::app::provider_output::pump_active_prompt_outputs(app);
                pumped_provider_run_ids
            })
            .await;
        let watchdog_dispatches = self
            .owned
            .workflow_collect_due_watchdog_dispatches(crate::session::unix_epoch_ms());
        self.spawn_workflow_prompt_dispatches(watchdog_dispatches);
        for provider_run_id in pumped_provider_run_ids {
            self.observe_git_after_provider_activity_if_pending(&provider_run_id)
                .await;
        }
    }

    pub(crate) fn terminal_session_change_sequence(&self, session_id: &str) -> u64 {
        self.owned
            .terminal_stream
            .session_change_sequence(session_id)
    }

    pub(crate) fn terminal_attachment_change_sequence(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> u64 {
        self.owned
            .terminal_stream
            .attachment_change_sequence(session_id, attachment_id)
    }

    pub(crate) async fn wait_for_terminal_session_change_after(
        &self,
        session_id: &str,
        sequence: u64,
    ) {
        self.owned
            .terminal_stream
            .wait_for_session_change_after(session_id, sequence)
            .await;
    }

    pub(crate) async fn wait_for_terminal_attachment_change_after(
        &self,
        session_id: &str,
        attachment_id: &str,
        sequence: u64,
    ) {
        self.owned
            .terminal_stream
            .wait_for_attachment_change_after(session_id, attachment_id, sequence)
            .await;
    }

    pub(crate) fn waiting_room_change_sequence(&self) -> u64 {
        self.owned.runtime_projection_changes.sequence()
    }

    pub(crate) async fn wait_for_waiting_room_change_after(&self, sequence: u64) {
        self.owned
            .runtime_projection_changes
            .wait_for_change_after(sequence)
            .await;
    }

    pub(crate) fn session_projection_change_sequence(&self) -> u64 {
        self.owned.session_projection.change_sequence()
    }

    pub(crate) fn session_projection_session_change_sequence(&self, session_id: &str) -> u64 {
        self.owned
            .session_projection
            .session_change_sequence(session_id)
    }

    pub(crate) async fn wait_for_session_projection_change_after(&self, sequence: u64) {
        self.owned
            .session_projection
            .wait_for_change_after(sequence)
            .await;
    }

    pub(crate) async fn wait_for_session_projection_session_change_after(
        &self,
        session_id: &str,
        sequence: u64,
    ) {
        self.owned
            .session_projection
            .wait_for_session_change_after(session_id, sequence)
            .await;
    }

    pub(crate) fn workflow_design_change_sequence(&self) -> u64 {
        self.owned.workflow_design_events.change_sequence()
    }

    pub(crate) async fn wait_for_workflow_design_change_after(&self, sequence: u64) {
        self.owned
            .workflow_design_events
            .wait_for_change_after(sequence)
            .await;
    }

    pub(crate) fn transport_runtime_pump_change_sequence(&self) -> u64 {
        self.owned.runtime_projection_changes.sequence()
    }

    pub(crate) async fn wait_for_transport_runtime_pump_change_after(&self, sequence: u64) {
        self.owned
            .runtime_projection_changes
            .wait_for_change_after(sequence)
            .await;
    }

    pub(crate) fn pty_output_change_sequence(&self) -> u64 {
        self.owned.pty_output_signal.sequence()
    }

    pub(crate) async fn wait_for_pty_output_change_after(&self, sequence: u64) {
        self.owned
            .pty_output_signal
            .wait_for_change_after(sequence)
            .await;
    }

    pub(crate) fn provider_run_actor_completion_sequence(&self) -> u64 {
        self.owned
            .provider_store
            .run_actor_completion_signal()
            .sequence()
    }

    pub(crate) async fn wait_for_provider_run_actor_completion_after(&self, sequence: u64) {
        self.owned
            .provider_store
            .run_actor_completion_signal()
            .wait_for_change_after(sequence)
            .await;
    }

    pub(crate) fn transport_runtime_pump_interval_ms(
        &self,
        active_interval_ms: u64,
        idle_interval_ms: u64,
        now_ms: u64,
    ) -> u64 {
        transport_runtime_pump_interval_for_state(
            self.next_structured_output_poll_due_at_ms(),
            self.next_workflow_watchdog_run_at_ms(now_ms),
            now_ms,
            active_interval_ms,
            idle_interval_ms,
        )
    }

    fn next_workflow_watchdog_run_at_ms(&self, now_ms: u64) -> Option<u64> {
        self.owned
            .session_store
            .list_non_ended_sessions_including_hidden()
            .iter()
            .flat_map(|session| session.workflow_watchdogs())
            .filter(|watchdog| {
                watchdog.enabled()
                    && !watchdog
                        .max_wakeups()
                        .is_some_and(|limit| watchdog.wakeups_executed() >= limit)
            })
            .map(|watchdog| {
                if watchdog.pending_run() {
                    now_ms
                } else {
                    watchdog.next_run_at_ms()
                }
            })
            .min()
    }

    fn next_structured_output_poll_due_at_ms(&self) -> Option<u64> {
        self.owned
            .session_store
            .list_non_ended_sessions_including_hidden()
            .iter()
            .flat_map(|session| {
                super::provider_output_runtime::provider_run_ids_for_owned_output_pump(
                    &self.owned,
                    session,
                )
            })
            .filter_map(|provider_run_id| {
                let run = self.owned.provider_store.get_run(&provider_run_id).ok()?;
                if !run.client_interface().is_arroba()
                    || !self
                        .owned
                        .provider_store
                        .run_uses_structured_prompt_io(&run)
                {
                    return None;
                }
                self.owned
                    .structured_output_records
                    .poll_due_at_ms(&provider_run_id)
            })
            .min()
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
        self.session_snapshot_projection_for_user(session_id, last_event_id, None)
    }

    fn session_snapshot_projection_for_user(
        &self,
        session_id: &str,
        last_event_id: u64,
        unread_for_user_id: Option<&str>,
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
        let agent_activity =
            self.agent_activity_for_session_with_unread(&session, unread_for_user_id);
        Ok(crate::runtime::projection::SessionSnapshotProjection {
            metadata: crate::runtime::projection::ProjectionMetadata::new(
                crate::runtime::projection::SESSION_SNAPSHOT_PROJECTION_VERSION,
                last_event_id,
            ),
            session,
            provider_run,
            agent_activity,
        })
    }

    fn read_only_session_snapshot_projection(
        &self,
        session_id: &str,
        last_event_id: u64,
    ) -> Result<crate::runtime::projection::SessionSnapshotProjection, DaemonError> {
        self.read_only_session_snapshot_projection_for_user(session_id, last_event_id, None)
    }

    fn read_only_session_snapshot_projection_for_user(
        &self,
        session_id: &str,
        last_event_id: u64,
        unread_for_user_id: Option<&str>,
    ) -> Result<crate::runtime::projection::SessionSnapshotProjection, DaemonError> {
        let mut session = self.owned.session_store.get_session(session_id)?;
        let agents = self.owned.agent_store.get_session_agents(session_id);
        session.set_agents(agents);
        self.owned.project_session_runtime_view(&mut session);
        let provider_run = session
            .active_provider_run_id()
            .and_then(|provider_run_id| {
                self.owned
                    .provider_store
                    .get_run(provider_run_id)
                    .ok()
                    .or_else(|| self.owned.provider_run_projection.get(provider_run_id))
            });
        let agent_activity =
            self.agent_activity_for_session_with_unread(&session, unread_for_user_id);
        Ok(crate::runtime::projection::SessionSnapshotProjection {
            metadata: crate::runtime::projection::ProjectionMetadata::new(
                crate::runtime::projection::SESSION_SNAPSHOT_PROJECTION_VERSION,
                last_event_id,
            ),
            session,
            provider_run,
            agent_activity,
        })
    }

    pub(crate) fn session_snapshot_projection_for_attachment(
        &self,
        session_id: &str,
        attachment_id: &str,
        last_event_id: u64,
    ) -> Result<crate::runtime::projection::SessionSnapshotProjection, DaemonError> {
        let attachment = self.owned.attachment_store.get_attachment(attachment_id)?;
        if attachment.session_id() != session_id {
            return Err(DaemonError::AttachmentNotInSession {
                session_id: session_id.to_string(),
                attachment_id: attachment_id.to_string(),
            });
        }
        let mut projection = self.session_snapshot_projection_for_user(
            session_id,
            last_event_id,
            Some(attachment.owner_user_id()),
        )?;
        projection.session = projection
            .session
            .redacted_for_user(attachment.owner_user_id());
        projection.agent_activity.retain(|agent_id, _| {
            projection
                .session
                .agents()
                .iter()
                .any(|agent| agent.id() == agent_id)
        });
        if projection
            .provider_run
            .as_ref()
            .and_then(|run| run.agent_instance_id())
            .is_some_and(|agent_id| {
                !projection
                    .session
                    .agents()
                    .iter()
                    .any(|agent| agent.id() == agent_id)
            })
        {
            projection.provider_run = None;
        }
        Ok(projection)
    }

    pub(crate) fn read_only_session_snapshot_projection_for_attachment(
        &self,
        session_id: &str,
        attachment_id: &str,
        last_event_id: u64,
    ) -> Result<crate::runtime::projection::SessionSnapshotProjection, DaemonError> {
        let attachment = self.owned.attachment_store.get_attachment(attachment_id)?;
        if attachment.session_id() != session_id {
            return Err(DaemonError::AttachmentNotInSession {
                session_id: session_id.to_string(),
                attachment_id: attachment_id.to_string(),
            });
        }
        let mut projection = self.read_only_session_snapshot_projection_for_user(
            session_id,
            last_event_id,
            Some(attachment.owner_user_id()),
        )?;
        projection.session = projection
            .session
            .redacted_for_user(attachment.owner_user_id());
        projection.agent_activity.retain(|agent_id, _| {
            projection
                .session
                .agents()
                .iter()
                .any(|agent| agent.id() == agent_id)
        });
        if projection
            .provider_run
            .as_ref()
            .and_then(|run| run.agent_instance_id())
            .is_some_and(|agent_id| {
                !projection
                    .session
                    .agents()
                    .iter()
                    .any(|agent| agent.id() == agent_id)
            })
        {
            projection.provider_run = None;
        }
        Ok(projection)
    }
}

fn transport_runtime_pump_interval_for_state(
    next_structured_output_poll_due_at_ms: Option<u64>,
    next_watchdog_run_at_ms: Option<u64>,
    now_ms: u64,
    minimum_interval_ms: u64,
    idle_interval_ms: u64,
) -> u64 {
    let fallback_interval_ms = idle_interval_ms;
    let next_due_at_ms = [
        next_structured_output_poll_due_at_ms,
        next_watchdog_run_at_ms,
    ]
    .into_iter()
    .flatten()
    .min();
    let Some(next_due_at_ms) = next_due_at_ms else {
        return fallback_interval_ms;
    };
    if next_due_at_ms <= now_ms {
        return minimum_interval_ms;
    }
    next_due_at_ms
        .saturating_sub(now_ms)
        .max(minimum_interval_ms)
        .min(fallback_interval_ms)
        .min(idle_interval_ms)
}

#[cfg(test)]
mod tests {
    use super::transport_runtime_pump_interval_for_state;

    #[test]
    fn transport_runtime_pump_interval_uses_idle_sweep_without_active_work() {
        assert_eq!(
            transport_runtime_pump_interval_for_state(None, None, 10_000, 500, 5_000),
            5_000,
        );
    }

    #[test]
    fn transport_runtime_pump_interval_uses_coarse_sweep_without_due_work() {
        assert_eq!(
            transport_runtime_pump_interval_for_state(None, None, 10_000, 500, 5_000),
            5_000,
        );
    }

    #[test]
    fn transport_runtime_pump_interval_tracks_due_watchdogs() {
        assert_eq!(
            transport_runtime_pump_interval_for_state(None, Some(10_250), 10_000, 500, 5_000),
            500,
        );
        assert_eq!(
            transport_runtime_pump_interval_for_state(None, Some(12_000), 10_000, 500, 5_000),
            2_000,
        );
    }

    #[test]
    fn transport_runtime_pump_interval_tracks_structured_output_poll_due_time() {
        assert_eq!(
            transport_runtime_pump_interval_for_state(Some(10_250), None, 10_000, 500, 5_000),
            500,
        );
        assert_eq!(
            transport_runtime_pump_interval_for_state(Some(12_000), None, 10_000, 500, 5_000),
            2_000,
        );
        assert_eq!(
            transport_runtime_pump_interval_for_state(Some(9_999), None, 10_000, 500, 5_000),
            500,
        );
    }
}
