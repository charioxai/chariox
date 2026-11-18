use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use chariox_relay::protocol::RelayKernelPresence;

use crate::local::{RemoteMachineInventory, RemoteMachineRecord};
use crate::session::unix_epoch_ms;

#[derive(Clone, Default)]
pub(crate) struct RemoteRelayInventoryProjectionStore {
    state: Arc<StdMutex<RemoteRelayInventoryProjectionState>>,
}

#[derive(Debug, Clone, Default)]
struct RemoteRelayInventoryProjectionState {
    remote_machines: Vec<RemoteMachineRecord>,
    offline_machines: Vec<crate::local::OfflineMachineRecord>,
    remote_kernels: Vec<RelayKernelPresence>,
    refreshed_at_ms: u64,
    refresh_requested_at_ms: u64,
    /// Kernels that were absent from the previous inventory and are present now, until taken.
    appeared_kernels: std::collections::BTreeSet<String>,
    /// The kernels the owner's Cloud directory still linked on its last two reads, oldest first.
    directory_reads: std::collections::VecDeque<std::collections::BTreeSet<String>>,
}

impl RemoteRelayInventoryProjectionStore {
    pub(crate) fn snapshot(&self) -> (Vec<RemoteMachineRecord>, Vec<RelayKernelPresence>) {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (state.remote_machines.clone(), state.remote_kernels.clone())
    }

    pub(crate) fn inventory(&self) -> RemoteMachineInventory {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        RemoteMachineInventory {
            machines: state.remote_machines.clone(),
            offline_machines: state.offline_machines.clone(),
        }
    }

    /// One read of the owner's Cloud directory: the kernels it still links, offline ones too.
    pub(crate) fn record_directory_read(&self, linked: std::collections::BTreeSet<String>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.directory_reads.push_back(linked);
        while state.directory_reads.len() > 2 {
            state.directory_reads.pop_front();
        }
    }

    /// Whether two reads of the directory exist, so a kernel can be told gone.
    pub(crate) fn has_directory_history(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .directory_reads
            .len()
            >= 2
    }

    /// The kernels among `kernels` that neither of the last two reads of the directory linked:
    /// removed from the account, not merely offline.
    pub(crate) fn gone_kernels(
        &self,
        kernels: &std::collections::BTreeSet<String>,
    ) -> std::collections::BTreeSet<String> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.directory_reads.len() < 2 {
            return std::collections::BTreeSet::new();
        }
        kernels
            .iter()
            .filter(|kernel| {
                state
                    .directory_reads
                    .iter()
                    .all(|linked| !linked.contains(*kernel))
            })
            .cloned()
            .collect()
    }

    /// The kernels that (re)appeared in the inventory since the last call.
    pub(crate) fn take_appeared_kernels(&self) -> Vec<String> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::mem::take(&mut state.appeared_kernels)
            .into_iter()
            .collect()
    }

    pub(crate) fn is_fresh(&self, now_ms: u64, stale_after_ms: u64) -> bool {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.refreshed_at_ms != 0 && now_ms.saturating_sub(state.refreshed_at_ms) < stale_after_ms
    }

    #[cfg(test)]
    fn refreshed_at_ms(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .refreshed_at_ms
    }

    pub(crate) fn should_request_refresh(
        &self,
        now_ms: u64,
        stale_after_ms: u64,
        cooldown_ms: u64,
    ) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let empty = state.remote_machines.is_empty() && state.remote_kernels.is_empty();
        let stale = state.refreshed_at_ms == 0
            || now_ms.saturating_sub(state.refreshed_at_ms) >= stale_after_ms;
        let cooled_down = state.refresh_requested_at_ms == 0
            || now_ms.saturating_sub(state.refresh_requested_at_ms) >= cooldown_ms;
        if (empty || stale) && cooled_down {
            state.refresh_requested_at_ms = now_ms;
            return true;
        }
        false
    }

    pub(crate) fn update(
        &self,
        inventory: RemoteMachineInventory,
        remote_kernels: Vec<RelayKernelPresence>,
    ) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.offline_machines = inventory.offline_machines;
        state.remote_machines = inventory
            .machines
            .into_iter()
            .map(filter_remote_machine_product_providers)
            .collect();
        let previous: std::collections::BTreeSet<String> = state
            .remote_kernels
            .iter()
            .map(|kernel| kernel.kernel_id.clone())
            .collect();
        state.remote_kernels = remote_kernels
            .into_iter()
            .map(filter_remote_kernel_product_providers)
            .collect();
        let appeared: Vec<String> = state
            .remote_kernels
            .iter()
            .filter(|kernel| !previous.contains(&kernel.kernel_id))
            .map(|kernel| kernel.kernel_id.clone())
            .collect();
        state.appeared_kernels.extend(appeared);
        state.refreshed_at_ms = unix_epoch_ms();
        state.refresh_requested_at_ms = state.refreshed_at_ms;
    }

    pub(crate) fn update_machine(&self, machine: RemoteMachineRecord) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(existing) = state
            .remote_machines
            .iter_mut()
            .find(|existing| existing.machine_id == machine.machine_id)
        {
            *existing = filter_remote_machine_product_providers(machine);
        } else {
            state
                .remote_machines
                .push(filter_remote_machine_product_providers(machine));
        }
        crate::local::provider_requests::sort_remote_machines(&mut state.remote_machines);
    }

    pub(crate) fn remove_machine(&self, machine_id: &str) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state
            .remote_machines
            .retain(|machine| machine.machine_id != machine_id);
        state
            .offline_machines
            .retain(|machine| machine.machine_id != machine_id);
        state
            .remote_kernels
            .retain(|kernel| kernel.machine_id != machine_id);
    }

    /// The relay is unreachable: no kernel is live and the projection holds only what the kernel
    /// remembers (reconnecting and offline machines). It stays stale, so the next refresh asks the relay.
    pub(crate) fn retain_remembered(&self, remembered: RemoteMachineInventory) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.remote_machines = remembered.machines;
        state.offline_machines = remembered.offline_machines;
        state.remote_kernels.clear();
        state.refreshed_at_ms = 0;
        state.refresh_requested_at_ms = 0;
        state.directory_reads.clear();
    }
}

