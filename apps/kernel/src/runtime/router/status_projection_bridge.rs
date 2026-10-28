use std::sync::Arc;

use super::CommandRouter;
use crate::error::DaemonError;
use crate::runtime::daemon_health_projection::DaemonHealthProjectionInput;
use crate::runtime::projection::DaemonHealthProjection;
use crate::runtime::waiting_room_control::projected_waiting_room_public_snapshot;

impl CommandRouter {
    pub(crate) async fn waiting_room_public_snapshot(
        &self,
        caller_user_id: &str,
        relay_caller: Option<&crate::runtime::command::KernelCaller>,
    ) -> Result<crate::local::WaitingRoomPublicSnapshot, DaemonError> {
        projected_waiting_room_public_snapshot(
            &self.runtime_state,
            &self.session_projection,
            &self.waiting_room_session_summaries,
            Arc::clone(&self.relay_state),
            self.config_projection.clone(),
            if relay_caller.is_none_or(|caller| self.relay_owner_directory_allowed(caller)) {
                self.remote_relay_inventory_projection.clone()
            } else {
                Default::default()
            },
            caller_user_id,
        )
        .await
    }

    #[allow(dead_code)]
    pub(crate) async fn daemon_health_projection(
        &self,
        last_event_id: u64,
    ) -> DaemonHealthProjection {
        self.daemon_health_cache
            .snapshot(self.daemon_health_projection_input(last_event_id))
            .await
    }

    pub(super) fn daemon_health_projection_input(
        &self,
        last_event_id: u64,
    ) -> DaemonHealthProjectionInput {
        DaemonHealthProjectionInput {
            last_event_id,
            request_metrics: self.request_metrics.clone(),
            session_runtime: self.session_runtime.clone(),
            agent_runtime: self.agent_runtime.clone(),
            workflow_runtime: self.workflow_runtime.clone(),
            provider_runtime_lanes: self.provider_runtime_lanes.clone(),
            capability_health: self.capability_health.clone(),
            session_projection: self.session_projection.clone(),
            agent_runtime_projection: self.agent_runtime_projection.clone(),
            provider_catalog_projection: self.provider_catalog_projection.clone(),
            provider_run_projection: self.provider_run_projection.clone(),
            transport_health: self.transport_health.clone(),
            terminal_health: self.terminal_health.clone(),
            workspace_coordinator: self.workspace_coordinator.clone(),
            runtime_state: self.runtime_state.clone(),
        }
    }
}
