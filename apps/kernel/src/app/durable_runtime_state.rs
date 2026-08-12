use crate::agent::AgentInstance;
use crate::app::DaemonApp;
use crate::durable_prompt_state::{
    DurablePromptStateEventPayload, DURABLE_PROMPT_STATE_EVENT_KIND,
};
use crate::durable_snapshot::{DurableKernelSnapshotPayload, DurableSnapshotScheduler};
use crate::durable_state::{DurableKernelStateStore, DurableStateEvent};
use crate::error::DaemonError;
use crate::runtime::metaagent_event::{
    MetaagentEventRecord, MetaagentEventSnapshot, MetaagentEventSubscription,
};
use crate::session::{DurablePromptPrivateState, RuntimeProject, RuntimeSession};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const DURABLE_EVENT_REPLAY_BATCH_SIZE: usize = 128;
const DURABLE_RESTORE_DIAGNOSTIC_SAMPLE_LIMIT: usize = 20;

#[derive(Default)]
struct DurableRestoreDiagnostics {
    ignored_prompt_state_count: usize,
    sample_missing_session_ids: BTreeSet<String>,
    sample_truncated: bool,
}

impl DurableRestoreDiagnostics {
    fn record_missing_prompt_session(&mut self, session_id: String) {
        self.ignored_prompt_state_count += 1;
        if self.sample_missing_session_ids.len() < DURABLE_RESTORE_DIAGNOSTIC_SAMPLE_LIMIT {
            self.sample_missing_session_ids.insert(session_id);
        } else if !self.sample_missing_session_ids.contains(&session_id) {
            self.sample_truncated = true;
        }
    }

    fn log_summary(&self) {
        if self.ignored_prompt_state_count == 0 {
            return;
        }
        let sample_session_ids = self
            .sample_missing_session_ids
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        crate::logging::warn_with_fields(
            "durable_state.restore",
            "ignored prompt states for missing runtime sessions",
            serde_json::json!({
                "ignored_prompt_state_count": self.ignored_prompt_state_count,
                "missing_session_sample_count": self.sample_missing_session_ids.len(),
                "sample_session_ids": sample_session_ids,
                "sample_truncated": self.sample_truncated,
            }),
        );
    }
}

#[cfg(test)]
mod durable_restore_diagnostics_tests {
    use super::{DurableRestoreDiagnostics, DURABLE_RESTORE_DIAGNOSTIC_SAMPLE_LIMIT};

    #[test]
    fn missing_session_diagnostics_retain_a_bounded_sample() {
        let mut diagnostics = DurableRestoreDiagnostics::default();
        for index in 0..(DURABLE_RESTORE_DIAGNOSTIC_SAMPLE_LIMIT + 5) {
            diagnostics.record_missing_prompt_session(format!("session-{index:02}"));
        }
        diagnostics.record_missing_prompt_session("session-00".to_string());

        assert_eq!(
            diagnostics.ignored_prompt_state_count,
            DURABLE_RESTORE_DIAGNOSTIC_SAMPLE_LIMIT + 6
        );
        assert_eq!(
            diagnostics.sample_missing_session_ids.len(),
            DURABLE_RESTORE_DIAGNOSTIC_SAMPLE_LIMIT
        );
        assert!(diagnostics.sample_truncated);
    }
}

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

fn durable_payload_entity_belongs_to_other_owner(
    event: &DurableStateEvent,
    field: &str,
    owner_field: &str,
    current_owner_id: &str,
) -> bool {
    event
        .payload
        .get(field)
        .and_then(|entity| entity.get(owner_field))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|owner_id| owner_id != current_owner_id)
}

fn durable_snapshot_belongs_to_other_kernel(
    payload: &serde_json::Value,
    current_daemon_id: &str,
) -> bool {
    let session_owners = payload
        .get("sessions")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|session| session.get("host_daemon_id"))
        .filter_map(serde_json::Value::as_str);
    let slice_owners = payload
        .get("slices")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|slice| slice.get("owner_kernel_id"))
        .filter_map(serde_json::Value::as_str);
    let owners = session_owners.chain(slice_owners).collect::<Vec<_>>();
    !owners.is_empty() && owners.iter().all(|owner| *owner != current_daemon_id)
}

#[derive(Default)]
struct RestoredExternalProviderAttachmentState {
    live_session_ids: BTreeSet<String>,
    live_agents: BTreeMap<String, AgentInstance>,
}

impl RestoredExternalProviderAttachmentState {
    fn restore_snapshot(&mut self, snapshot: DurableKernelSnapshotPayload) {
        self.live_session_ids = snapshot
            .sessions
            .iter()
            .map(|session| session.id().to_string())
            .collect();
        self.live_agents = snapshot
            .agents
            .into_iter()
            .filter(|agent| self.live_session_ids.contains(agent.session_id()))
            .map(|agent| (agent.id().to_string(), agent))
            .collect();
    }

