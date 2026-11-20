use super::{decode_durable_payload_field, DaemonApp, DURABLE_EVENT_REPLAY_BATCH_SIZE};
use crate::agent::{AgentInstance, AgentService, AgentServiceStore};
use crate::app::{AttachedProviderTranscriptCursorStore, ExternalProviderSessionIndexStore};
use crate::durable_prompt_state::{
    DurablePromptStateEventPayload, DURABLE_PROMPT_STATE_EVENT_KIND,
};
use crate::durable_snapshot::DurableKernelSnapshotPayload;
use crate::durable_state::{DurableKernelStateStore, DurableStateEvent, ReplayFailure};
use crate::error::DaemonError;
use crate::runtime::metaagent_event::{
    MetaagentEventRecord, MetaagentEventSnapshot, MetaagentEventStore, MetaagentEventSubscription,
};
use crate::runtime::projection::{AgentRuntimeProjectionStore, SessionStateProjectionStore};
use crate::runtime::prompt_state::PromptStateOwner;
use crate::session::{
    DurablePromptPrivateState, RuntimeProject, RuntimeSession, SessionService, SessionStateStore,
};
use crate::slice::SliceStore;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::time::Instant;

const DURABLE_RESTORE_DIAGNOSTIC_SAMPLE_LIMIT: usize = 20;

#[derive(Default)]
pub(super) struct DurableRestoreDiagnostics {
    // Compaction can leave activity before the first retained session record.
    session_activity_by_id: BTreeMap<String, u64>,
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

pub(super) struct ReplayStats {
    pub(super) replay_after_sequence: u64,
    pub(super) snapshot_restore_ms: u128,
    pub(super) replayed_event_count: usize,
    pub(super) replay_ms: u128,
}

pub(super) struct ReplayState<'a> {
    daemon_id: &'a str,
    sessions: &'a SessionStateStore,
    agents: &'a AgentServiceStore,
    slices: &'a SliceStore,
    prompt_state_owner: &'a PromptStateOwner,
    room_computer_revoked: &'a Mutex<BTreeSet<String>>,
    metaagent_events: &'a MetaagentEventStore,
    session_projection: &'a SessionStateProjectionStore,
    agent_runtime_projection: &'a AgentRuntimeProjectionStore,
    external_provider_sessions: &'a ExternalProviderSessionIndexStore,
    attached_provider_transcript_cursors: &'a AttachedProviderTranscriptCursorStore,
    app: Option<&'a DaemonApp>,
}

pub(in crate::app) fn check_durable_replay(
    daemon_id: &str,
    store: &DurableKernelStateStore,
) -> Result<(), ReplayFailure> {
    ReplayState {
        daemon_id,
        sessions: &SessionStateStore::new(
            SessionService::for_durable_replay(daemon_id).with_durability(store.clone()),
        ),
        agents: &AgentServiceStore::new(AgentService::new().with_durable_state(store.clone())),
        slices: &SliceStore::default(),
        prompt_state_owner: &PromptStateOwner::default(),
        room_computer_revoked: &Mutex::default(),
        metaagent_events: &MetaagentEventStore::default(),
        session_projection: &SessionStateProjectionStore::default(),
        agent_runtime_projection: &AgentRuntimeProjectionStore::default(),
        external_provider_sessions: &ExternalProviderSessionIndexStore::default(),
        attached_provider_transcript_cursors: &AttachedProviderTranscriptCursorStore::default(),
        app: None,
    }
    .replay(store)?;
    Ok(())
}

impl<'a> ReplayState<'a> {
    pub(super) fn for_startup(app: &'a DaemonApp) -> Self {
        Self {
            daemon_id: &app.config.daemon_id,
            sessions: &app.sessions,
            agents: &app.agents,
            slices: &app.slices,
            prompt_state_owner: &app.prompt_state_owner,
            room_computer_revoked: &app.room_computer_revoked,
            metaagent_events: &app.metaagent_events,
            session_projection: &app.session_projection,
            agent_runtime_projection: &app.agent_runtime_projection,
            external_provider_sessions: &app.external_provider_sessions,
            attached_provider_transcript_cursors: &app.attached_provider_transcript_cursors,
            app: Some(app),
        }
    }

    fn update_session_projection(&self, session: RuntimeSession) {
        if let Some(app) = self.app {
            app.update_session_projection(session);
        }
    }

    fn mark_agent_external_provider_sessions_attached(&self, agent: &AgentInstance) {
        if let Some(app) = self.app {
            app.mark_agent_external_provider_sessions_attached(agent);
        }
    }