fn filter_remote_machine_product_providers(
    mut machine: RemoteMachineRecord,
) -> RemoteMachineRecord {
    crate::provider::retain_public_inventory_providers(&mut machine.available_providers);
    machine
}

fn filter_remote_kernel_product_providers(mut kernel: RelayKernelPresence) -> RelayKernelPresence {
    crate::provider::retain_public_inventory_providers(&mut kernel.available_providers);
    kernel
}

#[cfg(test)]
mod tests {
    use super::RemoteRelayInventoryProjectionStore;

    fn kernels(ids: &[&str]) -> std::collections::BTreeSet<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    // A kernel is gone for good only once two consecutive reads of the owner's directory
    // omit it; one miss, or an offline kernel the directory still links, is not.
    #[test]
    fn a_kernel_is_gone_after_two_directory_reads_without_it() {
        let projection = RemoteRelayInventoryProjectionStore::default();
        let watched = kernels(&["asleep", "removed"]);
        assert!(projection.gone_kernels(&watched).is_empty(), "no read yet");
        projection.record_directory_read(kernels(&["asleep"]));
        assert!(
            projection.gone_kernels(&watched).is_empty(),
            "one read is not two"
        );
        projection.record_directory_read(kernels(&["asleep"]));
        assert_eq!(projection.gone_kernels(&watched), kernels(&["removed"]));
        projection.record_directory_read(kernels(&["asleep", "removed"]));
        assert!(
            projection.gone_kernels(&watched).is_empty(),
            "a kernel listed again in either of the last two reads stays"
        );
    }

    #[test]
    fn remote_relay_inventory_projection_requests_refresh_when_empty_or_stale() {
        let projection = RemoteRelayInventoryProjectionStore::default();
        assert!(!projection.is_fresh(10_000, 5_000));
        assert!(projection.should_request_refresh(10_000, 5_000, 1_000));
        assert!(
            !projection.should_request_refresh(10_500, 5_000, 1_000),
            "refresh should respect the cooldown while the projection remains empty"
        );
        assert!(
            projection.should_request_refresh(16_000, 5_000, 1_000),
            "stale empty projection should request another refresh after the cooldown"
        );
    }

