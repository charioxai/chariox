use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::time::sleep;

use crate::agent::{AgentInstance, AgentServiceStore};
use crate::durable_state::{DurableCheckpointEntity, DurableKernelStateStore};
use crate::error::DaemonError;
use crate::runtime::metaagent_event::{
    MetaagentEventRecord, MetaagentEventStore, MetaagentEventSubscription,
};
use crate::session::{
    DurablePromptPrivateState, RuntimeProject, RuntimeSession, SessionStateStore,
};
use crate::slice::{SliceBackupRecord, SliceRecord, SliceSavedStateRecord, SliceStore};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DurableKernelSnapshotPayload {
    #[serde(default)]
    pub(crate) projects: Vec<RuntimeProject>,
    pub(crate) sessions: Vec<RuntimeSession>,
    #[serde(default)]
    pub(crate) prompt_private_states: Vec<DurablePromptPrivateState>,
    pub(crate) agents: Vec<AgentInstance>,
    #[serde(default)]
    pub(crate) slices: Vec<SliceRecord>,
    #[serde(default)]
    pub(crate) slice_saved_states: Vec<SliceSavedStateRecord>,
    #[serde(default)]
    pub(crate) slice_backups: Vec<SliceBackupRecord>,
    #[serde(default)]
    pub(crate) metaagent_event_records: Vec<MetaagentEventRecord>,
    #[serde(default)]
    pub(crate) metaagent_event_subscriptions: Vec<MetaagentEventSubscription>,
}

