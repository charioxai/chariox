use crate::agent::AgentInstance;
use crate::app::DaemonApp;
use crate::durable_snapshot::{DurableKernelSnapshotPayload, DurableSnapshotScheduler};
use crate::durable_state::{DurableKernelStateStore, DurableStateEvent};
use crate::error::DaemonError;
use crate::runtime::metaagent_event::{
    MetaagentEventRecord, MetaagentEventSnapshot, MetaagentEventSubscription,
};
use crate::session::RuntimeSession;
use std::path::{Path, PathBuf};

fn decode_durable_payload_field<T>(
    event: &DurableStateEvent,
    field: &'static str,
    operation: &'static str,
) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned,
{
    let value = event
        .payload
        .get(field)
        .cloned()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation,
            message: format!(
                "durable state event {} ({}) missing payload field {field}",
                event.event_id, event.kind
            ),
        })?;
    serde_json::from_value(value).map_err(|error| DaemonError::LocalTransport {
        operation,
        message: format!(
            "durable state event {} ({}) has invalid payload field {field}: {error}",
            event.event_id, event.kind
        ),
    })
}

impl DaemonApp {
    pub(super) fn restore_durable_state(&mut self) -> Result<(), DaemonError> {
        let replay_after_sequence = match self.durable_state.latest_snapshot()? {
            Some(snapshot) => {
                self.restore_durable_state_snapshot(snapshot.payload)?;
                snapshot.sequence
            }
            None => 0,
        };
        for event in self
            .durable_state
            .load_events_after(replay_after_sequence)?
        {
            self.restore_durable_state_event(event)?;
        }
        self.restore_local_kernel_external_provider_attachments();
        self.reconcile_restored_runtime_state_after_restart()?;
        Ok(())
    }

