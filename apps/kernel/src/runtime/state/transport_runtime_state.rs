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

    pub(crate) fn terminal_stream_change_sequence(&self) -> u64 {
        self.owned.terminal_stream.change_sequence()
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

    pub(crate) async fn wait_for_terminal_stream_change_after(&self, sequence: u64) {
        self.owned
            .terminal_stream
            .wait_for_change_after(sequence)
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

    pub(crate) async fn wait_for_session_projection_change_after(&self, sequence: u64) {
        self.owned
            .session_projection
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

    pub(crate) fn transport_runtime_pump_interval_ms(
        &self,
        active_interval_ms: u64,
        idle_interval_ms: u64,
        now_ms: u64,
    ) -> u64 {
        transport_runtime_pump_interval_for_state(
            self.transport_runtime_has_active_work(),
            self.next_workflow_watchdog_run_at_ms(now_ms),
            now_ms,
            active_interval_ms,
            idle_interval_ms,
        )
    }

    fn transport_runtime_has_active_work(&self) -> bool {
        let sessions = self
            .owned
            .session_store
            .list_non_ended_sessions_including_hidden();
        if sessions.iter().any(|session| session.has_any_prompt_work()) {
            return true;
        }
        self.owned
            .provider_store
            .list_runs()
            .iter()
            .chain(self.owned.provider_run_projection.list().iter())
            .any(provider_run_counts_as_transport_active_work)
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
            metadata: crate::runtime::projection::ProjectionMetadata::new(2, last_event_id),
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
            metadata: crate::runtime::projection::ProjectionMetadata::new(2, last_event_id),
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

fn provider_run_counts_as_transport_active_work(run: &crate::provider::RuntimeProviderRun) -> bool {
    match run.state() {
        crate::provider::ProviderRunState::Starting => true,
        crate::provider::ProviderRunState::Running => !run.client_interface().is_arroba(),
        crate::provider::ProviderRunState::Parked | crate::provider::ProviderRunState::Ended => {
            false
        }
    }
}

fn transport_runtime_pump_interval_for_state(
    active_work: bool,
    next_watchdog_run_at_ms: Option<u64>,
    now_ms: u64,
    active_interval_ms: u64,
    idle_interval_ms: u64,
) -> u64 {
    if active_work {
        return active_interval_ms;
    }
    let Some(next_watchdog_run_at_ms) = next_watchdog_run_at_ms else {
        return idle_interval_ms;
    };
    if next_watchdog_run_at_ms <= now_ms.saturating_add(active_interval_ms) {
        return active_interval_ms;
    }
    next_watchdog_run_at_ms
        .saturating_sub(now_ms)
        .min(idle_interval_ms)
        .max(active_interval_ms)
}

#[cfg(test)]
mod tests {
    use super::{
        provider_run_counts_as_transport_active_work, transport_runtime_pump_interval_for_state,
    };

    #[test]
    fn transport_runtime_pump_interval_uses_idle_sweep_without_active_work() {
        assert_eq!(
            transport_runtime_pump_interval_for_state(false, None, 10_000, 500, 5_000),
            5_000,
        );
    }

    #[test]
    fn transport_runtime_pump_interval_stays_fast_with_active_work() {
        assert_eq!(
            transport_runtime_pump_interval_for_state(true, None, 10_000, 500, 5_000),
            500,
        );
    }

    #[test]
    fn transport_runtime_pump_interval_tracks_due_watchdogs() {
        assert_eq!(
            transport_runtime_pump_interval_for_state(false, Some(10_250), 10_000, 500, 5_000),
            500,
        );
        assert_eq!(
            transport_runtime_pump_interval_for_state(false, Some(12_000), 10_000, 500, 5_000),
            2_000,
        );
    }

    #[test]
    fn idle_arroba_provider_run_is_not_transport_active_work() {
        let request = crate::provider::LaunchProviderRequest::new(
            "session-1",
            "agent-1",
            "opencode",
            "default",
            "model",
        );
        let mut run = crate::provider::RuntimeProviderRun::new(
            "provider-run-1",
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::External,
                process_label: "test-provider".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: std::collections::BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: Some("test-runtime".to_string()),
            },
        );
        run.mark_running();

        assert!(!provider_run_counts_as_transport_active_work(&run));
    }

    #[test]
    fn starting_arroba_provider_run_is_transport_active_work() {
        let request = crate::provider::LaunchProviderRequest::new(
            "session-1",
            "agent-1",
            "opencode",
            "default",
            "model",
        );
        let run = crate::provider::RuntimeProviderRun::new(
            "provider-run-1",
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::External,
                process_label: "test-provider".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: std::collections::BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: Some("test-runtime".to_string()),
            },
        );

        assert!(provider_run_counts_as_transport_active_work(&run));
    }
}
