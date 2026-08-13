use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use chariox_relay::protocol::RelayKernelPresence;

use crate::local::RemoteMachineRecord;
use crate::session::unix_epoch_ms;

#[derive(Clone, Default)]
pub(crate) struct RemoteRelayInventoryProjectionStore {
    state: Arc<StdMutex<RemoteRelayInventoryProjectionState>>,
}

#[derive(Debug, Clone, Default)]
struct RemoteRelayInventoryProjectionState {
    remote_machines: Vec<RemoteMachineRecord>,
    remote_kernels: Vec<RelayKernelPresence>,
    refreshed_at_ms: u64,
    refresh_requested_at_ms: u64,
}

impl RemoteRelayInventoryProjectionStore {
    pub(crate) fn snapshot(&self) -> (Vec<RemoteMachineRecord>, Vec<RelayKernelPresence>) {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (state.remote_machines.clone(), state.remote_kernels.clone())
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
        remote_machines: Vec<RemoteMachineRecord>,
        remote_kernels: Vec<RelayKernelPresence>,
    ) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.remote_machines = remote_machines
            .into_iter()
            .map(filter_remote_machine_product_providers)
            .collect();
        state.remote_kernels = remote_kernels
            .into_iter()
            .map(filter_remote_kernel_product_providers)
            .collect();
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
        sort_remote_machines(&mut state.remote_machines);
        state.refreshed_at_ms = unix_epoch_ms();
        state.refresh_requested_at_ms = state.refreshed_at_ms;
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
            .remote_kernels
            .retain(|kernel| kernel.machine_id != machine_id);
        state.refreshed_at_ms = unix_epoch_ms();
        state.refresh_requested_at_ms = state.refreshed_at_ms;
    }

    pub(crate) fn clear(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.remote_machines.clear();
        state.remote_kernels.clear();
        state.refreshed_at_ms = 0;
        state.refresh_requested_at_ms = 0;
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

fn sort_remote_machines(remote_machines: &mut [RemoteMachineRecord]) {
    remote_machines.sort_by_key(|record| {
        (
            !record.online,
            record.pending,
            record.display_name.to_ascii_lowercase(),
            record.machine_id.clone(),
        )
    });
}

#[cfg(test)]
mod tests {
    use super::RemoteRelayInventoryProjectionStore;

    #[test]
    fn remote_relay_inventory_projection_requests_refresh_when_empty_or_stale() {
        let projection = RemoteRelayInventoryProjectionStore::default();
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
    fn remote_relay_inventory_projection_updates_machine_without_refresh() {
        use crate::local::{RemoteMachineRecord, RemoteMachineTrustStatus};

        let projection = RemoteRelayInventoryProjectionStore::default();
        projection.update(
            vec![RemoteMachineRecord {
                machine_id: "machine-1".to_string(),
                machine_alias: Some("builder".to_string()),
                registry_alias: None,
                display_name: "builder".to_string(),
                trust_status: RemoteMachineTrustStatus::Pending,
                online: true,
                pending: true,
                kernel_count: 1,
                available_providers: vec!["dev-stub".to_string()],
                provider_accounts: Vec::new(),
            }],
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

        projection.update_machine(RemoteMachineRecord {
            machine_id: "machine-1".to_string(),
            machine_alias: Some("builder".to_string()),
            registry_alias: Some("build box".to_string()),
            display_name: "build box".to_string(),
            trust_status: RemoteMachineTrustStatus::Approved,
            online: true,
            pending: false,
            kernel_count: 1,
            available_providers: vec!["dev-stub".to_string()],
            provider_accounts: Vec::new(),
        });

        let (machines, kernels) = projection.snapshot();
        assert_eq!(machines.len(), 1);
        assert_eq!(machines[0].trust_status, RemoteMachineTrustStatus::Approved);
        assert!(!machines[0].pending);
        assert_eq!(machines[0].registry_alias.as_deref(), Some("build box"));
        assert!(machines[0].available_providers.is_empty());
        assert_eq!(kernels.len(), 1);
        assert!(kernels[0].available_providers.is_empty());
    }

    #[test]
    fn remote_relay_inventory_projection_removes_machine_and_kernels() {
        use crate::local::{RemoteMachineRecord, RemoteMachineTrustStatus};

        let projection = RemoteRelayInventoryProjectionStore::default();
        projection.update(
            vec![RemoteMachineRecord {
                machine_id: "machine-1".to_string(),
                machine_alias: Some("builder".to_string()),
                registry_alias: None,
                display_name: "builder".to_string(),
                trust_status: RemoteMachineTrustStatus::Approved,
                online: true,
                pending: false,
                kernel_count: 1,
                available_providers: Vec::new(),
                provider_accounts: Vec::new(),
            }],
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

        projection.remove_machine("machine-1");

        let (machines, kernels) = projection.snapshot();
        assert!(machines.is_empty());
        assert!(kernels.is_empty());
    }
}
