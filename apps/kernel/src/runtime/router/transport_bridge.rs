use std::path::PathBuf;

use super::CommandRouter;
use crate::error::DaemonError;
use crate::local::{RelayStatus, RemoteMachineRecord};
use crate::provider::OpenCodeProviderCatalog;
use crate::runtime::projection::{SessionSnapshotProjection, TransportHealthStore};
use crate::slice::SliceRecord;

impl CommandRouter {
    pub(crate) fn runtime_state(&self) -> crate::runtime::state::KernelRuntimeState {
        self.runtime_state.clone()
    }

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

    pub(crate) fn terminal_stream_change_sequence(&self) -> u64 {
        self.runtime_state.terminal_stream_change_sequence()
    }

    pub(crate) fn terminal_attachment_change_sequence(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> u64 {
        self.runtime_state
            .terminal_attachment_change_sequence(session_id, attachment_id)
    }

    pub(crate) async fn wait_for_terminal_stream_change_after(&self, sequence: u64) {
        self.runtime_state
            .wait_for_terminal_stream_change_after(sequence)
            .await;
    }

    pub(crate) async fn wait_for_terminal_attachment_change_after(
        &self,
        session_id: &str,
        attachment_id: &str,
        sequence: u64,
    ) {
        self.runtime_state
            .wait_for_terminal_attachment_change_after(session_id, attachment_id, sequence)
            .await;
    }

    pub(crate) fn waiting_room_change_sequence(&self) -> u64 {
        self.runtime_state.waiting_room_change_sequence()
    }

    pub(crate) async fn wait_for_waiting_room_change_after(&self, sequence: u64) {
        self.runtime_state
            .wait_for_waiting_room_change_after(sequence)
            .await;
    }

    pub(crate) fn session_projection_change_sequence(&self) -> u64 {
        self.runtime_state.session_projection_change_sequence()
    }

    pub(crate) fn session_projection_session_change_sequence(&self, session_id: &str) -> u64 {
        self.runtime_state
            .session_projection_session_change_sequence(session_id)
    }

    pub(crate) async fn wait_for_session_projection_change_after(&self, sequence: u64) {
        self.runtime_state
            .wait_for_session_projection_change_after(sequence)
            .await;
    }

    pub(crate) async fn wait_for_session_projection_session_change_after(
        &self,
        session_id: &str,
        sequence: u64,
    ) {
        self.runtime_state
            .wait_for_session_projection_session_change_after(session_id, sequence)
            .await;
    }

    pub(crate) fn transport_runtime_pump_change_sequence(&self) -> u64 {
        self.runtime_state.transport_runtime_pump_change_sequence()
    }

    pub(crate) async fn wait_for_transport_runtime_pump_change_after(&self, sequence: u64) {
        self.runtime_state
            .wait_for_transport_runtime_pump_change_after(sequence)
            .await;
    }

    pub(crate) fn transport_runtime_pump_interval_ms(
        &self,
        active_interval_ms: u64,
        idle_interval_ms: u64,
        now_ms: u64,
    ) -> u64 {
        self.runtime_state.transport_runtime_pump_interval_ms(
            active_interval_ms,
            idle_interval_ms,
            now_ms,
        )
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

    pub(crate) fn transport_provider_catalog_snapshot(&self) -> Option<OpenCodeProviderCatalog> {
        self.provider_catalog_projection
            .get(crate::local::provider_requests::PROVIDER_CATALOG_CACHE_TTL)
    }

    pub(crate) fn transport_slices_snapshot(&self) -> Vec<SliceRecord> {
        self.runtime_state.list_slices()
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

    pub(crate) fn session_snapshot_projection_for_attachment(
        &self,
        session_id: &str,
        attachment_id: &str,
        last_event_id: u64,
    ) -> Result<SessionSnapshotProjection, DaemonError> {
        self.runtime_state
            .session_snapshot_projection_for_attachment(session_id, attachment_id, last_event_id)
    }
}
