//! The one interface the gateway lists kernels through. Sources today: the
//! relay inventory (hosted: filtered by the account's fresh Cloud targets) and
//! the kernels this kernel paired with. A registry or gossip mesh can implement
//! it later without touching the gateway or the CLI.
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use chariox_relay::protocol::RelayKernelPresence;

use crate::error::DaemonError;
use crate::local::{GatewayKernelUpdate, GatewayKernelVia};
use crate::runtime::projection::{
    DaemonConfigProjectionStore, RemoteRelayInventoryProjectionStore,
};
use crate::transport::relay_client::RelayClientState;

/// A kernel the machine list names that the relay carries no traffic for because it is too old
/// (it never appears among the live kernels), with the protocol it reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UpdateNeededKernel {
    pub(crate) kernel_id: String,
    pub(crate) machine_id: String,
    pub(crate) machine_alias: Option<String>,
    pub(crate) protocol: Option<u32>,
}

pub(crate) trait InventorySource: Send + Sync {
    fn via(&self) -> GatewayKernelVia;
    /// The kernels the latest refresh listed as live; never this kernel itself.
    fn kernels(&self) -> Vec<RelayKernelPresence>;
    /// The kernels the latest refresh named that are too old to be live; none by default.
    fn update_needed_kernels(&self) -> Vec<UpdateNeededKernel> {
        Vec::new()
    }
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

    fn update_needed_kernels(&self) -> Vec<UpdateNeededKernel> {
        let (machines, live) = self.projection.snapshot();
        let live: std::collections::BTreeSet<&str> = live
            .iter()
            .map(|kernel| kernel.kernel_id.as_str())
            .collect();
        // The machine list names a refused kernel next to the live ones (remote_inventory.rs);
        // what it names that is not live and is too old is exactly that.
        machines
            .iter()
            .flat_map(|machine| {
                machine
                    .kernels
                    .iter()
                    .map(move |kernel| UpdateNeededKernel {
                        kernel_id: kernel.kernel_id.clone(),
                        machine_id: machine.machine_id.clone(),
                        machine_alias: machine.machine_alias.clone(),
                        protocol: kernel.local_protocol,
                    })
            })
            .filter(|kernel| !live.contains(kernel.kernel_id.as_str()))
            .filter(|kernel| {
                super::update_needed::update_state(kernel.protocol)
                    == GatewayKernelUpdate::UpdateNeeded
            })
            .collect()
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