    #[test]
    fn remote_relay_inventory_projection_tracks_full_refresh_freshness() {
        let projection = RemoteRelayInventoryProjectionStore::default();
        projection.update(Default::default(), Vec::new());
        let now_ms = crate::session::unix_epoch_ms();

        assert!(projection.is_fresh(now_ms, 5_000));
        assert!(!projection.is_fresh(now_ms.saturating_add(5_000), 5_000));
    }

    #[test]
    fn remote_relay_inventory_projection_updates_machine_without_refresh() {
        use crate::local::{RemoteMachineRecord, RemoteMachineTrustStatus};

        let projection = RemoteRelayInventoryProjectionStore::default();
        projection.update(
            crate::local::RemoteMachineInventory {
                machines: vec![RemoteMachineRecord {
                    machine_id: "machine-1".to_string(),
                    machine_alias: Some("builder".to_string()),
                    display_name: "builder".to_string(),
                    trust_status: RemoteMachineTrustStatus::Pending,
                    presence: crate::local::RemoteMachinePresence::Online,
                    pending: true,
                    kernel_count: 1,
                    available_providers: vec!["dev-stub".to_string()],
                    provider_accounts: Vec::new(),
                }],
                ..Default::default()
            },
            vec![chariox_relay::protocol::RelayKernelPresence {
                kernel_id: "kernel-1".to_string(),
                machine_id: "machine-1".to_string(),
                machine_alias: Some("builder".to_string()),
                relay_alias: None,
                kernel_alias: Some("default".to_string()),
                available_providers: vec!["dev-stub".to_string()],
                provider_accounts: Vec::new(),
                capabilities: vec!["kernel_ws".to_string()],
                accepting_remote_leases: true,
                leased_agent_count: 0,
                local_session_count: 0,
                public_key: "public-key".to_string(),
            }],
        );
        let refreshed_at_ms = projection.refreshed_at_ms();

        projection.update_machine(RemoteMachineRecord {
            machine_id: "machine-1".to_string(),
            machine_alias: Some("builder".to_string()),
            display_name: "build box".to_string(),
            trust_status: RemoteMachineTrustStatus::Approved,
            presence: crate::local::RemoteMachinePresence::Online,
            pending: false,
            kernel_count: 1,
            available_providers: vec!["dev-stub".to_string()],
            provider_accounts: Vec::new(),
        });

        assert_eq!(projection.refreshed_at_ms(), refreshed_at_ms);

        let (machines, kernels) = projection.snapshot();
        assert_eq!(machines.len(), 1);
        assert_eq!(machines[0].trust_status, RemoteMachineTrustStatus::Approved);
        assert!(!machines[0].pending);
        assert!(machines[0].available_providers.is_empty());
        assert_eq!(kernels.len(), 1);
        assert!(kernels[0].available_providers.is_empty());
    }

    #[test]
    fn remote_relay_inventory_projection_removes_machine_and_kernels() {
        use crate::local::{RemoteMachineRecord, RemoteMachineTrustStatus};

        let projection = RemoteRelayInventoryProjectionStore::default();
        projection.update(
            crate::local::RemoteMachineInventory {
                machines: vec![RemoteMachineRecord {
                    machine_id: "machine-1".to_string(),
                    machine_alias: Some("builder".to_string()),
                    display_name: "builder".to_string(),
                    trust_status: RemoteMachineTrustStatus::Approved,
                    presence: crate::local::RemoteMachinePresence::Online,
                    pending: false,
                    kernel_count: 1,
                    available_providers: Vec::new(),
                    provider_accounts: Vec::new(),
                }],
                ..Default::default()
            },
            vec![chariox_relay::protocol::RelayKernelPresence {
                kernel_id: "kernel-1".to_string(),
                machine_id: "machine-1".to_string(),
                machine_alias: Some("builder".to_string()),
                relay_alias: None,
                kernel_alias: Some("default".to_string()),
                available_providers: Vec::new(),
                provider_accounts: Vec::new(),
                capabilities: Vec::new(),
                accepting_remote_leases: true,
                leased_agent_count: 0,
                local_session_count: 0,
                public_key: "public-key".to_string(),
            }],
        );
        let refreshed_at_ms = projection.refreshed_at_ms();

        projection.remove_machine("machine-1");

        assert_eq!(projection.refreshed_at_ms(), refreshed_at_ms);
        let (machines, kernels) = projection.snapshot();
        assert!(machines.is_empty());
        assert!(kernels.is_empty());
    }
}