    pub(super) fn replay(
        &self,
        durable_state: &DurableKernelStateStore,
    ) -> Result<ReplayStats, ReplayFailure> {
        let snapshot_started = Instant::now();
        let snapshot = durable_state.latest_snapshot_for_owner(self.daemon_id)?;
        // Populate historical Room lookup before the live-only restore drops
        // ended checkpoint records. Replay ordering is part of the migration.
        durable_state.backfill_retired_sessions(self.daemon_id, snapshot.as_ref())?;
        let replay_after_sequence = match snapshot {
            Some(snapshot) => {
                if self
                    .restore_durable_state_snapshot(snapshot.payload)
                    .map_err(|error| ReplayFailure::at_snapshot(snapshot.sequence, error))?
                {
                    snapshot.sequence
                } else {
                    crate::logging::info_with_fields(
                        "durable_state.restore",
                        "ignored durable snapshot for another kernel owner",
                        serde_json::json!({
                            "snapshot_sequence": snapshot.sequence,
                            "daemon_id": self.daemon_id,
                        }),
                    );
                    0
                }
            }
            None => 0,
        };
        let snapshot_restore_ms = snapshot_started.elapsed().as_millis();
        let replay_started = Instant::now();
        let mut diagnostics = DurableRestoreDiagnostics::default();
        let mut replay_cursor = replay_after_sequence;
        let mut replayed_event_count = 0usize;
        loop {
            let events = durable_state
                .load_restore_events_after_batch(replay_cursor, DURABLE_EVENT_REPLAY_BATCH_SIZE)?;
            if events.is_empty() {
                break;
            }
            for event in events {
                replay_cursor = replay_cursor.max(event.sequence);
                self.restore_durable_state_event(&event, &mut diagnostics)
                    .map_err(|error| {
                        ReplayFailure::at_event(event.kind, event.event_id, event.sequence, error)
                    })?;
                replayed_event_count += 1;
            }
        }
        let replay_ms = replay_started.elapsed().as_millis();
        diagnostics.log_summary();

        Ok(ReplayStats {
            replay_after_sequence,
            snapshot_restore_ms,
            replayed_event_count,
            replay_ms,
        })
    }

    fn restore_durable_state_snapshot(
        &self,
        payload: serde_json::Value,
    ) -> Result<bool, DaemonError> {
        if durable_snapshot_belongs_to_other_kernel(&payload, self.daemon_id) {
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
            .filter(|session| session.status() != crate::session::SessionStatus::Ended)
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
            if !restored_session_ids.contains(session.id())
                || session.status() == crate::session::SessionStatus::Ended
            {
                continue;
            }
            if let Some(states) = prompt_private_states_by_session.get(session.id()) {
                session.restore_durable_prompt_private_states(states);
            }
            self.prompt_state_owner.restore_session_state(&session);
            self.sessions.restore_session(session);
        }
        let restored_slices = snapshot
            .slices
            .into_iter()
            .filter(|slice| slice.owner_kernel_id == self.daemon_id)
            .collect::<Vec<_>>();
        let restored_slice_ids = restored_slices
            .iter()
            .map(|slice| slice.id.clone())
            .collect::<BTreeSet<_>>();
        self.slices.restore_records(restored_slices);
        self.slices
            .restore_saved_state_records(snapshot.slice_saved_states, snapshot.slice_backups);
        self.slices.restore_pending_backup_restore_records(
            snapshot
                .pending_slice_backup_restores
                .into_iter()
                .filter(|transaction| restored_slice_ids.contains(&transaction.source_slice_id))
                .collect(),
        );
        self.slices.restore_pending_restore_acknowledgement_records(
            snapshot
                .pending_slice_backup_restore_acknowledgements
                .into_iter()
                .filter(|pending| restored_slice_ids.contains(&pending.source_slice_id))
                .collect(),
        );
        let mut restored_agent_ids = std::collections::BTreeSet::<String>::new();
        for agent in snapshot.agents {
            if !restored_session_ids.contains(agent.session_id()) {
                continue;
            }
            restored_agent_ids.insert(agent.id().to_string());
            self.mark_agent_external_provider_sessions_attached(&agent);
            self.agents.restore_agent(agent);
        }
        *self
            .room_computer_revoked
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = snapshot
            .room_computer_revoked
            .into_iter()
            .filter(|id| restored_agent_ids.contains(id))
            .collect();
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
        session.host_daemon_id() == self.daemon_id
    }

