use super::*;

const STALE_TERMINAL_ATTACHMENT_TIMEOUT_MS: u64 = 30_000;
const MAX_PROVIDER_PROCESS_GC_INTERVAL_MS: u64 = 30_000;
const MIN_PROVIDER_PROCESS_GC_INTERVAL_MS: u64 = 250;

impl KernelRuntimeState {
    pub(crate) fn waiting_room_auxiliary_projection(
        &self,
        owner_user_id: &str,
        request: &crate::local::ListExternalProviderSessionsRequest,
    ) -> (
        crate::local::ExternalProviderSessionPage,
        crate::runtime::metaagent_event::MetaagentEventStore,
    ) {
        (
            self.owned
                .external_provider_sessions
                .list_for_owner(owner_user_id, request),
            self.owned.metaagent_events.clone(),
        )
    }

    pub(crate) fn durable_snapshot_scheduler(
        &self,
    ) -> Option<crate::durable_snapshot::DurableSnapshotScheduler> {
        let config = self.owned.config_projection.snapshot();
        let policy = crate::durable_snapshot::DurableCheckpointPolicy::from_user_state_config(
            &config.user_config.state,
        )?;
        Some(
            crate::durable_snapshot::DurableSnapshotScheduler::new_with_policy(
                config.daemon_id,
                self.owned.durable_state_store.clone(),
                self.owned.session_store.clone(),
                self.owned.agent_store.clone(),
                self.owned.slice_store.clone(),
                self.owned.metaagent_events.clone(),
                policy,
            ),
        )
    }

    pub(crate) async fn pump_transport_runtime(&self) {
        if !self.owned.publication_activation.is_active() {
            return;
        }
        let now_ms = crate::session::unix_epoch_ms();
        self.sweep_stale_terminal_attachments(now_ms).await;
        super::workflow_publication_runtime_lifecycle::reconcile_bound_workflow_publication_runtimes(
            self,
        )
        .await;
        self.retry_due_provider_launch_failures(now_ms).await;
        if self.claim_provider_process_gc(now_ms) {
            if let Err(error) = self.reap_idle_provider_processes(now_ms).await {
                crate::logging::warn_with_fields(
                    "daemon.provider_process_gc",
                    "provider process gc failed",
                    serde_json::json!({
                        "error": error.to_string(),
                    }),
                );
            }
        }
        let mut ready_provider_run_ids = self.owned.pty_output_signal.take_ready_provider_run_ids();
        ready_provider_run_ids.extend(
            self.owned
                .provider_store
                .run_actor_completion_signal()
                .take_ready_provider_run_ids(),
        );
        ready_provider_run_ids.extend(
            self.owned
                .structured_output_records
                .take_due_provider_run_ids(now_ms),
        );
        ready_provider_run_ids.extend(
            self.owned
                .provider_output_deadlines
                .take_due_provider_run_ids(now_ms),
        );
        self.owned.reap_structured_prompt_jobs();
        let mut pumped_provider_run_ids = Vec::with_capacity(ready_provider_run_ids.len());
        for provider_run_id in ready_provider_run_ids {
            let Ok(provider_run) = self.owned.provider_store.get_run(&provider_run_id) else {
                continue;
            };
            let session_id = provider_run.session_id().to_string();
            let recipients = self
                .owned
                .attachment_store
                .list_session_attachment_ids(&session_id);
            // Browser/CLI terminal-output requests use this same provider lane. Keep the
            // background pump in it too: otherwise both paths can apply one completion
            // concurrently, allowing a queued prompt promoted by the first path to be
            // settled immediately by the second path's stale turn state.
            let _permit = self.provider_runtime_lanes.acquire(&provider_run_id).await;
            match self
                .pump_owned_provider_output(&session_id, &provider_run_id, recipients, false)
                .await
            {
                Ok(_) => pumped_provider_run_ids.push(provider_run_id),
                Err(error) => crate::logging::warn_with_fields(
                    "daemon.provider_output",
                    "ready provider output pump failed",
                    serde_json::json!({
                        "session_id": session_id,
                        "provider_run_id": provider_run_id,
                        "error": error.to_string(),
                    }),
                ),
            }
        }
        let watchdog_dispatches = self
            .owned
            .workflow_collect_due_watchdog_dispatches(crate::session::unix_epoch_ms());
        self.spawn_workflow_prompt_dispatches(watchdog_dispatches);
        self.dispatch_due_agent_prompt_schedules(crate::session::unix_epoch_ms())
            .await;
        for provider_run_id in pumped_provider_run_ids {
            self.observe_git_after_provider_activity_if_pending(&provider_run_id)
                .await;
        }
    }

    pub(crate) async fn record_terminal_attachment_heartbeat(
        &self,
        session_id: &str,
        attachment_id: &str,
        now_ms: u64,
    ) -> Result<(), DaemonError> {
        self.owned
            .attachment_store
            .record_heartbeat(session_id, attachment_id, now_ms)
    }

