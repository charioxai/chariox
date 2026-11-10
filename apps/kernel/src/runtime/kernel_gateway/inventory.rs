//! The one interface the gateway lists kernels through. Sources today: the
//! relay inventory (hosted: filtered by the account's fresh Cloud targets) and
//! the kernels this kernel paired with. A registry or gossip mesh can implement
//! it later without touching the gateway or the CLI.
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use chariox_relay::protocol::RelayKernelPresence;

use crate::error::DaemonError;
use crate::local::GatewayKernelVia;
use crate::runtime::projection::{
    DaemonConfigProjectionStore, RemoteRelayInventoryProjectionStore,
};
use crate::transport::relay_client::RelayClientState;

pub(crate) trait InventorySource: Send + Sync {
    fn via(&self) -> GatewayKernelVia;
    /// The kernels the latest refresh listed as live; never this kernel itself.
    fn kernels(&self) -> Vec<RelayKernelPresence>;
    fn is_fresh(&self) -> bool;
    /// Listed within the last `max_age_ms`. A question asked by a person or an agent
    /// (`chariox kernels`) wants a list this young, not one that is merely good enough to route by.
    fn refreshed_within(&self, max_age_ms: u64) -> bool;
    fn refresh(&self) -> Pin<Box<dyn Future<Output = Result<(), DaemonError>> + Send + '_>>;
}

pub(crate) struct RelayInventory {
    pub(crate) config: DaemonConfigProjectionStore,
    pub(crate) projection: RemoteRelayInventoryProjectionStore,
    pub(crate) relay_state: Arc<tokio::sync::RwLock<RelayClientState>>,
}

impl InventorySource for RelayInventory {
    fn via(&self) -> GatewayKernelVia {
        if self.config.snapshot().cloud_relay.is_some() {
            GatewayKernelVia::Hosted
        } else {
            GatewayKernelVia::SelfHosted
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

    fn refreshed_within(&self, max_age_ms: u64) -> bool {
        self.projection
            .is_fresh(crate::session::unix_epoch_ms(), max_age_ms)
    }

    fn refresh(&self) -> Pin<Box<dyn Future<Output = Result<(), DaemonError>> + Send + '_>> {
        Box::pin(
            crate::transport::relay_client::refresh_remote_inventory_projection(
                self.config.clone(),
                self.projection.clone(),
                Some(Arc::clone(&self.relay_state)),
            ),
        )
    }
}
