use std::path::PathBuf;

use super::CommandRouter;
use crate::error::DaemonError;
use crate::local::{RelayStatus, RemoteMachineRecord};
use crate::runtime::projection::{SessionSnapshotProjection, TransportHealthStore};

impl CommandRouter {
    pub(crate) fn kernel_websocket_bind_address(&self) -> (String, u16) {
        let config = self.config_projection.snapshot();
        (config.kernel_websocket_host, config.kernel_websocket_port)
    }

    pub(crate) fn kernel_websocket_connection_config(&self) -> (usize, u64) {
        let config = self.config_projection.snapshot();
        (
            config.kernel_websocket_queue_capacity,
            config.kernel_websocket_write_delay_ms,
        )
    }

    pub(crate) fn kernel_event_counter_path(&self) -> PathBuf {
        self.config_projection
            .snapshot()
            .kernel_event_counter_path()
    }

    pub(crate) fn relay_event_counter_path(&self) -> PathBuf {
        self.config_projection
            .snapshot()
            .kernel_relay_event_counter_path()
    }

    pub(crate) fn transport_health_store(&self) -> TransportHealthStore {
        self.transport_health.clone()
    }

    pub(crate) fn durable_snapshot_scheduler(
        &self,
    ) -> Option<crate::durable_snapshot::DurableSnapshotScheduler> {
        self.runtime_state.durable_snapshot_scheduler()
    }

    pub(crate) async fn pump_transport_runtime(&self) {
        self.runtime_state.pump_transport_runtime().await;
    }

    pub(crate) async fn shutdown_cleanup(&self) -> Result<(), DaemonError> {
        self.runtime_state.shutdown_cleanup().await
    }

    pub(crate) async fn transport_relay_status_snapshot(&self) -> RelayStatus {
        crate::runtime::remote_relay_inventory::projected_relay_status(
            self.relay_state.clone(),
            self.config_projection.clone(),
        )
        .await
    }

    pub(crate) fn transport_remote_machines_snapshot(&self) -> Vec<RemoteMachineRecord> {
        let (machines, _) = self.remote_relay_inventory_projection.snapshot();
        machines
    }

    pub(crate) fn clear_remote_relay_inventory_projection(&self) {
        self.remote_relay_inventory_projection.clear();
    }

    pub(crate) async fn refresh_remote_relay_inventory_projection(
        &self,
    ) -> Result<(), DaemonError> {
        crate::transport::relay_client::refresh_remote_inventory_projection(
            self.config_projection.clone(),
            self.remote_relay_inventory_projection.clone(),
        )
        .await
    }

    pub(crate) fn session_snapshot_projection(
        &self,
        session_id: &str,
        last_event_id: u64,
    ) -> Result<SessionSnapshotProjection, DaemonError> {
        self.runtime_state
            .session_snapshot_projection(session_id, last_event_id)
    }
}