    fn snapshot_has_current_kernel_state(&self, snapshot: &DurableKernelSnapshotPayload) -> bool {
        snapshot
            .sessions
            .iter()
            .any(|session| self.session_belongs_to_current_kernel(session))
            || snapshot
                .slices
                .iter()
                .any(|slice| slice.owner_kernel_id == self.daemon_id)
    }

    fn refresh_restored_session_projections(&self) -> Result<(), DaemonError> {
        let sessions = self.sessions.read().store().list();
        for mut session in sessions {
            let agents = self.agents.get_session_agents(session.id());
            session.set_agents(agents);
            self.prompt_state_owner.restore_session_state(&session);
            let session = self.sessions.restore_session(session);
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
        let session = self.sessions.restore_session(session);
        self.update_session_projection(session);
        Ok(())
    }

    pub(super) fn restore_durable_state_event(
        &self,
        event: &DurableStateEvent,
        diagnostics: &mut DurableRestoreDiagnostics,
    ) -> Result<(), DaemonError> {
        match event.kind.as_str() {
            "project.created" | "project.updated" => {
                let project: RuntimeProject = decode_durable_payload_field(
                    event,
                    "project",
                    "durable_state.restore_project",
                )?;
                self.sessions.restore_projects(vec![project]);
            }
            "project.deleted" => {
                let project: RuntimeProject = decode_durable_payload_field(
                    event,
                    "project",
                    "durable_state.restore_project_delete",
                )?;
                let _ = self
                    .sessions
                    .delete_project_record(project.id(), project.owner_user_id());
            }
            "session.created" => {
                if durable_payload_entity_belongs_to_other_owner(
                    event,
                    "session",
                    "host_daemon_id",
                    self.daemon_id,
                ) {
                    return Ok(());
                }
                let session: RuntimeSession = decode_durable_payload_field(
                    event,
                    "session",
                    "durable_state.restore_session",
                )?;
                if !self.session_belongs_to_current_kernel(&session) {
                    return Ok(());
                }
                let default_agent: AgentInstance = decode_durable_payload_field(
                    event,
                    "default_agent",
                    "durable_state.restore_default_agent",
                )?;
                self.mark_agent_external_provider_sessions_attached(&default_agent);
                self.prompt_state_owner.restore_session_state(&session);
                let session = self.sessions.restore_session(session);
                self.agents.restore_agent(default_agent);
                self.update_session_projection(session);
            }
            "session.activity.updated" => {
                let session_id =
                    event
                        .subject_id
                        .as_deref()
                        .ok_or_else(|| DaemonError::LocalTransport {
                            operation: "durable_state.restore_session_activity",
                            message: "Session activity has no session id".into(),
                        })?;
                let timestamp_ms =
                    event.payload["last_activity_at_ms"]
                        .as_u64()
                        .ok_or_else(|| DaemonError::LocalTransport {
                            operation: "durable_state.restore_session_activity",
                            message: "Session activity has no timestamp".into(),
                        })?;
                diagnostics
                    .session_activity_by_id
                    .entry(session_id.to_owned())
                    .and_modify(|previous| *previous = (*previous).max(timestamp_ms))
                    .or_insert(timestamp_ms);
                if let Ok(session) = self.sessions.get_session(session_id) {
                    if self.session_belongs_to_current_kernel(&session)
                        && session.status() != crate::session::SessionStatus::Ended
                    {
                        let session = self
                            .sessions
                            .note_session_activity(session_id, timestamp_ms)?;
                        self.update_session_projection(session);
                    }
                }
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
                match self.agents.get_agent(&prompt_state.agent_id) {
                    Ok(_) => {}
                    Err(DaemonError::AgentNotFound { .. })
                        if prompt_state.active_prompt.is_none()
                            && prompt_state.queued_prompts.is_empty() =>
                    {
                        // Deletion wins over late empty queue cleanup, including
                        // the removed agent's cached last-sent timestamp.
                        return Ok(());
                    }
                    Err(error) => return Err(error),
                }
                self.sessions.note_session_activity(
                    &prompt_state.session_id,
                    prompt_state.last_activity_at_ms,
                )?;
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
            "workflow.runtime.updated" => {}
            "workflow.publication.materialized" | "workflow.publication.reconfigured" => {
                let session: RuntimeSession = decode_durable_payload_field(
                    event,
                    "session",
                    "durable_state.restore_publication_materialization",
                )?;
                if !self.session_belongs_to_current_kernel(&session) {
                    return Ok(());
                }
                for agent in session.agents() {
                    if agent.session_id() != session.id()
                        || agent.owner_user_id() != session.owner_user_id()
                    {
                        return Err(DaemonError::LocalTransport {
                            operation: "durable_state.restore_publication_materialization",
                            message: "publication agent ownership does not match its session"
                                .to_string(),
                        });
                    }
                }
                for agent in session.agents() {
                    self.agents.restore_agent(agent.clone());
                }
                self.prompt_state_owner.restore_session_state(&session);
                let session = self.sessions.restore_session(session);
                self.update_session_projection(session);
            }
            "session.updated" => {
                if durable_payload_entity_belongs_to_other_owner(
                    event,
                    "session",
                    "host_daemon_id",
                    self.daemon_id,
                ) {
                    return Ok(());
                }
                let mut session: RuntimeSession = decode_durable_payload_field(
                    event,
                    "session",
                    "durable_state.restore_session_update",
                )?;
                if !self.session_belongs_to_current_kernel(&session) {
                    return Ok(());
                }
                if let Some(timestamp_ms) = diagnostics.session_activity_by_id.get(session.id()) {
                    session.note_activity_at(*timestamp_ms);
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
                let session = self.sessions.restore_session(session);
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
                    event,
                    "sessions",
                    "durable_state.restore_session_batch_update",
                )?;
                for mut session in sessions {
                    if !self.session_belongs_to_current_kernel(&session) {
                        continue;
                    }
                    // MP-08 / MP-10 / MP-11: public batch snapshots cannot
                    // replace queued user skill intent with client authority.
                    let private = self
                        .sessions
                        .get_session(session.id())
                        .map(|current| current.durable_prompt_private_states())
                        .unwrap_or_default();
                    session.restore_durable_prompt_private_states(&private);
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
            "agent.room_computer_access_updated" => {
                let owner: String = decode_durable_payload_field(
                    event,
                    "owner_id",
                    "durable_state.restore_computer_access",
                )?;
                if owner != self.daemon_id {
                    return Ok(());
                }
                let ids: Vec<String> = decode_durable_payload_field(
                    event,
                    "agent_ids",
                    "durable_state.restore_computer_access",
                )?;
                let allowed: bool = decode_durable_payload_field(
                    event,
                    "allowed",
                    "durable_state.restore_computer_access",
                )?;
                let mut revoked = self
                    .room_computer_revoked
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                for id in ids {
                    if self.agents.get_agent(&id).is_err() {
                        continue;
                    }
                    if allowed {
                        revoked.remove(&id);
                    } else {
                        revoked.insert(id);
                    }
                }
            }
            "agent.created" => {
                let agent: AgentInstance =
                    decode_durable_payload_field(event, "agent", "durable_state.restore_agent")?;
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
                    event,
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
                    event,
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
                    event,
                    "session",
                    "host_daemon_id",
                    self.daemon_id,
                ) {
                    return Ok(());
                }
                let session: RuntimeSession = decode_durable_payload_field(
                    event,
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
                if let Some(app) = self.app {
                    app.forget_room_computer_session_access(session.id());
                }
                self.agents.remove_session_agents(session.id());
                self.sessions.remove_restored_session(session.id());
                self.session_projection.remove(session.id());
                self.agent_runtime_projection.remove_session(session.id());
            }
            "session.deleted" => {
                if durable_payload_entity_belongs_to_other_owner(
                    event,
                    "session",
                    "host_daemon_id",
                    self.daemon_id,
                ) {
                    return Ok(());
                }
                let mut session: RuntimeSession = decode_durable_payload_field(
                    event,
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
                if let Some(app) = self.app {
                    app.forget_room_computer_session_access(session.id());
                }
                self.agents.remove_session_agents(session.id());
                session.set_agents(Vec::new());
                self.sessions.remove_restored_session(session.id());
                self.session_projection.remove(session.id());
                self.agent_runtime_projection.update_session(&session);
            }
            "agent.deleted" => {
                let agent: AgentInstance = decode_durable_payload_field(
                    event,
                    "agent",
                    "durable_state.restore_deleted_agent",
                )?;
                let session_id = agent.session_id().to_string();
                if let Some(app) = self.app {
                    app.forget_room_computer_access(agent.id());
                }
                self.external_provider_sessions
                    .detach_agent(&session_id, agent.id());
                self.attached_provider_transcript_cursors
                    .detach_agent(&session_id, agent.id());
                self.prompt_state_owner
                    .remove_agent(&session_id, agent.id());
                if self.sessions.get_session(&session_id).is_ok() {
                    let session_store = self.sessions;
                    let mut sessions = session_store.write();
                    let _ = self.agents.destroy_agent(agent.id(), &mut sessions);
                    drop(sessions);
                    self.refresh_restored_agent_session_projection(&session_id)?;
                }
            }
            "slice.created" | "slice.updated" => {
                let slice: crate::slice::SliceRecord =
                    decode_durable_payload_field(event, "slice", "durable_state.restore_slice")?;
                if slice.owner_kernel_id == self.daemon_id {
                    let mut slices = self.slices.list();
                    slices.retain(|record| record.id != slice.id);
                    slices.push(slice);
                    self.slices.restore_records(slices);
                }
            }
            "slice.deleted" => {
                let slice: crate::slice::SliceRecord = decode_durable_payload_field(
                    event,
                    "slice",
                    "durable_state.restore_deleted_slice",
                )?;
                if slice.owner_kernel_id == self.daemon_id {
                    let mut slices = self.slices.list();
                    slices.retain(|record| record.id != slice.id);
                    self.slices.restore_records(slices);
                }
            }
            "slice.state.saved" => {
                if event.payload.get("slice").is_some() {
                    let slice: crate::slice::SliceRecord = decode_durable_payload_field(
                        event,
                        "slice",
                        "durable_state.restore_slice_saved_state_slice",
                    )?;
                    if slice.owner_kernel_id == self.daemon_id {
                        let mut slices = self.slices.list();
                        slices.retain(|record| record.id != slice.id);
                        slices.push(slice);
                        self.slices.restore_records(slices);
                    }
                }
                let state: crate::slice::SliceSavedStateRecord = decode_durable_payload_field(
                    event,
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
                    event,
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
                    event,
                    "backup",
                    "durable_state.restore_slice_backup",
                )?;
                let mut backups = self.slices.list_backups();
                backups.retain(|record| record.id != backup.id);
                backups.push(backup);
                self.slices
                    .restore_saved_state_records(self.slices.list_saved_states(), backups);
            }
            "slice.backup.restore.started" => {
                let slice: crate::slice::SliceRecord = decode_durable_payload_field(
                    event,
                    "slice",
                    "durable_state.restore_slice_backup_restore_started_slice",
                )?;
                if slice.owner_kernel_id == self.daemon_id {
                    let transaction: crate::slice::SliceBackupRestoreTransactionRecord =
                        decode_durable_payload_field(
                            event,
                            "transaction",
                            "durable_state.restore_slice_backup_restore_started",
                        )?;
                    self.slices.replay_backup_restore_started(transaction);
                }
            }
            "slice.backup.restore.committed" | "slice.backup.restore.rolled_back" => {
                let slice: crate::slice::SliceRecord = decode_durable_payload_field(
                    event,
                    "slice",
                    "durable_state.restore_slice_backup_restore_resolution_slice",
                )?;
                if slice.owner_kernel_id == self.daemon_id {
                    let transaction_id: String = decode_durable_payload_field(
                        event,
                        "transaction_id",
                        "durable_state.restore_slice_backup_restore_resolution_id",
                    )?;
                    let state: crate::slice::SliceSavedStateRecord = decode_durable_payload_field(
                        event,
                        "state",
                        "durable_state.restore_slice_backup_restore_resolution_state",
                    )?;
                    // Legacy resolutions predate durable acknowledgements.
                    let acknowledgement = if event.payload.get("acknowledgement").is_some() {
                        decode_durable_payload_field(
                            event,
                            "acknowledgement",
                            "durable_state.restore_slice_backup_restore_acknowledgement",
                        )?
                    } else {
                        None
                    };
                    self.slices.replay_backup_restore_resolution(
                        &transaction_id,
                        slice,
                        state,
                        acknowledgement,
                    );
                }
            }
            "slice.backup.restore.acknowledged" => {
                let transaction_id: String = decode_durable_payload_field(
                    event,
                    "transaction_id",
                    "durable_state.restore_slice_backup_restore_acknowledged",
                )?;
                self.slices
                    .replay_backup_restore_acknowledged(&transaction_id);
            }
            "metaagent.event.recorded"
            | "metaagent.event.read"
            | "metaagent.event.acked"
            | "metaagent.event.delivery_updated" => {
                let record: MetaagentEventRecord = decode_durable_payload_field(
                    event,
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
                    event,
                    "subscription",
                    "durable_state.restore_metaagent_subscription",
                )?;
                if self.agents.get_agent(&subscription.metaagent_id).is_ok() {
                    self.metaagent_events.restore_subscription(subscription);
                }
            }
            "metaagent.subscription.deleted" => {
                let subscription: MetaagentEventSubscription = decode_durable_payload_field(
                    event,
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
