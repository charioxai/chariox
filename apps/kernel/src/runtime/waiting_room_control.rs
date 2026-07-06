use std::sync::Arc;

use tokio::sync::Mutex;
use tokio::sync::RwLock;

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::local::{
    ListExternalProviderSessionsRequest, LocalDaemonRequest, LocalDaemonResponse,
    WaitingRoomPublicSnapshot,
};
use crate::runtime::projection::{
    DaemonConfigProjectionStore, RemoteRelayInventoryProjectionStore,
};
use crate::runtime::relay_config_control::projected_relay_status_view;
use crate::runtime::session_read_control::execute_list_sessions_request;
use crate::runtime::state::KernelRuntimeState;
use crate::runtime::terminal_pairings::paired_terminal_records;
use crate::runtime::waiting_room_public_projection::build_waiting_room_public_snapshot;
use crate::session::unix_epoch_ms;
use crate::transport::relay_client::RelayClientState;

pub(crate) async fn execute_waiting_room_inventory_request(
    app: Arc<Mutex<DaemonApp>>,
    runtime_state: &KernelRuntimeState,
    relay_state: Arc<RwLock<RelayClientState>>,
    config_projection: DaemonConfigProjectionStore,
    remote_relay_inventory_projection: RemoteRelayInventoryProjectionStore,
    caller_user_id: &str,
) -> Result<LocalDaemonResponse, DaemonError> {
    Ok(LocalDaemonResponse::WaitingRoomInventory {
        snapshot: projected_waiting_room_public_snapshot(
            app,
            runtime_state,
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
    app: Arc<Mutex<DaemonApp>>,
    runtime_state: &KernelRuntimeState,
    relay_state: Arc<RwLock<RelayClientState>>,
    config_projection: DaemonConfigProjectionStore,
    remote_relay_inventory_projection: RemoteRelayInventoryProjectionStore,
    request: LocalDaemonRequest,
    caller_user_id: &str,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::GetWaitingRoomInventory(_) => {
            execute_waiting_room_inventory_request(
                Arc::clone(&app),
                runtime_state,
                relay_state,
                config_projection,
                remote_relay_inventory_projection,
                caller_user_id,
            )
            .await
        }
        LocalDaemonRequest::GetWaitingRoomPublicSnapshot(_) => {
            execute_waiting_room_public_snapshot_request(
                Arc::clone(&app),
                runtime_state,
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
    app: Arc<Mutex<DaemonApp>>,
    runtime_state: &KernelRuntimeState,
    relay_state: Arc<RwLock<RelayClientState>>,
    config_projection: DaemonConfigProjectionStore,
    remote_relay_inventory_projection: RemoteRelayInventoryProjectionStore,
    caller_user_id: &str,
) -> Result<LocalDaemonResponse, DaemonError> {
    Ok(LocalDaemonResponse::WaitingRoomPublicSnapshot {
        snapshot: projected_waiting_room_public_snapshot(
            app,
            runtime_state,
            relay_state,
            config_projection,
            remote_relay_inventory_projection,
            caller_user_id,
        )
        .await?,
    })
}

pub(crate) async fn projected_waiting_room_public_snapshot(
    app: Arc<Mutex<DaemonApp>>,
    runtime_state: &KernelRuntimeState,
    relay_state: Arc<RwLock<RelayClientState>>,
    config_projection: DaemonConfigProjectionStore,
    remote_relay_inventory_projection: RemoteRelayInventoryProjectionStore,
    caller_user_id: &str,
) -> Result<WaitingRoomPublicSnapshot, DaemonError> {
    let runtime_sessions =
        match execute_list_sessions_request(runtime_state, crate::local::ListSessionsRequest)
            .await?
        {
            LocalDaemonResponse::SessionsListed { sessions } => sessions,
            _response => {
                return Err(DaemonError::LocalTransport {
                    operation: "build waiting room inventory",
                    message: format!("list sessions produced unexpected response `{}`", "unknown"),
                });
            }
        };
    let relay_status = projected_relay_status_view(relay_state, config_projection).await;
    let (remote_machines, remote_kernels) = remote_relay_inventory_projection.snapshot();
    let terminals = paired_terminal_records();
    let (external_provider_session_page, metaagent_events) = {
        let app =
            crate::runtime::app_lock::lock_app_instrumented(&app, "waiting_room_control").await;
        (
            app.external_provider_session_index_store().list(
                &ListExternalProviderSessionsRequest {
                    provider: None,
                    cursor: None,
                    limit: Some(25),
                },
            ),
            app.metaagent_event_store(),
        )
    };
    build_waiting_room_public_snapshot(
        runtime_sessions,
        &metaagent_events,
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