impl DurableKernelSnapshotPayload {
    pub(crate) fn capture(
        sessions: &SessionStateStore,
        agents: &AgentServiceStore,
        slices: &SliceStore,
        metaagent_events: &MetaagentEventStore,
    ) -> Self {
        let projects = sessions.read().durable_projects();
        let durable_sessions = sessions.read().durable_sessions();
        let durable_session_ids = durable_sessions
            .iter()
            .map(|session| session.id().to_string())
            .collect::<std::collections::BTreeSet<_>>();
        let prompt_private_states = durable_sessions
            .iter()
            .flat_map(RuntimeSession::durable_prompt_private_states)
            .collect();
        let agents = agents
            .list_agents()
            .into_iter()
            .filter(|agent| durable_session_ids.contains(agent.session_id()))
            .collect();
        let slice_records = slices.list();
        let slice_saved_states = slices.list_saved_states();
        let slice_backups = slices.list_backups();
        let metaagent_snapshot = metaagent_events.snapshot();
        Self {
            projects,
            sessions: durable_sessions,
            prompt_private_states,
            agents,
            slices: slice_records,
            slice_saved_states,
            slice_backups,
            metaagent_event_records: metaagent_snapshot.records,
            metaagent_event_subscriptions: metaagent_snapshot.subscriptions,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DurableSnapshotTickOutcome {
    pub(crate) latest_event_sequence: u64,
    pub(crate) latest_snapshot_sequence: u64,
    pub(crate) wrote_snapshot: bool,
}

#[derive(Clone)]
pub(crate) struct DurableSnapshotScheduler {
    owner_id: String,
    durable_state: DurableKernelStateStore,
    sessions: SessionStateStore,
    agents: AgentServiceStore,
    slices: SliceStore,
    metaagent_events: MetaagentEventStore,
    interval_events: u64,
}

impl DurableSnapshotScheduler {
    pub(crate) fn new(
        owner_id: impl Into<String>,
        durable_state: DurableKernelStateStore,
        sessions: SessionStateStore,
        agents: AgentServiceStore,
        slices: SliceStore,
        metaagent_events: MetaagentEventStore,
        interval_events: u64,
    ) -> Self {
        Self {
            owner_id: owner_id.into(),
            durable_state,
            sessions,
            agents,
            slices,
            metaagent_events,
            interval_events,
        }
    }

    pub(crate) fn tick_once(&self) -> Result<DurableSnapshotTickOutcome, DaemonError> {
        let latest_event_sequence = self.durable_state.latest_event_sequence()?;
        let latest_snapshot_sequence = self
            .durable_state
            .latest_snapshot_sequence_for_owner(&self.owner_id)?;
        if latest_event_sequence.saturating_sub(latest_snapshot_sequence) < self.interval_events {
            return Ok(DurableSnapshotTickOutcome {
                latest_event_sequence,
                latest_snapshot_sequence,
                wrote_snapshot: false,
            });
        }

        let payload = DurableKernelSnapshotPayload::capture(
            &self.sessions,
            &self.agents,
            &self.slices,
            &self.metaagent_events,
        );
        self.durable_state.save_entity_checkpoint(
            &self.owner_id,
            latest_event_sequence,
            checkpoint_entities(&payload)?,
        )?;

        Ok(DurableSnapshotTickOutcome {
            latest_event_sequence,
            latest_snapshot_sequence,
            wrote_snapshot: true,
        })
    }

    pub(crate) async fn run(self, poll_interval: Duration) {
        loop {
            sleep(poll_interval).await;
            match self.tick_once() {
                Ok(outcome) if outcome.wrote_snapshot => {
                    let writer = self.durable_state.writer_health_snapshot();
                    crate::logging::debug_with_fields(
                        "durable_state.snapshot",
                        "saved durable state snapshot",
                        serde_json::json!({
                            "sequence": outcome.latest_event_sequence,
                            "previous_snapshot_sequence": outcome.latest_snapshot_sequence,
                            "writer_committed_batches": writer.committed_batches,
                            "writer_committed_records": writer.committed_records,
                            "writer_max_batch_records": writer.max_batch_records,
                        }),
                    );
                }
                Ok(_) => {}
                Err(error) => {
                    crate::logging::warn_with_fields(
                        "durable_state.snapshot",
                        "failed to save durable state snapshot",
                        serde_json::json!({
                            "error": error.to_string(),
                        }),
                    );
                }
            }
        }
    }
}

fn checkpoint_entities(
    payload: &DurableKernelSnapshotPayload,
) -> Result<Vec<DurableCheckpointEntity>, DaemonError> {
    let payload = serde_json::to_value(payload).map_err(|error| DaemonError::LocalTransport {
        operation: "durable_state.encode_checkpoint_payload",
        message: error.to_string(),
    })?;
    let object = payload
        .as_object()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "durable_state.encode_checkpoint_payload",
            message: "durable checkpoint payload must be an object".to_string(),
        })?;
    let mut entities = Vec::new();
    for (kind, values) in object {
        let values = values
            .as_array()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "durable_state.encode_checkpoint_payload",
                message: format!("durable checkpoint group `{kind}` must be an array"),
            })?;
        for (index, value) in values.iter().enumerate() {
            let id = checkpoint_entity_id(kind, value, index);
            entities.push(DurableCheckpointEntity {
                kind: kind.clone(),
                id,
                payload_json: serde_json::to_string(value).map_err(|error| {
                    DaemonError::LocalTransport {
                        operation: "durable_state.encode_checkpoint_entity",
                        message: error.to_string(),
                    }
                })?,
            });
        }
    }
    Ok(entities)
}

