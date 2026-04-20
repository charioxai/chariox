use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::time::sleep;

use crate::agent::{AgentInstance, AgentServiceStore};
use crate::durable_state::DurableKernelStateStore;
use crate::error::DaemonError;
use crate::session::{RuntimeSession, SessionStateStore};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DurableKernelSnapshotPayload {
    pub(crate) sessions: Vec<RuntimeSession>,
    pub(crate) agents: Vec<AgentInstance>,
}

impl DurableKernelSnapshotPayload {
    pub(crate) fn capture(sessions: &SessionStateStore, agents: &AgentServiceStore) -> Self {
        let sessions = sessions.read().store().list();
        let agents = agents.list_agents();
        Self { sessions, agents }
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
    durable_state: DurableKernelStateStore,
    sessions: SessionStateStore,
    agents: AgentServiceStore,
    interval_events: u64,
}

impl DurableSnapshotScheduler {
    pub(crate) fn new(
        durable_state: DurableKernelStateStore,
        sessions: SessionStateStore,
        agents: AgentServiceStore,
        interval_events: u64,
    ) -> Self {
        Self {
            durable_state,
            sessions,
            agents,
            interval_events,
        }
    }

    pub(crate) fn tick_once(&self) -> Result<DurableSnapshotTickOutcome, DaemonError> {
        let latest_event_sequence = self.durable_state.latest_event_sequence()?;
        let latest_snapshot_sequence = self.durable_state.latest_snapshot_sequence()?;
        if latest_event_sequence.saturating_sub(latest_snapshot_sequence) < self.interval_events {
            return Ok(DurableSnapshotTickOutcome {
                latest_event_sequence,
                latest_snapshot_sequence,
                wrote_snapshot: false,
            });
        }

        let payload = DurableKernelSnapshotPayload::capture(&self.sessions, &self.agents);
        let payload =
            serde_json::to_value(payload).map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.encode_snapshot_payload",
                message: error.to_string(),
            })?;
        self.durable_state
            .save_snapshot(latest_event_sequence, payload)?;

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
                    crate::logging::debug_with_fields(
                        "durable_state.snapshot",
                        "saved durable state snapshot",
                        serde_json::json!({
                            "sequence": outcome.latest_event_sequence,
                            "previous_snapshot_sequence": outcome.latest_snapshot_sequence,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::DaemonApp;
    use crate::config::DaemonConfig;
    use crate::session::CreateSessionRequest;

    #[test]
    fn tick_once_skips_until_interval_is_reached() {
        let mut app =
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should bootstrap");
        crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");

        let scheduler = DurableSnapshotScheduler::new(
            app.durable_state_store(),
            app.session_state_store(),
            app.agents().clone(),
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

        let scheduler = DurableSnapshotScheduler::new(
            app.durable_state_store(),
            app.session_state_store(),
            app.agents().clone(),
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
    }

    #[tokio::test]
    async fn tick_once_does_not_need_main_app_lock() {
        let mut app =
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should bootstrap");
        crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let scheduler = DurableSnapshotScheduler::new(
            app.durable_state_store(),
            app.session_state_store(),
            app.agents().clone(),
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