    fn apply_event(&mut self, event: &DurableStateEvent) {
        match event.kind.as_str() {
            "session.created" => {
                if let Ok(session) = decode_durable_payload_field::<RuntimeSession>(
                    event,
                    "session",
                    "durable_state.scan_external_provider_attachment_session",
                ) {
                    self.live_session_ids.insert(session.id().to_string());
                }
                if let Ok(agent) = decode_durable_payload_field::<AgentInstance>(
                    event,
                    "default_agent",
                    "durable_state.scan_external_provider_attachment_default_agent",
                ) {
                    self.restore_agent_if_session_live(agent);
                }
            }
            "session.updated" => {
                if let Ok(session) = decode_durable_payload_field::<RuntimeSession>(
                    event,
                    "session",
                    "durable_state.scan_external_provider_attachment_session_update",
                ) {
                    self.live_session_ids.insert(session.id().to_string());
                }
            }
            "sessions.updated" => {
                if let Ok(sessions) = decode_durable_payload_field::<Vec<RuntimeSession>>(
                    event,
                    "sessions",
                    "durable_state.scan_external_provider_attachment_session_batch",
                ) {
                    self.live_session_ids
                        .extend(sessions.into_iter().map(|session| session.id().to_string()));
                }
            }
            "agent.created"
            | "agent.mcp_granted"
            | "agent.mcp_revoked"
            | "agent.skill_granted"
            | "agent.skill_revoked"
            | "agent.extension_granted"
            | "agent.extension_revoked"
            | "agent.runtime_profile_updated"
            | "agent.updated" => {
                if let Ok(agent) = decode_durable_payload_field::<AgentInstance>(
                    event,
                    "agent",
                    "durable_state.scan_external_provider_attachment_agent",
                ) {
                    self.restore_agent_if_session_live(agent);
                }
            }
            "agents.created" => {
                if let Ok(agents) = decode_durable_payload_field::<Vec<AgentInstance>>(
                    event,
                    "agents",
                    "durable_state.scan_external_provider_attachment_agent_batch",
                ) {
                    for agent in agents {
                        self.restore_agent_if_session_live(agent);
                    }
                }
            }
            "agent.deleted" => {
                if let Ok(agent) = decode_durable_payload_field::<AgentInstance>(
                    event,
                    "agent",
                    "durable_state.scan_external_provider_attachment_agent_deleted",
                ) {
                    self.live_agents.remove(agent.id());
                }
            }
            "session.ended" | "session.deleted" => {
                if let Ok(session) = decode_durable_payload_field::<RuntimeSession>(
                    event,
                    "session",
                    "durable_state.scan_external_provider_attachment_session_removed",
                ) {
                    self.remove_session(session.id());
                }
            }
            _ => {}
        }
    }

    fn restore_agent_if_session_live(&mut self, agent: AgentInstance) {
        if self.live_session_ids.contains(agent.session_id()) {
            self.live_agents.insert(agent.id().to_string(), agent);
        }
    }

    fn remove_session(&mut self, session_id: &str) {
        self.live_session_ids.remove(session_id);
        self.live_agents
            .retain(|_, agent| agent.session_id() != session_id);
    }
}

impl DaemonApp {
    pub(super) fn restore_durable_state(&mut self) -> Result<(), DaemonError> {
        let replay_after_sequence = match self.durable_state.latest_snapshot()? {
            Some(snapshot) => {
                if self.restore_durable_state_snapshot(snapshot.payload)? {
                    snapshot.sequence
                } else {
                    crate::logging::info_with_fields(
                        "durable_state.restore",
                        "ignored durable snapshot for another kernel owner",
                        serde_json::json!({
                            "snapshot_sequence": snapshot.sequence,
                            "daemon_id": self.config.daemon_id,
                        }),
                    );
                    0
                }
            }
            None => 0,
        };
        let mut diagnostics = DurableRestoreDiagnostics::default();
        let mut replay_cursor = replay_after_sequence;
        loop {
            let events = self
                .durable_state
                .load_events_after_batch(replay_cursor, DURABLE_EVENT_REPLAY_BATCH_SIZE)?;
            if events.is_empty() {
                break;
            }
            for event in events {
                replay_cursor = replay_cursor.max(event.sequence);
                self.restore_durable_state_event(event, &mut diagnostics)?;
            }
        }
        diagnostics.log_summary();
        self.reconcile_restored_default_project_workspaces()?;
        self.remove_restored_projects_without_visible_sessions()?;
        self.reconcile_restored_duplicate_project_names()?;
        self.restore_local_kernel_external_provider_attachments();
        self.reconcile_restored_slice_agent_attachments()?;
        self.reconcile_restored_runtime_state_after_restart()?;
        Ok(())
    }