fn checkpoint_entity_id(kind: &str, value: &serde_json::Value, index: usize) -> String {
    if kind == "prompt_private_states" {
        if let (Some(session_id), Some(prompt_id)) = (
            value.get("session_id").and_then(serde_json::Value::as_str),
            value.get("prompt_id").and_then(serde_json::Value::as_str),
        ) {
            return format!("{session_id}:{prompt_id}");
        }
    }
    ["id", "event_id", "subscription_id", "slice_id", "backup_id"]
        .into_iter()
        .find_map(|field| value.get(field).and_then(serde_json::Value::as_str))
        .map(str::to_string)
        .unwrap_or_else(|| format!("{index:020}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::DaemonApp;
    use crate::config::DaemonConfig;
    use crate::session::CreateSessionRequest;

    #[test]
    fn prompt_private_checkpoint_ids_are_scoped_by_session() {
        let first = serde_json::json!({
            "session_id": "session-1",
            "prompt_id": "prompt-1"
        });
        let second = serde_json::json!({
            "session_id": "session-2",
            "prompt_id": "prompt-1"
        });

        assert_eq!(
            checkpoint_entity_id("prompt_private_states", &first, 0),
            "session-1:prompt-1"
        );
        assert_eq!(
            checkpoint_entity_id("prompt_private_states", &second, 1),
            "session-2:prompt-1"
        );
    }

    #[test]
    fn snapshot_excludes_ephemeral_runtime_sessions() {
        let mut app =
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should bootstrap");
        let (durable_session, _) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("durable session should be created");
        let ephemeral_session = app
            .session_state_store()
            .create_ephemeral_session(
                CreateSessionRequest::new("worker-runtime", "worker-runtime").with_hidden(true),
            )
            .expect("ephemeral runtime session should be created");

        let snapshot = DurableKernelSnapshotPayload::capture(
            &app.session_state_store(),
            app.agents(),
            &app.slices(),
            &app.metaagent_event_store(),
        );

        assert!(snapshot
            .sessions
            .iter()
            .any(|session| session.id() == durable_session.id()));
        assert!(snapshot
            .sessions
            .iter()
            .all(|session| session.id() != ephemeral_session.id()));
    }

    #[test]
    fn tick_once_skips_until_interval_is_reached() {
        let mut app =
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should bootstrap");
        crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");

        let scheduler = DurableSnapshotScheduler::new(
            app.config().daemon_id.clone(),
            app.durable_state_store(),
            app.session_state_store(),
            app.agents().clone(),
            app.slices(),
            app.metaagent_event_store(),
            10,
        );
        let outcome = scheduler.tick_once().expect("tick should succeed");

        assert!(!outcome.wrote_snapshot);
        assert_eq!(
            app.durable_state_store()
                .latest_snapshot_sequence()
                .expect("snapshot sequence should load"),
            0
        );
    }

    #[test]
    fn tick_once_writes_snapshot_when_interval_is_reached() {
        let mut app =
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should bootstrap");
        let (session, _) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let slice = app
            .slices()
            .create(
                &app.config().daemon_id,
                &app.config().host_machine_id,
                crate::slice::CreateSliceInput {
                    name: "linux-dev".to_string(),
                    backend: crate::slice::SliceBackendKind::LocalDocker,
                    os: "linux".to_string(),
                    display_mode: crate::slice::SliceDisplayMode::Headed,
                    workspace_id: None,
                    worktree_id: None,
                    workspace_mount: Some("/repo".to_string()),
                    worker_kernel_ref: None,
                    display_url: Some("http://127.0.0.1:6080".to_string()),
                    provider_auth: Vec::new(),
                    from_saved_state: None,
                    now_ms: 42,
                },
            )
            .expect("slice should create");

        let scheduler = DurableSnapshotScheduler::new(
            app.config().daemon_id.clone(),
            app.durable_state_store(),
            app.session_state_store(),
            app.agents().clone(),
            app.slices(),
            app.metaagent_event_store(),
            1,
        );
        let outcome = scheduler.tick_once().expect("tick should succeed");
        let snapshot = app
            .durable_state_store()
            .latest_snapshot()
            .expect("snapshot should load")
            .expect("snapshot should exist");

        assert!(outcome.wrote_snapshot);
        assert_eq!(snapshot.sequence, outcome.latest_event_sequence);
        assert_eq!(snapshot.payload["sessions"][0]["id"], session.id());
        assert_eq!(snapshot.payload["slices"][0]["id"], slice.id);
    }

    #[tokio::test]
    async fn tick_once_does_not_need_main_app_lock() {
        let mut app =
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should bootstrap");
        crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let scheduler = DurableSnapshotScheduler::new(
            app.config().daemon_id.clone(),
            app.durable_state_store(),
            app.session_state_store(),
            app.agents().clone(),
            app.slices(),
            app.metaagent_event_store(),
            1,
        );
        let app = std::sync::Arc::new(tokio::sync::Mutex::new(app));
        let _guard = app.lock().await;

        let outcome = tokio::time::timeout(Duration::from_secs(1), async {
            scheduler.tick_once().expect("tick should succeed")
        })
        .await
        .expect("snapshot tick should not wait for the app lock");

        assert!(outcome.wrote_snapshot);
    }
}
