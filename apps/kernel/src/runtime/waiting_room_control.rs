use std::sync::Arc;

use tokio::sync::RwLock;

use crate::error::DaemonError;
use crate::local::{
    ListExternalProviderSessionsRequest, LocalDaemonRequest, LocalDaemonResponse,
    WaitingRoomPublicSnapshot,
};
use crate::runtime::projection::{
    DaemonConfigProjectionStore, RemoteRelayInventoryProjectionStore, SessionStateProjectionStore,
};
use crate::runtime::relay_config_control::projected_relay_status_view;
use crate::runtime::state::KernelRuntimeState;
use crate::runtime::terminal_pairings::paired_terminal_records;
use crate::runtime::waiting_room_public_projection::{
    build_waiting_room_public_snapshot_from_cached_shared, WaitingRoomSessionSummaryProjectionStore,
};
use crate::session::unix_epoch_ms;
use crate::transport::relay_client::RelayClientState;

pub(crate) async fn execute_waiting_room_inventory_request(
    runtime_state: &KernelRuntimeState,
    session_projection: &SessionStateProjectionStore,
    waiting_room_session_summaries: &WaitingRoomSessionSummaryProjectionStore,
    relay_state: Arc<RwLock<RelayClientState>>,
    config_projection: DaemonConfigProjectionStore,
    remote_relay_inventory_projection: RemoteRelayInventoryProjectionStore,
    caller_user_id: &str,
) -> Result<LocalDaemonResponse, DaemonError> {
    Ok(LocalDaemonResponse::WaitingRoomInventory {
        snapshot: projected_waiting_room_public_snapshot(
            runtime_state,
            session_projection,
            waiting_room_session_summaries,
            relay_state,
            config_projection,
            remote_relay_inventory_projection,
            caller_user_id,
        )
        .await?
        .into(),
    })
}

pub(crate) async fn execute_waiting_room_request(
    runtime_state: &KernelRuntimeState,
    session_projection: &SessionStateProjectionStore,
    waiting_room_session_summaries: &WaitingRoomSessionSummaryProjectionStore,
    relay_state: Arc<RwLock<RelayClientState>>,
    config_projection: DaemonConfigProjectionStore,
    remote_relay_inventory_projection: RemoteRelayInventoryProjectionStore,
    request: LocalDaemonRequest,
    caller_user_id: &str,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::GetWaitingRoomInventory(_) => {
            execute_waiting_room_inventory_request(
                runtime_state,
                session_projection,
                waiting_room_session_summaries,
                relay_state,
                config_projection,
                remote_relay_inventory_projection,
                caller_user_id,
            )
            .await
        }
        LocalDaemonRequest::GetWaitingRoomPublicSnapshot(_) => {
            execute_waiting_room_public_snapshot_request(
                runtime_state,
                session_projection,
                waiting_room_session_summaries,
                relay_state,
                config_projection,
                remote_relay_inventory_projection,
                caller_user_id,
            )
            .await
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "waiting room request",
            message: "unsupported waiting room request".to_string(),
        }),
    }
}

pub(crate) async fn execute_waiting_room_public_snapshot_request(
    runtime_state: &KernelRuntimeState,
    session_projection: &SessionStateProjectionStore,
    waiting_room_session_summaries: &WaitingRoomSessionSummaryProjectionStore,
    relay_state: Arc<RwLock<RelayClientState>>,
    config_projection: DaemonConfigProjectionStore,
    remote_relay_inventory_projection: RemoteRelayInventoryProjectionStore,
    caller_user_id: &str,
) -> Result<LocalDaemonResponse, DaemonError> {
    Ok(LocalDaemonResponse::WaitingRoomPublicSnapshot {
        snapshot: projected_waiting_room_public_snapshot(
            runtime_state,
            session_projection,
            waiting_room_session_summaries,
            relay_state,
            config_projection,
            remote_relay_inventory_projection,
            caller_user_id,
        )
        .await?,
    })
}

pub(crate) async fn projected_waiting_room_public_snapshot(
    runtime_state: &KernelRuntimeState,
    session_projection: &SessionStateProjectionStore,
    waiting_room_session_summaries: &WaitingRoomSessionSummaryProjectionStore,
    relay_state: Arc<RwLock<RelayClientState>>,
    config_projection: DaemonConfigProjectionStore,
    remote_relay_inventory_projection: RemoteRelayInventoryProjectionStore,
    caller_user_id: &str,
) -> Result<WaitingRoomPublicSnapshot, DaemonError> {
    let runtime_sessions = session_projection
        .list_shared()
        .unwrap_or_else(|| Arc::from([]));
    let relay_status = projected_relay_status_view(relay_state, config_projection).await;
    let (remote_machines, remote_kernels) = remote_relay_inventory_projection.snapshot();
    let terminals = paired_terminal_records();
    let (external_provider_session_page, metaagent_events) = runtime_state
        .waiting_room_auxiliary_projection(&ListExternalProviderSessionsRequest {
            provider: None,
            cursor: None,
            limit: Some(25),
        });
    let external_working_agents = session_projection.external_observed_working_agents();
    build_waiting_room_public_snapshot_from_cached_shared(
        runtime_sessions.as_ref(),
        session_projection.change_sequence(),
        waiting_room_session_summaries,
        &metaagent_events,
        &external_working_agents,
        external_provider_session_page.sessions,
        external_provider_session_page.has_more,
        external_provider_session_page.next_cursor,
        relay_status,
        remote_machines,
        remote_kernels,
        terminals,
        unix_epoch_ms(),
        caller_user_id,
    )
}

#[cfg(test)]
mod architecture_tests {
    #[test]
    fn waiting_room_hot_path_uses_owned_projections_only() {
        let source = include_str!("waiting_room_control.rs");
        for forbidden in [
            ["lock", "_app"].concat(),
            ["list_session", "_snapshots"].concat(),
            ["execute_list", "_sessions_request"].concat(),
        ] {
            assert!(
                !source.contains(&forbidden),
                "waiting-room hot path must not contain {forbidden}"
            );
        }
    }
}