    fn reconcile_restored_default_project_workspaces(&self) -> Result<(), DaemonError> {
        let projects_before = self
            .sessions
            .durable_projects()
            .into_iter()
            .map(|project| (project.id().to_string(), project))
            .collect::<BTreeMap<_, _>>();
        let mut replacements_by_workspace = BTreeMap::<(String, String), BTreeSet<String>>::new();
        let mut migrations = Vec::<(RuntimeSession, String)>::new();
        let mut canonical_workspace_by_legacy_id = BTreeMap::<String, Option<String>>::new();
        for session in self.sessions.list_all_sessions() {
            if session.is_hidden() {
                continue;
            }
            let Some(project) = projects_before.get(session.project_id()) else {
                continue;
            };
            if project.kind() != crate::session::RuntimeProjectKind::Default {
                continue;
            }
            let workspace_id = canonical_workspace_by_legacy_id
                .entry(session.workspace_id().to_string())
                .or_insert_with(|| {
                    crate::runtime::workspace_git_common::canonical_workspace_path(
                        session.workspace_id(),
                        session.worktree_id(),
                    )
                })
                .clone();
            let Some(workspace_id) = workspace_id else {
                continue;
            };
            if workspace_id == session.workspace_id() {
                continue;
            }
            replacements_by_workspace
                .entry((session.owner_user_id().to_string(), workspace_id.clone()))
                .or_default()
                .insert(project.id().to_string());
            migrations.push((session, workspace_id));
        }

        let mut migrated_sessions = Vec::new();
        for (session, workspace_id) in migrations {
            let replaced_project_ids = replacements_by_workspace
                .get(&(session.owner_user_id().to_string(), workspace_id.clone()))
                .expect("planned workspace migration should have replacement projects");
            let project_name =
                crate::runtime::workspace_git_common::workspace_display_label(&workspace_id);
            if let Some(migrated) = self.sessions.migrate_default_project_workspace(
                session.id(),
                &workspace_id,
                project_name.as_deref(),
                replaced_project_ids,
            )? {
                migrated_sessions.push((session, migrated));
            }
        }

        let projects_after = self
            .sessions
            .durable_projects()
            .into_iter()
            .map(|project| (project.id().to_string(), project))
            .collect::<BTreeMap<_, _>>();
        for (project_id, project) in &projects_after {
            let kind = match projects_before.get(project_id) {
                None => "project.created",
                Some(previous) if previous != project => "project.updated",
                Some(_) => continue,
            };
            self.durable_state.append_event(
                kind,
                Some(project_id.clone()),
                serde_json::json!({ "project": project }),
            )?;
        }
        for (previous, session) in migrated_sessions {
            self.durable_state.append_event(
                "session.updated",
                Some(session.id().to_string()),
                serde_json::json!({
                    "session": &session,
                    "reason": "canonical_workspace_project_migration",
                }),
            )?;
            self.update_session_projection(session.clone());
            crate::logging::info_with_fields(
                "durable_state.restore",
                "migrated linked-worktree session into canonical workspace project",
                serde_json::json!({
                    "session_id": session.id(),
                    "previous_workspace_id": previous.workspace_id(),
                    "workspace_id": session.workspace_id(),
                    "previous_project_id": previous.project_id(),
                    "project_id": session.project_id(),
                    "worktree_id": session.worktree_id(),
                }),
            );
        }
        Ok(())
    }

    fn remove_restored_projects_without_visible_sessions(&self) -> Result<(), DaemonError> {
        for project in self.sessions.remove_projects_without_visible_sessions() {
            self.durable_state.append_event(
                "project.deleted",
                Some(project.id().to_string()),
                serde_json::json!({ "project": &project }),
            )?;
            crate::logging::info_with_fields(
                "durable_state.restore",
                "removed project without visible sessions",
                serde_json::json!({
                    "project_id": project.id(),
                    "project_name": project.name(),
                }),
            );
        }
        Ok(())
    }

    fn reconcile_restored_duplicate_project_names(&self) -> Result<(), DaemonError> {
        for project in self.sessions.reconcile_duplicate_project_names() {
            self.durable_state.append_event(
                "project.updated",
                Some(project.id().to_string()),
                serde_json::json!({ "project": &project }),
            )?;
            crate::logging::info_with_fields(
                "durable_state.restore",
                "renamed duplicate project",
                serde_json::json!({
                    "project_id": project.id(),
                    "project_name": project.name(),
                }),
            );
        }
        Ok(())
    }

    fn reconcile_restored_slice_agent_attachments(&self) -> Result<(), DaemonError> {
        let slices = self
            .slices
            .list()
            .into_iter()
            .map(|slice| (slice.id.clone(), slice))
            .collect::<BTreeMap<_, _>>();
        let attachments = self
            .agents
            .list_agents()
            .into_iter()
            .filter_map(|agent| {
                let remote = agent.remote_execution()?;
                let slice_id = remote.worker_machine_id.strip_prefix("slice:")?.trim();
                let slice = slices.get(slice_id)?;
                let session_missing = !slice
                    .session_ids
                    .iter()
                    .any(|session_id| session_id == agent.session_id());
                let agent_missing = !slice
                    .agent_ids
                    .iter()
                    .any(|agent_id| agent_id == agent.id());
                (session_missing || agent_missing).then(|| crate::slice::SliceAgentAttachment {
                    slice_ref: slice.id.clone(),
                    session_id: agent.session_id().to_string(),
                    agent_id: agent.id().to_string(),
                })
            })
            .collect::<Vec<_>>();
        for slice in self
            .slices
            .attach_agents(attachments, crate::session::unix_epoch_ms())?
        {
            self.durable_state.append_event(
                "slice.updated",
                Some(slice.id.clone()),
                serde_json::json!({ "slice": &slice }),
            )?;
            crate::logging::info_with_fields(
                "durable_state.restore",
                "restored missing slice agent attachment",
                serde_json::json!({
                    "slice_id": slice.id,
                    "session_ids": slice.session_ids,
                    "agent_ids": slice.agent_ids,
                }),
            );
        }
        Ok(())
    }

