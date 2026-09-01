use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::time::sleep;

use crate::agent::{AgentInstance, AgentServiceStore};
use crate::durable_state::{
    DurableCheckpointEntity, DurableCheckpointMarker, DurableEventTailStatistics,
    DurableKernelStateStore,
};
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
    pub(crate) previous_checkpoint_sequence: u64,
    pub(crate) tail_event_count: u64,
    pub(crate) tail_bytes: u64,
    pub(crate) wrote_snapshot: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DurableCheckpointPolicy {
    pub(crate) changed_entity_limit: u64,
    pub(crate) tail_byte_limit: u64,
    pub(crate) elapsed_time_limit: Duration,
    pub(crate) hard_tail_byte_limit: u64,
}

impl DurableCheckpointPolicy {
    pub(crate) fn new(
        changed_entity_limit: u64,
        tail_byte_limit: u64,
        elapsed_time_limit: Duration,
        hard_tail_byte_limit: u64,
    ) -> Self {
        Self {
            changed_entity_limit: changed_entity_limit.max(1),
            tail_byte_limit: tail_byte_limit.max(1),
            elapsed_time_limit: elapsed_time_limit.max(Duration::from_secs(1)),
            hard_tail_byte_limit: hard_tail_byte_limit.max(tail_byte_limit.max(1)),
        }
    }

    #[cfg(test)]
    fn event_count_only(changed_entity_limit: u64) -> Self {
        Self::new(
            changed_entity_limit,
            u64::MAX,
            Duration::from_secs(u32::MAX as u64),
            u64::MAX,
        )
    }

    pub(crate) fn from_user_state_config(state: &crate::config::UserStateConfig) -> Option<Self> {
        if state.snapshot_interval_events.is_none()
            && state.snapshot_interval_bytes.is_none()
            && state.snapshot_interval_seconds.is_none()
            && state.snapshot_max_tail_bytes.is_none()
        {
            return None;
        }
        let tail_byte_limit = state
            .snapshot_interval_bytes
            .map(u64::from)
            .unwrap_or(u64::MAX);
        Some(Self::new(
            state
                .snapshot_interval_events
                .map(u64::from)
                .unwrap_or(u64::MAX),
            tail_byte_limit,
            Duration::from_secs(
                state
                    .snapshot_interval_seconds
                    .map(u64::from)
                    .unwrap_or(u32::MAX as u64),
            ),
            state
                .snapshot_max_tail_bytes
                .map(u64::from)
                .unwrap_or(u64::MAX),
        ))
    }
}

#[derive(Clone)]
pub(crate) struct DurableSnapshotScheduler {
    owner_id: String,
    durable_state: DurableKernelStateStore,
    sessions: SessionStateStore,
    agents: AgentServiceStore,
    slices: SliceStore,
    metaagent_events: MetaagentEventStore,
    policy: DurableCheckpointPolicy,
    checkpoint_marker: Arc<Mutex<Option<DurableCheckpointMarker>>>,
}

impl DurableSnapshotScheduler {
    #[cfg(test)]
    pub(crate) fn new(
        owner_id: impl Into<String>,
        durable_state: DurableKernelStateStore,
        sessions: SessionStateStore,
        agents: AgentServiceStore,
        slices: SliceStore,
        metaagent_events: MetaagentEventStore,
        interval_events: u64,
    ) -> Self {
        Self::new_with_policy(
            owner_id,
            durable_state,
            sessions,
            agents,
            slices,
            metaagent_events,
            DurableCheckpointPolicy::event_count_only(interval_events),
        )
    }

    pub(crate) fn new_with_policy(
        owner_id: impl Into<String>,
        durable_state: DurableKernelStateStore,
        sessions: SessionStateStore,
        agents: AgentServiceStore,
        slices: SliceStore,
        metaagent_events: MetaagentEventStore,
        policy: DurableCheckpointPolicy,
    ) -> Self {
        Self {
            owner_id: owner_id.into(),
            durable_state,
            sessions,
            agents,
            slices,
            metaagent_events,
            policy,
            checkpoint_marker: Arc::new(Mutex::new(None)),
        }
    }

