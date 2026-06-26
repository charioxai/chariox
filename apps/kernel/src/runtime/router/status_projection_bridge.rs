use std::sync::Arc;

use super::CommandRouter;
use crate::error::DaemonError;
use crate::runtime::daemon_health_projection::{
    build_daemon_health_projection, DaemonHealthProjectionInput,
};
use crate::runtime::projection::DaemonHealthProjection;
use crate::runtime::waiting_room_control::projected_waiting_room_public_snapshot;
use crate::session::DEFAULT_LOCAL_USER_ID;

impl CommandRouter {
    pub(crate) async fn waiting_room_public_snapshot(
        &self,
    ) -> Result<crate::local::WaitingRoomPublicSnapshot, DaemonError> {
        projected_waiting_room_public_snapshot(
            Arc::clone(&self.app),
            &self.runtime_state,
            Arc::clone(&self.relay_state),
            self.config_projection.clone(),
            self.remote_relay_inventory_projection.clone(),
            DEFAULT_LOCAL_USER_ID,
        )
        .await
    }

    #[allow(dead_code)]
    pub(crate) async fn daemon_health_projection(
        &self,
        last_event_id: u64,
    ) -> DaemonHealthProjection {
        build_daemon_health_projection(self.daemon_health_projection_input(last_event_id)).await
    }

    pub(super) fn daemon_health_projection_input(
        &self,
        last_event_id: u64,
    ) -> DaemonHealthProjectionInput<'_> {
        DaemonHealthProjectionInput {
            last_event_id,
            session_runtime: &self.session_runtime,
            agent_runtime: &self.agent_runtime,
            workflow_runtime: &self.workflow_runtime,
            provider_runtime_lanes: &self.provider_runtime_lanes,
            capability_health: &self.capability_health,
            session_projection: &self.session_projection,
            agent_runtime_projection: &self.agent_runtime_projection,
            provider_catalog_projection: &self.provider_catalog_projection,
            provider_run_projection: &self.provider_run_projection,
            transport_health: &self.transport_health,
            terminal_health: &self.terminal_health,
            workspace_coordinator: &self.workspace_coordinator,
            runtime_state: &self.runtime_state,
        }
    }
}