    pub(crate) async fn sweep_stale_terminal_attachments(&self, now_ms: u64) {
        let stale_attachment_ids = self
            .owned
            .attachment_store
            .stale_terminal_attachment_ids(now_ms, STALE_TERMINAL_ATTACHMENT_TIMEOUT_MS);
        for attachment_id in stale_attachment_ids {
            match self.owned.detach(&attachment_id) {
                Ok(attachment) => {
                    crate::logging::warn_with_fields(
                        "daemon.session",
                        "detached stale terminal attachment",
                        serde_json::json!({
                            "session_id": attachment.session_id(),
                            "attachment_id": attachment.id(),
                            "stale_after_ms": STALE_TERMINAL_ATTACHMENT_TIMEOUT_MS,
                        }),
                    );
                }
                Err(error) => {
                    crate::logging::warn_with_fields(
                        "daemon.session",
                        "failed detaching stale terminal attachment",
                        serde_json::json!({
                            "attachment_id": attachment_id,
                            "error": error.to_string(),
                        }),
                    );
                }
            }
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

    pub(crate) async fn reap_idle_provider_processes(
        &self,
        now_ms: u64,
    ) -> Result<crate::app::ProviderProcessReapSummary, DaemonError> {
        let config = self.owned.config_projection.snapshot();
        let idle_ttl_ms = config.provider_process_idle_ttl_ms;
        let orphan_ttl_ms = config.provider_process_orphan_ttl_ms;
        let summary = self
            .with_app_side_effect(move |app| {
                app.reap_idle_provider_processes(now_ms, idle_ttl_ms, orphan_ttl_ms)
            })
            .await?;
        if summary.tracked_processes_reaped > 0 || summary.orphan_processes_reaped > 0 {
            crate::logging::warn_with_fields(
                "daemon.provider_process_gc",
                "provider process gc reaped idle processes",
                serde_json::json!({
                    "tracked_processes_reaped": summary.tracked_processes_reaped,
                    "orphan_processes_reaped": summary.orphan_processes_reaped,
                }),
            );
        }
        Ok(summary)
    }

    fn claim_provider_process_gc(&self, now_ms: u64) -> bool {
        let config = self.owned.config_projection.snapshot();
        let interval_ms = provider_process_gc_interval_ms(
            config.provider_process_idle_ttl_ms,
            config.provider_process_orphan_ttl_ms,
        );
        claim_periodic_sweep(
            &self.owned.next_provider_process_gc_at_ms,
            now_ms,
            interval_ms,
        )
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

    pub(crate) fn record_waiting_room_change(&self) {
        self.owned.runtime_projection_changes.record_change();
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
        let next_runtime_due_at_ms = [
            self.next_structured_output_poll_due_at_ms(),
            self.owned.provider_output_deadlines.next_due_at_ms(),
            self.owned.provider_launch_failure_retries.next_due_at_ms(),
        ]
        .into_iter()
        .flatten()
        .min();
        transport_runtime_pump_interval_for_state(
            next_runtime_due_at_ms,
            self.owned
                .session_store
                .read()
                .next_scheduled_runtime_wake_at_ms(),
            now_ms,
            active_interval_ms,
            idle_interval_ms,
        )
    }

    fn next_structured_output_poll_due_at_ms(&self) -> Option<u64> {
        self.owned.structured_output_records.next_poll_due_at_ms()
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
}

fn transport_runtime_pump_interval_for_state(
    next_runtime_due_at_ms: Option<u64>,
    next_watchdog_run_at_ms: Option<u64>,
    now_ms: u64,
    minimum_interval_ms: u64,
    idle_interval_ms: u64,
) -> u64 {
    let fallback_interval_ms = idle_interval_ms;
    let next_due_at_ms = [next_runtime_due_at_ms, next_watchdog_run_at_ms]
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

fn provider_process_gc_interval_ms(idle_ttl_ms: u64, orphan_ttl_ms: u64) -> u64 {
    idle_ttl_ms
        .min(orphan_ttl_ms)
        .min(MAX_PROVIDER_PROCESS_GC_INTERVAL_MS)
        .max(MIN_PROVIDER_PROCESS_GC_INTERVAL_MS)
}

fn claim_periodic_sweep(next_at_ms: &AtomicU64, now_ms: u64, interval_ms: u64) -> bool {
    let mut next_at = next_at_ms.load(Ordering::Acquire);
    loop {
        if now_ms < next_at {
            return false;
        }
        match next_at_ms.compare_exchange_weak(
            next_at,
            now_ms.saturating_add(interval_ms),
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return true,
            Err(current) => next_at = current,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        claim_periodic_sweep, provider_process_gc_interval_ms,
        transport_runtime_pump_interval_for_state,
    };
    use std::sync::atomic::AtomicU64;

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

    #[test]
    fn provider_process_gc_interval_respects_short_ttls_and_caps_default_work() {
        assert_eq!(provider_process_gc_interval_ms(1_000, u64::MAX), 1_000);
        assert_eq!(provider_process_gc_interval_ms(300_000, 30_000), 30_000);
        assert_eq!(provider_process_gc_interval_ms(0, 30_000), 250);
    }

    #[test]
    fn periodic_sweep_claims_once_per_interval() {
        let next_at_ms = AtomicU64::new(0);
        assert!(claim_periodic_sweep(&next_at_ms, 10_000, 30_000));
        assert!(!claim_periodic_sweep(&next_at_ms, 10_001, 30_000));
        assert!(!claim_periodic_sweep(&next_at_ms, 39_999, 30_000));
        assert!(claim_periodic_sweep(&next_at_ms, 40_000, 30_000));
    }
}
