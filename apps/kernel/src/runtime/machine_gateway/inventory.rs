//! The one interface the gateway lists machines through. Sources today: the
//! relay inventory (hosted: filtered by the account's fresh Cloud targets) and
//! the kernels this kernel paired with. A registry or gossip mesh can implement
//! it later without touching the gateway or the CLI.
use std::future::Future;
use std::pin::Pin;

use chariox_relay::protocol::RelayKernelPresence;

use crate::error::DaemonError;
use crate::local::GatewayMachineVia;
use crate::runtime::projection::{
    DaemonConfigProjectionStore, RemoteRelayInventoryProjectionStore,
};

pub(crate) trait InventorySource: Send + Sync {
    fn via(&self) -> GatewayMachineVia;
    /// The kernels the latest refresh listed as live; never this kernel itself.
    fn kernels(&self) -> Vec<RelayKernelPresence>;
    fn is_fresh(&self) -> bool;
    fn refresh(&self) -> Pin<Box<dyn Future<Output = Result<(), DaemonError>> + Send + '_>>;
}

pub(crate) struct RelayInventory {
    pub(crate) config: DaemonConfigProjectionStore,
    pub(crate) projection: RemoteRelayInventoryProjectionStore,
}

impl InventorySource for RelayInventory {
    fn via(&self) -> GatewayMachineVia {
        if self.config.snapshot().cloud_relay.is_some() {
            GatewayMachineVia::Hosted
        } else {
            GatewayMachineVia::SelfHosted
        }
    }

    fn kernels(&self) -> Vec<RelayKernelPresence> {
        self.projection.snapshot().1
    }

    fn is_fresh(&self) -> bool {
        let stale_after_ms = self
            .config
            .snapshot()
            .relay_heartbeat_ms
            .saturating_mul(2)
            .max(1_000);
        self.projection
            .is_fresh(crate::session::unix_epoch_ms(), stale_after_ms)
    }

    fn refresh(&self) -> Pin<Box<dyn Future<Output = Result<(), DaemonError>> + Send + '_>> {
        Box::pin(
            crate::transport::relay_client::refresh_remote_inventory_projection(
                self.config.clone(),
                self.projection.clone(),
            ),
        )
    }
}