    fn restore_durable_state_snapshot(
        &mut self,
        payload: serde_json::Value,
    ) -> Result<bool, DaemonError> {
        if durable_snapshot_belongs_to_other_kernel(&payload, &self.config.daemon_id) {
            return Ok(false);
        }
        let snapshot: DurableKernelSnapshotPayload =
            serde_json::from_value(payload).map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.restore_snapshot",
                message: error.to_string(),
            })?;
        if !self.snapshot_has_current_kernel_state(&snapshot) {
            return Ok(false);
        }
        let restored_session_ids: std::collections::BTreeSet<String> = snapshot
            .sessions
            .iter()
            .filter(|session| self.session_belongs_to_current_kernel(session))
            .map(|session| session.id().to_string())
            .collect();
        let mut prompt_private_states_by_session =
            BTreeMap::<String, Vec<DurablePromptPrivateState>>::new();
        for state in snapshot.prompt_private_states {
            prompt_private_states_by_session
                .entry(state.session_id.clone())
                .or_default()
                .push(state);
        }
        self.sessions.restore_projects(snapshot.projects);
        for mut session in snapshot.sessions {
            if !restored_session_ids.contains(session.id()) {
                continue;
            }
            if let Some(states) = prompt_private_states_by_session.get(session.id()) {
                session.restore_durable_prompt_private_states(states);
            }
            self.prompt_state_owner.restore_session_state(&session);
            self.restore_session_with_project_migration(session);
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
        Ok(true)
    }

    fn session_belongs_to_current_kernel(&self, session: &RuntimeSession) -> bool {
        session.host_daemon_id() == self.config.daemon_id
    }

    fn snapshot_has_current_kernel_state(&self, snapshot: &DurableKernelSnapshotPayload) -> bool {
        snapshot
            .sessions
            .iter()
            .any(|session| self.session_belongs_to_current_kernel(session))
            || snapshot
                .slices
                .iter()
                .any(|slice| slice.owner_kernel_id == self.config.daemon_id)
    }

    fn mark_agent_external_provider_sessions_attached(&self, agent: &AgentInstance) {
        if let Some(import) = agent.external_provider_import() {
            self.external_provider_sessions.mark_import_attached(
                import,
                agent.session_id(),
                agent.id(),
            );
        }
        self.external_provider_sessions.mark_resume_state_attached(
            agent.provider_resume_state(),
            agent.session_id(),
            agent.id(),
        );
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
        let mut attachment_state = RestoredExternalProviderAttachmentState::default();
        let replay_after_sequence = match store.latest_snapshot()? {
            Some(snapshot) => {
                let payload: DurableKernelSnapshotPayload =
                    serde_json::from_value(snapshot.payload).map_err(|error| {
                        DaemonError::LocalTransport {
                            operation: "durable_state.scan_external_provider_attachment_snapshot",
                            message: error.to_string(),
                        }
                    })?;
                attachment_state.restore_snapshot(payload);
                snapshot.sequence
            }
            None => 0,
        };
        let mut replay_cursor = replay_after_sequence;
        loop {
            let events =
                store.load_events_after_batch(replay_cursor, DURABLE_EVENT_REPLAY_BATCH_SIZE)?;
            if events.is_empty() {
                break;
            }
            for event in events {
                replay_cursor = event.sequence;
                attachment_state.apply_event(&event);
            }
        }
        let mut marker_count = 0usize;
        for agent in attachment_state.live_agents.values() {
            marker_count += self.mark_agent_external_provider_sessions_attached_counted(agent);
        }
        Ok(marker_count)
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
        count += self.external_provider_sessions.mark_resume_state_attached(
            agent.provider_resume_state(),
            agent.session_id(),
            agent.id(),
        );
        count
    }

    fn refresh_restored_session_projections(&self) -> Result<(), DaemonError> {
        let sessions = self.sessions.read().store().list();
        for mut session in sessions {
            let agents = self.agents.get_session_agents(session.id());
            session.set_agents(agents);
            self.prompt_state_owner.restore_session_state(&session);
            let session = self.restore_session_with_project_migration(session);
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
        self.prompt_state_owner.restore_session_state(&session);
        let session = self.restore_session_with_project_migration(session);
        self.update_session_projection(session);
        Ok(())
    }

    fn reconcile_restored_runtime_state_after_restart(&self) -> Result<(), DaemonError> {
        let sessions = self.sessions.read().store().list();
        let mut reconciled_runtime_state = false;
        for mut session in sessions {
            let reconciliation = session.reconcile_after_kernel_restart();
            if !reconciliation.changed() {
                continue;
            }
            reconciled_runtime_state = true;
            let agents = self.agents.get_session_agents(session.id());
            session.set_agents(agents);
            self.prompt_state_owner.restore_session_state(&session);
            let session = self.restore_session_with_project_migration(session);
            self.update_session_projection(session.clone());
            crate::logging::info_with_fields(
                "durable_state.restore",
                "reconciled runtime state after kernel restart",
                serde_json::json!({
                    "session_id": session.id(),
                    "cleared_active_provider_run": reconciliation.cleared_active_provider_run,
                    "cleared_attachment_count": reconciliation.cleared_attachment_count,
                    "recoverable_prompt_count": reconciliation.recoverable_prompt_count,
                    "recoverable_workflow_run_count": reconciliation.recoverable_workflow_run_count,
                    "interrupted_prompt_count": reconciliation.interrupted_prompt_count,
                    "stopped_workflow_run_count": reconciliation.stopped_workflow_run_count,
                }),
            );
        }
        if reconciled_runtime_state {
            self.save_durable_state_snapshot()?;
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

    fn restore_session_with_project_migration(&self, session: RuntimeSession) -> RuntimeSession {
        let project_name =
            crate::runtime::workspace_git_common::workspace_display_label(session.workspace_id());
        self.sessions
            .restore_session_with_default_project_name_hint(session, project_name.as_deref())
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

    fn restore_durable_state_event(
        &mut self,
        event: DurableStateEvent,
        diagnostics: &mut DurableRestoreDiagnostics,
    ) -> Result<(), DaemonError> {
        match event.kind.as_str() {
            "project.created" | "project.updated" => {
                let project: RuntimeProject = decode_durable_payload_field(
                    &event,
                    "project",
                    "durable_state.restore_project",
                )?;
                self.sessions.restore_projects(vec![project]);
            }
            "project.deleted" => {
                let project: RuntimeProject = decode_durable_payload_field(
                    &event,
                    "project",
                    "durable_state.restore_project_delete",
                )?;
                let _ = self
                    .sessions
                    .delete_project_record(project.id(), project.owner_user_id());
            }
            "session.created" => {
                if durable_payload_entity_belongs_to_other_owner(
                    &event,
                    "session",
                    "host_daemon_id",
                    &self.config.daemon_id,
                ) {
                    return Ok(());
                }
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
                self.prompt_state_owner.restore_session_state(&session);
                let session = self.restore_session_with_project_migration(session);
                self.agents.restore_agent(default_agent);
                self.update_session_projection(session);
            }
            DURABLE_PROMPT_STATE_EVENT_KIND => {
                let mut prompt_state =
                    serde_json::from_value::<DurablePromptStateEventPayload>(event.payload.clone())
                        .map_err(|error| DaemonError::LocalTransport {
                            operation: "durable_state.restore_prompt_state_event",
                            message: error.to_string(),
                        })?;
                let session = match self.sessions.get_session(&prompt_state.session_id) {
                    Ok(session) => session,
                    Err(DaemonError::SessionNotFound { .. }) => {
                        diagnostics.record_missing_prompt_session(prompt_state.session_id);
                        return Ok(());
                    }
                    Err(error) => return Err(error),
                };
                if !self.session_belongs_to_current_kernel(&session) {
                    return Ok(());
                }
                prompt_state.restore_private_states();
                if let Some(timestamp_ms) = prompt_state.last_prompt_sent_at_ms {
                    self.agents
                        .note_prompt_sent_at(&prompt_state.agent_id, timestamp_ms)?;
                    self.sessions.note_prompt_sent(
                        &prompt_state.session_id,
                        &prompt_state.agent_id,
                        timestamp_ms,
                    )?;
                }
                let session = self.sessions.mirror_agent_prompt_state(
                    &prompt_state.session_id,
                    &prompt_state.agent_id,
                    prompt_state.active_prompt,
                    prompt_state.queued_prompts,
                )?;
                self.prompt_state_owner.restore_session_state(&session);
                self.update_session_projection(session);
            }
            "session.updated" => {
                if durable_payload_entity_belongs_to_other_owner(
                    &event,
                    "session",
                    "host_daemon_id",
                    &self.config.daemon_id,
                ) {
                    return Ok(());
                }
                let mut session: RuntimeSession = decode_durable_payload_field(
                    &event,
                    "session",
                    "durable_state.restore_session_update",
                )?;
                if !self.session_belongs_to_current_kernel(&session) {
                    return Ok(());
                }
                let private_states =
                    if let Some(states) = event.payload.get("prompt_private_states") {
                        serde_json::from_value::<Vec<DurablePromptPrivateState>>(states.clone())
                            .map_err(|error| DaemonError::LocalTransport {
                                operation: "durable_state.restore_prompt_private_states",
                                message: error.to_string(),
                            })?
                    } else {
                        self.sessions
                            .get_session(session.id())
                            .map(|current| current.durable_prompt_private_states())
                            .unwrap_or_default()
                    };
                if !private_states.is_empty() {
                    session.restore_durable_prompt_private_states(&private_states);
                }
                let preserved_prompt_states = self
                    .sessions
                    .get_session(session.id())
                    .map(|current| current.prompt_states().clone())
                    .unwrap_or_default();
                let session = self.restore_session_with_project_migration(session);
                for (agent_id, prompt_state) in preserved_prompt_states {
                    if prompt_state.active_prompt().is_none()
                        && prompt_state.queued_prompts().is_empty()
                    {
                        continue;
                    }
                    self.sessions.mirror_agent_prompt_state(
                        session.id(),
                        &agent_id,
                        prompt_state.active_prompt().cloned(),
                        prompt_state.queued_prompts().clone(),
                    )?;
                }
                let session = self.sessions.get_session(session.id())?;
                self.prompt_state_owner.restore_session_state(&session);
                self.update_session_projection(session);
            }
            "sessions.updated" => {
                let sessions: Vec<RuntimeSession> = decode_durable_payload_field(
                    &event,
                    "sessions",
                    "durable_state.restore_session_batch_update",
                )?;
                for session in sessions {
                    if !self.session_belongs_to_current_kernel(&session) {
                        continue;
                    }
                    let preserved_prompt_states = self
                        .sessions
                        .get_session(session.id())
                        .map(|current| current.prompt_states().clone())
                        .unwrap_or_default();
                    self.sessions.restore_session(session.clone());
                    for (agent_id, prompt_state) in preserved_prompt_states {
                        if prompt_state.active_prompt().is_none()
                            && prompt_state.queued_prompts().is_empty()
                        {
                            continue;
                        }
                        self.sessions.mirror_agent_prompt_state(
                            session.id(),
                            &agent_id,
                            prompt_state.active_prompt().cloned(),
                            prompt_state.queued_prompts().clone(),
                        )?;
                    }
                    let session = self.sessions.get_session(session.id())?;
                    self.prompt_state_owner.restore_session_state(&session);
                    self.update_session_projection(session);
                }
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
                if durable_payload_entity_belongs_to_other_owner(
                    &event,
                    "session",
                    "host_daemon_id",
                    &self.config.daemon_id,
                ) {
                    return Ok(());
                }
                let mut session: RuntimeSession = decode_durable_payload_field(
                    &event,
                    "session",
                    "durable_state.restore_ended_session",
                )?;
                if !self.session_belongs_to_current_kernel(&session) {
                    return Ok(());
                }
                self.external_provider_sessions.detach_session(session.id());
                self.attached_provider_transcript_cursors
                    .detach_session(session.id());
                self.prompt_state_owner.remove_session(session.id());
                self.agents.remove_session_agents(session.id());
                session.set_agents(Vec::new());
                let session = self.restore_session_with_project_migration(session);
                self.update_session_projection(session);
            }
            "session.deleted" => {
                if durable_payload_entity_belongs_to_other_owner(
                    &event,
                    "session",
                    "host_daemon_id",
                    &self.config.daemon_id,
                ) {
                    return Ok(());
                }
                let mut session: RuntimeSession = decode_durable_payload_field(
                    &event,
                    "session",
                    "durable_state.restore_deleted_session",
                )?;
                if !self.session_belongs_to_current_kernel(&session) {
                    return Ok(());
                }
                self.external_provider_sessions.detach_session(session.id());
                self.attached_provider_transcript_cursors
                    .detach_session(session.id());
                self.prompt_state_owner.remove_session(session.id());
                self.agents.remove_session_agents(session.id());
                session.set_agents(Vec::new());
                self.sessions.remove_restored_session(session.id());
                self.session_projection.remove(session.id());
                self.agent_runtime_projection.update_session(&session);
            }
            "agent.deleted" => {
                let agent: AgentInstance = decode_durable_payload_field(
                    &event,
                    "agent",
                    "durable_state.restore_deleted_agent",
                )?;
                let session_id = agent.session_id().to_string();
                self.external_provider_sessions
                    .detach_agent(&session_id, agent.id());
                self.attached_provider_transcript_cursors
                    .detach_agent(&session_id, agent.id());
                self.prompt_state_owner
                    .remove_agent(&session_id, agent.id());
                if self.sessions.get_session(&session_id).is_ok() {
                    let session_store = self.session_state_store();
                    let mut sessions = session_store.write();
                    let _ = self.agents.destroy_agent(agent.id(), &mut sessions);
                    drop(sessions);
                    self.refresh_restored_agent_session_projection(&session_id)?;
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durable_restore_merges_linked_worktree_default_project_into_main_workspace() {
        let config = crate::config::DaemonConfig::for_tests();
        let repo = temp_git_repo("durable-default-project-workspace");
        let worktree = repo.with_file_name(format!(
            "{}-linked",
            repo.file_name().and_then(|name| name.to_str()).unwrap()
        ));
        run_git(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "feature/durable-default-project-workspace",
                worktree.to_str().unwrap(),
            ],
        );
        let repo_id = std::fs::canonicalize(&repo).unwrap().display().to_string();
        let worktree_id = std::fs::canonicalize(&worktree)
            .unwrap()
            .display()
            .to_string();
        let main_project = crate::session::RuntimeProject::new(
            "project-main",
            crate::session::DEFAULT_LOCAL_USER_ID,
            &repo_id,
            "owner/repo",
            crate::session::RuntimeProjectKind::Default,
        );
        let linked_project = crate::session::RuntimeProject::new(
            "project-linked",
            crate::session::DEFAULT_LOCAL_USER_ID,
            &worktree_id,
            "owner/repo",
            crate::session::RuntimeProjectKind::Default,
        );
        let mut main_session = crate::session::RuntimeSession::new(
            "session-main",
            Some("main-session".to_string()),
            &repo_id,
            &repo_id,
            config.host_machine_id.clone(),
            config.daemon_id.clone(),
        );
        assert!(main_session.assign_project_id(main_project.id()));
        let mut linked_session = crate::session::RuntimeSession::new(
            "session-linked",
            Some("linked-session".to_string()),
            &worktree_id,
            &worktree_id,
            config.host_machine_id.clone(),
            config.daemon_id.clone(),
        );
        assert!(linked_session.assign_project_id(linked_project.id()));
        linked_session.set_active_provider_run(Some("legacy-provider-run".to_string()));
        let main_agent = default_agent_for_session(&main_session, "agent-main");
        let linked_agent = default_agent_for_session(&linked_session, "agent-linked");
        {
            let app = DaemonApp::bootstrap(config.clone()).expect("daemon should boot");
            for project in [&main_project, &linked_project] {
                app.durable_state_store()
                    .append_event(
                        "project.created",
                        Some(project.id().to_string()),
                        serde_json::json!({ "project": project }),
                    )
                    .expect("legacy project should persist");
            }
            for (session, agent, project) in [
                (&main_session, &main_agent, &main_project),
                (&linked_session, &linked_agent, &linked_project),
            ] {
                app.durable_state_store()
                    .append_event(
                        "session.created",
                        Some(session.id().to_string()),
                        serde_json::json!({
                            "session": session,
                            "default_agent": agent,
                            "project": project,
                        }),
                    )
                    .expect("legacy session should persist");
            }
        }

        let app = DaemonApp::bootstrap(config.clone()).expect("daemon should migrate");
        let migrated = app
            .sessions()
            .get_session(linked_session.id())
            .expect("linked session should restore");
        assert_eq!(migrated.workspace_id(), repo_id);
        assert_eq!(migrated.worktree_id(), worktree_id);
        assert_eq!(migrated.project_id(), main_project.id());
        assert!(app.sessions().get_project(linked_project.id()).is_err());
        assert_eq!(
            app.sessions()
                .list_projects(crate::session::DEFAULT_LOCAL_USER_ID, true)
                .len(),
            1
        );
        drop(app);

        let app = DaemonApp::bootstrap(config).expect("migration should survive restart");
        let migrated = app
            .sessions()
            .get_session(linked_session.id())
            .expect("linked session should survive restart");
        assert_eq!(migrated.workspace_id(), repo_id);
        assert_eq!(migrated.worktree_id(), worktree_id);
        assert_eq!(migrated.project_id(), main_project.id());
        assert!(app.sessions().get_project(linked_project.id()).is_err());
        drop(app);

        let _ = std::fs::remove_dir_all(&worktree);
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn later_session_snapshot_preserves_newer_durable_prompt_queue() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-prompt-order",
                "worktree-prompt-order",
            ))
            .expect("session should create");
        let queued = crate::session::PromptQueueItem::new(
            "prompt-newer-than-snapshot",
            "attachment-prompt-order",
            agent.id(),
            "preserve me",
            crate::session::PromptStatus::Queued,
        );
        app.sessions_mut()
            .mirror_agent_prompt_state(
                session.id(),
                agent.id(),
                None,
                std::collections::VecDeque::from([queued]),
            )
            .expect("newer prompt state should mirror");

        let stale_session = session.clone();
        let event = DurableStateEvent {
            sequence: 2,
            event_id: "event-stale-session-update".to_string(),
            kind: "session.updated".to_string(),
            subject_id: Some(session.id().to_string()),
            timestamp_ms: crate::session::unix_epoch_ms(),
            payload: serde_json::json!({"session": stale_session}),
        };
        let mut diagnostics = DurableRestoreDiagnostics::default();
        app.restore_durable_state_event(event, &mut diagnostics)
            .expect("stale session snapshot should restore");

        let restored = app
            .sessions()
            .get_session(session.id())
            .expect("session should remain available");
        assert_eq!(
            restored
                .queued_prompts_for_agent(agent.id())
                .and_then(|prompts| prompts.front())
                .map(|prompt| prompt.id()),
            Some("prompt-newer-than-snapshot")
        );
    }

    #[test]
    fn durable_restore_removes_project_without_visible_sessions() {
        let config = crate::config::DaemonConfig::for_tests();
        let project = crate::session::RuntimeProject::new(
            "legacy-empty-project",
            crate::session::DEFAULT_LOCAL_USER_ID,
            "legacy-workspace",
            "Legacy empty project",
            crate::session::RuntimeProjectKind::Default,
        );
        let mut hidden_session = crate::session::RuntimeSession::new(
            "legacy-publication-runtime",
            None,
            "legacy-workspace",
            "legacy-worktree",
            config.host_machine_id.clone(),
            config.daemon_id.clone(),
        );
        hidden_session.set_hidden(true);
        assert!(hidden_session.assign_project_id(project.id()));
        let default_agent = crate::agent::AgentInstance::new(
            "legacy-publication-agent",
            "legacy-publication-agent-ref",
            hidden_session.id(),
            None,
            "dev-stub",
            None,
            None,
            None,
            crate::agent::GridPosition::new(0, 0, 1, 1),
        );
        {
            let app = DaemonApp::bootstrap(config.clone()).expect("daemon should boot");
            app.durable_state_store()
                .append_event(
                    "project.created",
                    Some(project.id().to_string()),
                    serde_json::json!({ "project": &project }),
                )
                .expect("legacy empty project should persist");
            app.durable_state_store()
                .append_event(
                    "session.created",
                    Some(hidden_session.id().to_string()),
                    serde_json::json!({
                        "session": &hidden_session,
                        "default_agent": &default_agent,
                        "project": &project,
                    }),
                )
                .expect("legacy hidden publication runtime should persist");
        }

        let app = DaemonApp::bootstrap(config.clone()).expect("daemon should restore");
        assert!(app.sessions().get_project(project.id()).is_err());
        assert_eq!(
            app.sessions()
                .get_session(hidden_session.id())
                .expect("hidden publication runtime should remain")
                .project_id(),
            ""
        );
        drop(app);

        let app = DaemonApp::bootstrap(config).expect("daemon should restore cleanup event");
        assert!(app.sessions().get_project(project.id()).is_err());
        assert_eq!(
            app.sessions()
                .get_session(hidden_session.id())
                .expect("hidden publication runtime should survive cleanup restart")
                .project_id(),
            ""
        );
    }

    #[test]
    fn restart_reconciliation_restores_missing_slice_agent_ids_from_worker_placement() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace",
                "worktree",
            ))
            .expect("session should create");
        let slice = app
            .slices()
            .create(
                &app.config().daemon_id,
                &app.config().host_machine_id,
                crate::slice::CreateSliceInput {
                    name: "restored-slice".to_string(),
                    backend: crate::slice::SliceBackendKind::LocalDocker,
                    os: "linux".to_string(),
                    display_mode: crate::slice::SliceDisplayMode::Headed,
                    workspace_id: None,
                    worktree_id: None,
                    workspace_mount: None,
                    worker_kernel_ref: None,
                    display_url: None,
                    provider_auth: Vec::new(),
                    from_saved_state: None,
                    now_ms: 1,
                },
            )
            .expect("slice should create");
        app.agents()
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: "worker-kernel".to_string(),
                    worker_machine_id: format!("slice:{}", slice.id),
                    execution_lease_id: "lease-1".to_string(),
                    leased_agent_id: "leased-agent-1".to_string(),
                    active_worker_provider_run_id: None,
                    relay_url: None,
                    relay_token: None,
                },
            )
            .expect("agent should bind to slice worker");

        app.reconcile_restored_slice_agent_attachments()
            .expect("restart reconciliation should succeed");

        let restored = app
            .slices()
            .resolve(&slice.id)
            .expect("slice should remain available");
        assert_eq!(restored.session_ids, vec![session.id().to_string()]);
        assert_eq!(restored.agent_ids, vec![agent.id().to_string()]);
    }

    fn default_agent_for_session(
        session: &crate::session::RuntimeSession,
        agent_id: &str,
    ) -> crate::agent::AgentInstance {
        crate::agent::AgentInstance::new(
            agent_id,
            agent_id,
            session.id(),
            None,
            "dev-stub",
            None,
            None,
            None,
            crate::agent::GridPosition::new(0, 0, 1, 1),
        )
    }

    fn temp_git_repo(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "arroba-{label}-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&root).expect("temp repo should be created");
        run_git(&root, &["init"]);
        run_git(&root, &["config", "user.email", "tests@example.invalid"]);
        run_git(&root, &["config", "user.name", "Arroba Tests"]);
        std::fs::write(root.join("README.md"), "durable workspace migration\n")
            .expect("fixture should be written");
        run_git(&root, &["add", "README.md"]);
        run_git(&root, &["commit", "-m", "initial"]);
        root
    }

    fn run_git(cwd: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("git command should run");
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