    fn restore_durable_state_snapshot(
        &mut self,
        payload: serde_json::Value,
    ) -> Result<(), DaemonError> {
        let snapshot: DurableKernelSnapshotPayload =
            serde_json::from_value(payload).map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.restore_snapshot",
                message: error.to_string(),
            })?;
        let restored_session_ids: std::collections::BTreeSet<String> = snapshot
            .sessions
            .iter()
            .filter(|session| self.session_belongs_to_current_kernel(session))
            .map(|session| session.id().to_string())
            .collect();
        for session in snapshot.sessions {
            if !restored_session_ids.contains(session.id()) {
                continue;
            }
            self.sessions.restore_session(session);
        }
        let restored_slices = snapshot
            .slices
            .into_iter()
            .filter(|slice| slice.owner_kernel_id == self.config.daemon_id)
            .collect::<Vec<_>>();
        self.slices.restore_records(restored_slices);
        self.slices
            .restore_saved_state_records(snapshot.slice_saved_states, snapshot.slice_backups);
        let mut restored_agent_ids = std::collections::BTreeSet::<String>::new();
        for agent in snapshot.agents {
            if !restored_session_ids.contains(agent.session_id()) {
                continue;
            }
            restored_agent_ids.insert(agent.id().to_string());
            self.mark_agent_external_provider_sessions_attached(&agent);
            self.agents.restore_agent(agent);
        }
        self.metaagent_events
            .restore_snapshot(MetaagentEventSnapshot {
                records: snapshot
                    .metaagent_event_records
                    .into_iter()
                    .filter(|record| restored_session_ids.contains(&record.session_id))
                    .collect(),
                subscriptions: snapshot
                    .metaagent_event_subscriptions
                    .into_iter()
                    .filter(|subscription| restored_agent_ids.contains(&subscription.metaagent_id))
                    .collect(),
            });
        self.refresh_restored_session_projections()?;
        Ok(())
    }

    fn session_belongs_to_current_kernel(&self, session: &RuntimeSession) -> bool {
        session.host_daemon_id() == self.config.daemon_id
    }

    fn mark_agent_external_provider_sessions_attached(&self, agent: &AgentInstance) {
        if let Some(import) = agent.external_provider_import() {
            self.external_provider_sessions.mark_import_attached(
                import,
                agent.session_id(),
                agent.id(),
            );
        }
        let resume_state = agent.provider_resume_state();
        for (provider, provider_session_id) in [
            ("codex", resume_state.codex_thread_id()),
            ("opencode", resume_state.opencode_session_id()),
            ("claude", resume_state.claude_session_id()),
        ] {
            let Some(provider_session_id) = provider_session_id else {
                continue;
            };
            self.external_provider_sessions
                .mark_provider_session_attached(
                    provider,
                    provider_session_id,
                    agent.session_id(),
                    agent.id(),
                );
        }
    }

    fn restore_local_kernel_external_provider_attachments(&self) {
        let Some(root) = self.local_kernel_state_root() else {
            return;
        };
        let mut state_db_count = 0usize;
        let mut marker_count = 0usize;
        let mut error_count = 0usize;
        let Ok(entries) = std::fs::read_dir(&root) else {
            return;
        };
        for entry in entries.flatten() {
            let state_path = entry.path().join("state.db");
            if !state_path.is_file() {
                continue;
            }
            state_db_count += 1;
            match self.restore_external_provider_attachments_from_state_db(&state_path) {
                Ok(count) => marker_count += count,
                Err(error) => {
                    error_count += 1;
                    crate::logging::warn_with_fields(
                        "durable_state.restore",
                        "failed to scan local kernel state for external provider attachments",
                        serde_json::json!({
                            "state_path": state_path.display().to_string(),
                            "error": error.to_string(),
                        }),
                    );
                }
            }
        }
        crate::logging::info_with_fields(
            "durable_state.restore",
            "scanned local kernel state for external provider attachments",
            serde_json::json!({
                "state_root": root.display().to_string(),
                "state_db_count": state_db_count,
                "attachment_marker_count": marker_count,
                "error_count": error_count,
            }),
        );
    }

    fn local_kernel_state_root(&self) -> Option<PathBuf> {
        let state_path = self.config.durable_state_path();
        if state_path.file_name()?.to_str()? != "state.db" {
            return None;
        }
        let daemon_dir = state_path.parent()?;
        let kernels_dir = daemon_dir.parent()?;
        (kernels_dir.file_name()?.to_str()? == "kernels").then(|| kernels_dir.to_path_buf())
    }

    fn restore_external_provider_attachments_from_state_db(
        &self,
        state_path: &Path,
    ) -> Result<usize, DaemonError> {
        let store = DurableKernelStateStore::open(state_path.to_path_buf())?;
        let mut marker_count = 0usize;
        for event in store.load_events_after(0)? {
            marker_count += self.mark_external_provider_attachments_from_event(&event);
        }
        Ok(marker_count)
    }

    fn mark_external_provider_attachments_from_event(&self, event: &DurableStateEvent) -> usize {
        match event.kind.as_str() {
            "session.created" => decode_durable_payload_field::<AgentInstance>(
                event,
                "default_agent",
                "durable_state.restore_external_provider_attachment",
            )
            .ok()
            .map(|agent| self.mark_agent_external_provider_sessions_attached_counted(&agent))
            .unwrap_or(0),
            "agent.created"
            | "agent.mcp_granted"
            | "agent.mcp_revoked"
            | "agent.skill_granted"
            | "agent.skill_revoked"
            | "agent.extension_granted"
            | "agent.extension_revoked"
            | "agent.runtime_profile_updated"
            | "agent.updated" => decode_durable_payload_field::<AgentInstance>(
                event,
                "agent",
                "durable_state.restore_external_provider_attachment",
            )
            .ok()
            .map(|agent| self.mark_agent_external_provider_sessions_attached_counted(&agent))
            .unwrap_or(0),
            "agents.created" => decode_durable_payload_field::<Vec<AgentInstance>>(
                event,
                "agents",
                "durable_state.restore_external_provider_attachment_batch",
            )
            .ok()
            .map(|agents| {
                agents
                    .iter()
                    .map(|agent| self.mark_agent_external_provider_sessions_attached_counted(agent))
                    .sum()
            })
            .unwrap_or(0),
            _ => 0,
        }
    }

    fn mark_agent_external_provider_sessions_attached_counted(
        &self,
        agent: &AgentInstance,
    ) -> usize {
        let mut count = 0usize;
        if let Some(import) = agent.external_provider_import() {
            self.external_provider_sessions.mark_import_attached(
                import,
                agent.session_id(),
                agent.id(),
            );
            count += 1;
        }
        let resume_state = agent.provider_resume_state();
        for (provider, provider_session_id) in [
            ("codex", resume_state.codex_thread_id()),
            ("opencode", resume_state.opencode_session_id()),
            ("claude", resume_state.claude_session_id()),
        ] {
            let Some(provider_session_id) = provider_session_id else {
                continue;
            };
            self.external_provider_sessions
                .mark_provider_session_attached(
                    provider,
                    provider_session_id,
                    agent.session_id(),
                    agent.id(),
                );
            count += 1;
        }
        count
    }

    fn refresh_restored_session_projections(&self) -> Result<(), DaemonError> {
        let sessions = self.sessions.read().store().list();
        for mut session in sessions {
            let agents = self.agents.get_session_agents(session.id());
            session.set_agents(agents);
            self.sessions.restore_session(session.clone());
            self.update_session_projection(session);
        }
        Ok(())
    }

    fn refresh_restored_agent_session_projection(
        &self,
        session_id: &str,
    ) -> Result<(), DaemonError> {
        let mut session = self.sessions.get_session(session_id)?;
        let agents = self.agents.get_session_agents(session_id);
        session.set_agents(agents);
        self.sessions.restore_session(session.clone());
        self.update_session_projection(session);
        Ok(())
    }

    fn reconcile_restored_runtime_state_after_restart(&self) -> Result<(), DaemonError> {
        let sessions = self.sessions.read().store().list();
        for mut session in sessions {
            let reconciliation = session.reconcile_after_kernel_restart();
            if !reconciliation.changed() {
                continue;
            }
            let agents = self.agents.get_session_agents(session.id());
            session.set_agents(agents);
            self.sessions.restore_session(session.clone());
            self.update_session_projection(session.clone());
            crate::logging::info_with_fields(
                "durable_state.restore",
                "reconciled runtime state after kernel restart",
                serde_json::json!({
                    "session_id": session.id(),
                    "cleared_active_provider_run": reconciliation.cleared_active_provider_run,
                    "interrupted_prompt_count": reconciliation.interrupted_prompt_count,
                    "stopped_workflow_run_count": reconciliation.stopped_workflow_run_count,
                }),
            );
        }
        let reconciled_slices = self.slices.reconcile_after_kernel_restart_with_host_state(
            crate::session::unix_epoch_ms(),
            crate::slice::inspect_local_docker_slice_host_runtime,
        );
        for slice in reconciled_slices {
            self.durable_state.append_event(
                "slice.updated",
                Some(slice.id.clone()),
                serde_json::json!({ "slice": &slice }),
            )?;
            crate::logging::info_with_fields(
                "durable_state.restore",
                "reconciled slice runtime state after kernel restart",
                serde_json::json!({
                    "slice_id": slice.id,
                    "slice_name": slice.name,
                    "status": slice.status,
                }),
            );
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub(crate) fn save_durable_state_snapshot(&self) -> Result<(), DaemonError> {
        let sequence = self.durable_state.latest_event_sequence()?;
        let payload = DurableKernelSnapshotPayload::capture(
            &self.sessions,
            &self.agents,
            &self.slices,
            &self.metaagent_events,
        );
        self.durable_state.save_snapshot(
            sequence,
            serde_json::to_value(payload).map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.encode_snapshot_payload",
                message: error.to_string(),
            })?,
        )?;
        Ok(())
    }

    pub(crate) fn durable_snapshot_scheduler(&self) -> Option<DurableSnapshotScheduler> {
        let interval_events = self.config.user_config.state.snapshot_interval_events? as u64;
        Some(DurableSnapshotScheduler::new(
            self.durable_state_store(),
            self.session_state_store(),
            self.agents.clone(),
            self.slices.clone(),
            self.metaagent_events.clone(),
            interval_events,
        ))
    }

    fn restore_durable_state_event(&mut self, event: DurableStateEvent) -> Result<(), DaemonError> {
        match event.kind.as_str() {
            "session.created" => {
                let session: RuntimeSession = decode_durable_payload_field(
                    &event,
                    "session",
                    "durable_state.restore_session",
                )?;
                if !self.session_belongs_to_current_kernel(&session) {
                    return Ok(());
                }
                let default_agent: AgentInstance = decode_durable_payload_field(
                    &event,
                    "default_agent",
                    "durable_state.restore_default_agent",
                )?;
                self.mark_agent_external_provider_sessions_attached(&default_agent);
                self.sessions.restore_session(session.clone());
                self.agents.restore_agent(default_agent);
                self.update_session_projection(session);
            }
            "session.updated" => {
                let session: RuntimeSession = decode_durable_payload_field(
                    &event,
                    "session",
                    "durable_state.restore_session_update",
                )?;
                if !self.session_belongs_to_current_kernel(&session) {
                    return Ok(());
                }
                self.sessions.restore_session(session.clone());
                self.update_session_projection(session);
            }
            "agent.created" => {
                let agent: AgentInstance =
                    decode_durable_payload_field(&event, "agent", "durable_state.restore_agent")?;
                let session_id = agent.session_id().to_string();
                if self.sessions.get_session(&session_id).is_err() {
                    return Ok(());
                }
                self.mark_agent_external_provider_sessions_attached(&agent);
                self.agents.restore_agent(agent);
                self.refresh_restored_agent_session_projection(&session_id)?;
            }
            "agents.created" => {
                let agents: Vec<AgentInstance> = decode_durable_payload_field(
                    &event,
                    "agents",
                    "durable_state.restore_agent_batch",
                )?;
                let mut restored_session_ids = std::collections::BTreeSet::new();
                for agent in agents {
                    let session_id = agent.session_id().to_string();
                    if self.sessions.get_session(&session_id).is_err() {
                        continue;
                    }
                    self.mark_agent_external_provider_sessions_attached(&agent);
                    self.agents.restore_agent(agent);
                    restored_session_ids.insert(session_id);
                }
                for session_id in restored_session_ids {
                    self.refresh_restored_agent_session_projection(&session_id)?;
                }
            }
            "agent.mcp_granted"
            | "agent.mcp_revoked"
            | "agent.skill_granted"
            | "agent.skill_revoked"
            | "agent.extension_granted"
            | "agent.extension_revoked"
            | "agent.runtime_profile_updated"
            | "agent.updated" => {
                let agent: AgentInstance = decode_durable_payload_field(
                    &event,
                    "agent",
                    "durable_state.restore_agent_update",
                )?;
                let session_id = agent.session_id().to_string();
                if self.sessions.get_session(&session_id).is_err() {
                    return Ok(());
                }
                self.mark_agent_external_provider_sessions_attached(&agent);
                self.agents.restore_agent(agent);
                self.refresh_restored_agent_session_projection(&session_id)?;
            }
            "session.ended" => {
                let mut session: RuntimeSession = decode_durable_payload_field(
                    &event,
                    "session",
                    "durable_state.restore_ended_session",
                )?;
                if !self.session_belongs_to_current_kernel(&session) {
                    return Ok(());
                }
                self.agents.remove_session_agents(session.id());
                session.set_agents(Vec::new());
                self.sessions.restore_session(session.clone());
                self.update_session_projection(session);
            }
            "session.deleted" => {
                let mut session: RuntimeSession = decode_durable_payload_field(
                    &event,
                    "session",
                    "durable_state.restore_deleted_session",
                )?;
                if !self.session_belongs_to_current_kernel(&session) {
                    return Ok(());
                }
                self.agents.remove_session_agents(session.id());
                session.set_agents(Vec::new());
                self.sessions.remove_restored_session(session.id());
                self.session_projection.remove(session.id());
                self.history_projection.remove(session.id());
                self.agent_runtime_projection.update_session(&session);
            }
            "slice.created" | "slice.updated" => {
                let slice: crate::slice::SliceRecord =
                    decode_durable_payload_field(&event, "slice", "durable_state.restore_slice")?;
                if slice.owner_kernel_id == self.config.daemon_id {
                    let mut slices = self.slices.list();
                    slices.retain(|record| record.id != slice.id);
                    slices.push(slice);
                    self.slices.restore_records(slices);
                }
            }
            "slice.deleted" => {
                let slice: crate::slice::SliceRecord = decode_durable_payload_field(
                    &event,
                    "slice",
                    "durable_state.restore_deleted_slice",
                )?;
                if slice.owner_kernel_id == self.config.daemon_id {
                    let mut slices = self.slices.list();
                    slices.retain(|record| record.id != slice.id);
                    self.slices.restore_records(slices);
                }
            }
            "slice.state.saved" => {
                let state: crate::slice::SliceSavedStateRecord = decode_durable_payload_field(
                    &event,
                    "state",
                    "durable_state.restore_slice_saved_state",
                )?;
                let mut states = self.slices.list_saved_states();
                states.retain(|record| record.id != state.id);
                states.push(state);
                self.slices
                    .restore_saved_state_records(states, self.slices.list_backups());
            }
            "slice.state.deleted" => {
                let state: crate::slice::SliceSavedStateRecord = decode_durable_payload_field(
                    &event,
                    "state",
                    "durable_state.restore_deleted_slice_saved_state",
                )?;
                let mut states = self.slices.list_saved_states();
                states.retain(|record| record.id != state.id);
                self.slices
                    .restore_saved_state_records(states, self.slices.list_backups());
            }
            "slice.backup.created" => {
                let backup: crate::slice::SliceBackupRecord = decode_durable_payload_field(
                    &event,
                    "backup",
                    "durable_state.restore_slice_backup",
                )?;
                let mut backups = self.slices.list_backups();
                backups.retain(|record| record.id != backup.id);
                backups.push(backup);
                self.slices
                    .restore_saved_state_records(self.slices.list_saved_states(), backups);
            }
            "metaagent.event.recorded"
            | "metaagent.event.read"
            | "metaagent.event.acked"
            | "metaagent.event.delivery_updated" => {
                let record: MetaagentEventRecord = decode_durable_payload_field(
                    &event,
                    "record",
                    "durable_state.restore_metaagent_event_record",
                )?;
                if self.sessions.get_session(&record.session_id).is_ok()
                    && self.agents.get_agent(&record.metaagent_id).is_ok()
                {
                    self.metaagent_events.restore_record(record);
                }
            }
            "metaagent.subscription.created" => {
                let subscription: MetaagentEventSubscription = decode_durable_payload_field(
                    &event,
                    "subscription",
                    "durable_state.restore_metaagent_subscription",
                )?;
                if self.agents.get_agent(&subscription.metaagent_id).is_ok() {
                    self.metaagent_events.restore_subscription(subscription);
                }
            }
            "metaagent.subscription.deleted" => {
                let subscription: MetaagentEventSubscription = decode_durable_payload_field(
                    &event,
                    "subscription",
                    "durable_state.restore_deleted_metaagent_subscription",
                )?;
                self.metaagent_events.remove_restored_subscription(
                    &subscription.metaagent_id,
                    &subscription.subscription_id,
                );
            }
            _ => {}
        }
        Ok(())
    }
}