    pub(crate) fn tick_once(&self) -> Result<DurableSnapshotTickOutcome, DaemonError> {
        let checkpoint_marker = {
            let cached = *self
                .checkpoint_marker
                .lock()
                .expect("durable checkpoint marker lock poisoned");
            match cached {
                Some(marker) => marker,
                None => self
                    .durable_state
                    .latest_checkpoint_marker_for_owner(&self.owner_id)?,
            }
        };
        {
            let mut cached = self
                .checkpoint_marker
                .lock()
                .expect("durable checkpoint marker lock poisoned");
            if cached.is_none() {
                *cached = Some(checkpoint_marker);
            }
        }
        let tail = self
            .durable_state
            .event_tail_statistics(checkpoint_marker.sequence)?;
        let now_ms = crate::session::unix_epoch_ms();
        let should_checkpoint = checkpoint_due(self.policy, checkpoint_marker, tail, now_ms);
        if !should_checkpoint {
            return Ok(DurableSnapshotTickOutcome {
                latest_event_sequence: tail.latest_sequence,
                previous_checkpoint_sequence: checkpoint_marker.sequence,
                tail_event_count: tail.event_count,
                tail_bytes: tail.encoded_bytes,
                wrote_snapshot: false,
            });
        }

        let mut checkpoint_sequence = tail.latest_sequence;
        self.write_checkpoint(checkpoint_sequence)?;
        let post_checkpoint_tail = self
            .durable_state
            .event_tail_statistics(checkpoint_sequence)?;
        if post_checkpoint_tail.encoded_bytes > self.policy.hard_tail_byte_limit {
            checkpoint_sequence = post_checkpoint_tail.latest_sequence;
            self.write_checkpoint(checkpoint_sequence)?;
            let remaining = self
                .durable_state
                .event_tail_statistics(checkpoint_sequence)?;
            if remaining.encoded_bytes > self.policy.hard_tail_byte_limit {
                return Err(DaemonError::LocalTransport {
                    operation: "durable_state.enforce_checkpoint_tail_budget",
                    message: format!(
                        "post-checkpoint event tail is {} bytes, above hard limit {}",
                        remaining.encoded_bytes, self.policy.hard_tail_byte_limit
                    ),
                });
            }
        }
        *self
            .checkpoint_marker
            .lock()
            .expect("durable checkpoint marker lock poisoned") = Some(DurableCheckpointMarker {
            sequence: checkpoint_sequence,
            timestamp_ms: now_ms,
        });

        Ok(DurableSnapshotTickOutcome {
            latest_event_sequence: checkpoint_sequence,
            previous_checkpoint_sequence: checkpoint_marker.sequence,
            tail_event_count: tail.event_count,
            tail_bytes: tail.encoded_bytes,
            wrote_snapshot: true,
        })
    }

    fn write_checkpoint(&self, sequence: u64) -> Result<(), DaemonError> {
        let payload = DurableKernelSnapshotPayload::capture(
            &self.sessions,
            &self.agents,
            &self.slices,
            &self.metaagent_events,
        );
        self.durable_state.save_entity_checkpoint(
            &self.owner_id,
            sequence,
            checkpoint_entities(&payload)?,
        )?;
        Ok(())
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
                            "previous_checkpoint_sequence": outcome.previous_checkpoint_sequence,
                            "tail_event_count": outcome.tail_event_count,
                            "tail_bytes": outcome.tail_bytes,
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

fn checkpoint_due(
    policy: DurableCheckpointPolicy,
    marker: DurableCheckpointMarker,
    tail: DurableEventTailStatistics,
    now_ms: u64,
) -> bool {
    let elapsed_baseline_ms = if marker.timestamp_ms > 0 {
        marker.timestamp_ms
    } else {
        tail.oldest_timestamp_ms.unwrap_or(now_ms)
    };
    let elapsed_ms = now_ms.saturating_sub(elapsed_baseline_ms);
    tail.event_count > 0
        && (tail.event_count >= policy.changed_entity_limit
            || tail.encoded_bytes >= policy.tail_byte_limit
            || elapsed_ms >= policy.elapsed_time_limit.as_millis() as u64)
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
    fn checkpoint_policy_triggers_on_count_bytes_or_elapsed_time() {
        let policy = DurableCheckpointPolicy::new(10, 1_000, Duration::from_secs(30), 2_000);
        let marker = DurableCheckpointMarker {
            sequence: 7,
            timestamp_ms: 10_000,
        };
        let tail = |event_count, encoded_bytes| DurableEventTailStatistics {
            event_count,
            encoded_bytes,
            oldest_timestamp_ms: Some(10_000),
            latest_sequence: 7 + event_count,
        };

        assert!(checkpoint_due(policy, marker, tail(10, 100), 10_001));
        assert!(checkpoint_due(policy, marker, tail(1, 1_000), 10_001));
        assert!(checkpoint_due(policy, marker, tail(1, 100), 40_000));
        assert!(!checkpoint_due(policy, marker, tail(1, 100), 39_999));
        assert!(!checkpoint_due(policy, marker, tail(0, 2_000), 50_000));
    }

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
                    display_backend: Default::default(),
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
