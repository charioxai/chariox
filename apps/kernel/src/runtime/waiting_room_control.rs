use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::local::{LocalDaemonResponse, WaitingRoomPublicSnapshot};
use crate::runtime::projection::DaemonConfigProjectionStore;
use crate::runtime::relay_config_control::projected_relay_status_view;
use crate::runtime::session_read_control::execute_list_sessions_request;
use crate::runtime::terminal_pairings::paired_terminal_records;
use crate::runtime::waiting_room_public_projection::build_waiting_room_public_snapshot;
use crate::session::unix_epoch_ms;
use crate::transport::relay_client::RelayClientState;

pub(crate) async fn execute_waiting_room_inventory_request(
    app: &Arc<Mutex<DaemonApp>>,
    relay_state: Arc<RwLock<RelayClientState>>,
    config_projection: DaemonConfigProjectionStore,
) -> Result<LocalDaemonResponse, DaemonError> {
    Ok(LocalDaemonResponse::WaitingRoomInventory {
        snapshot: projected_waiting_room_public_snapshot(app, relay_state, config_projection)
            .await?
            .into(),
    })
}

pub(crate) async fn execute_waiting_room_public_snapshot_request(
    app: &Arc<Mutex<DaemonApp>>,
    relay_state: Arc<RwLock<RelayClientState>>,
    config_projection: DaemonConfigProjectionStore,
) -> Result<LocalDaemonResponse, DaemonError> {
    Ok(LocalDaemonResponse::WaitingRoomPublicSnapshot {
        snapshot: projected_waiting_room_public_snapshot(app, relay_state, config_projection)
            .await?,
    })
}

pub(crate) async fn waiting_room_inventory_version(
    app: &Arc<Mutex<DaemonApp>>,
    relay_state: Arc<RwLock<RelayClientState>>,
    config_projection: DaemonConfigProjectionStore,
) -> Result<String, DaemonError> {
    match execute_waiting_room_inventory_request(app, relay_state, config_projection).await? {
        LocalDaemonResponse::WaitingRoomInventory { snapshot } => Ok(snapshot.inventory_version),
        _response => Err(DaemonError::LocalTransport {
            operation: "build waiting room inventory version",
            message: "waiting room inventory request produced unexpected response".to_string(),
        }),
    }
}

async fn projected_waiting_room_public_snapshot(
    app: &Arc<Mutex<DaemonApp>>,
    relay_state: Arc<RwLock<RelayClientState>>,
    config_projection: DaemonConfigProjectionStore,
) -> Result<WaitingRoomPublicSnapshot, DaemonError> {
    let runtime_sessions =
        match execute_list_sessions_request(app, crate::local::ListSessionsRequest).await? {
            LocalDaemonResponse::SessionsListed { sessions } => sessions,
            _response => {
                return Err(DaemonError::LocalTransport {
                    operation: "build waiting room inventory",
                    message: format!("list sessions produced unexpected response `{}`", "unknown"),
                });
            }
        };
    let relay_status = projected_relay_status_view(relay_state, config_projection).await;
    let terminals = paired_terminal_records();
    build_waiting_room_public_snapshot(runtime_sessions, relay_status, terminals, unix_epoch_ms())
}
