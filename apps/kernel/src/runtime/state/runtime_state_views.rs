use std::collections::{BTreeMap, BTreeSet};

use crate::app::ActiveTurnState;

use super::*;

impl KernelRuntimeState {
    pub(crate) fn start_active_turn_with_trace_id(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        provider_run_id: &str,
        trace_id: &str,
    ) {
        self.owned.active_turns.start(
            ActiveTurnState::new(
                session_id.to_string(),
                agent_id.to_string(),
                prompt_id.to_string(),
                provider_run_id.to_string(),
            )
            .with_trace_id(trace_id),
        );
    }

    pub(crate) fn agent_activity_for_session(
        &self,
        session: &crate::session::RuntimeSession,
    ) -> BTreeMap<String, crate::runtime::projection::AgentRuntimeActivity> {
        let prompt_activity = self.owned.prompt_activity.read();
        let active_turns = self.owned.active_turns.snapshot();
        crate::runtime::projection::agent_activity_for_session_projection(
            session,
            |agent_id| {
                self.owned
                    .provider_run_projection
                    .get_for_agent(session.id(), agent_id)
                    .or_else(|| {
                        self.owned
                            .provider_store
                            .get_run_for_agent(session.id(), agent_id)
                    })
            },
            &prompt_activity,
            &active_turns,
            None,
        )
    }

    pub(crate) async fn config_snapshot(&self) -> crate::config::DaemonConfig {
        self.owned.config_projection.snapshot()
    }

    pub(crate) async fn workspace_live_sync_health_snapshot(
        &self,
    ) -> crate::runtime::projection::WorkspaceLiveSyncHealthSnapshot {
        let reservations = self
            .owned
            .workspace_live_sync_coordinator
            .lock()
            .await
            .active_reservation_snapshots();
        let active_reservation_artifacts = reservations
            .iter()
            .map(|reservation| reservation.artifact_id.clone())
            .collect::<BTreeSet<_>>()
            .len();
        crate::runtime::projection::WorkspaceLiveSyncHealthSnapshot {
            active_reservations: reservations.len(),
            active_reservation_artifacts,
            workspace_identity: self.owned.workspace_identity_monitor.health_snapshot(),
            external_changes: self
                .owned
                .workspace_live_sync_external_changes
                .health_snapshot(),
        }
    }
}
