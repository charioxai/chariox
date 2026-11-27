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

/// The kernels `machines` name that are not among the `live` ones and are too old for this
/// gateway. The machine list names a refused kernel next to the live ones (remote_inventory.rs).
pub(crate) fn update_needed_in(
    _machines: &[crate::local::RemoteMachineRecord],
    _live: &[RelayKernelPresence],
) -> Vec<UpdateNeededKernel> {
    Vec::new()
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
        update_needed_in(&machines, &live)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::{RemoteMachineKernel, RemoteMachinePresence, RemoteMachineRecord};

    fn machine(
        id: &str,
        alias: Option<&str>,
        kernels: &[(&str, Option<u32>)],
    ) -> RemoteMachineRecord {
        RemoteMachineRecord {
            directory: None,
            machine_id: id.into(),
            machine_alias: alias.map(Into::into),
            display_name: id.into(),
            trust_status: crate::local::RemoteMachineTrustStatus::Approved,
            presence: RemoteMachinePresence::Online,
            pending: false,
            kernel_count: kernels.len(),
            available_providers: Vec::new(),
            provider_accounts: Vec::new(),
            kernels: kernels
                .iter()
                .map(|(kernel_id, protocol)| RemoteMachineKernel {
                    kernel_id: (*kernel_id).into(),
                    local_protocol: *protocol,
                })
                .collect(),
        }
    }

    fn live(id: &str, protocol: u32) -> RelayKernelPresence {
        RelayKernelPresence {
            kernel_id: id.into(),
            machine_id: "m".into(),
            machine_alias: None,
            relay_alias: None,
            kernel_alias: None,
            available_providers: Vec::new(),
            provider_accounts: Vec::new(),
            capabilities: Vec::new(),
            accepting_remote_leases: false,
            leased_agent_count: 0,
            local_session_count: 0,
            public_key: String::new(),
            local_protocol: Some(protocol),
        }
    }

    // Row 490: the machine list names a relay-refused kernel beside the live ones; only what it
    // names that is not live and is too old becomes a listed, update-needed kernel.
    #[test]
    fn a_machine_names_the_kernels_the_relay_refused_beside_the_live_ones() {
        let current = super::super::update_needed::MINIMUM_TARGET_PROTOCOL;
        let machines = [
            machine(
                "m-pc",
                Some("pc"),
                &[("k-live", Some(current)), ("k-1011", Some(1011))],
            ),
            machine(
                "m-nas",
                None,
                &[("k-silent", None), ("k-reconnecting", Some(current))],
            ),
        ];
        let refused = update_needed_in(&machines, &[live("k-live", current)]);
        let ids: Vec<_> = refused
            .iter()
            .map(|kernel| kernel.kernel_id.as_str())
            .collect();
        assert_eq!(
            ids,
            ["k-1011", "k-silent"],
            "a live kernel and a current one that is merely not live are not refused"
        );
        assert_eq!(refused[0].machine_alias.as_deref(), Some("pc"));
        assert_eq!(refused[0].protocol, Some(1011));
        assert_eq!(refused[1].protocol, None);
    }
}
