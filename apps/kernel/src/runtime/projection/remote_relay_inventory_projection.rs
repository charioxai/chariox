use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use chariox_relay::protocol::RelayKernelPresence;

use crate::local::{RemoteMachineInventory, RemoteMachineRecord};
use crate::session::unix_epoch_ms;

#[derive(Clone, Default)]
pub(crate) struct RemoteRelayInventoryProjectionStore {
    state: Arc<StdMutex<RemoteRelayInventoryProjectionState>>,
    /// Clients following the machine list, local and over the relay: while any is attached the
    /// connector keeps the list fresh.
    watchers: Arc<std::sync::atomic::AtomicUsize>,
    /// The renewal of the list that requests wait for.
    renewal: super::machine_list_renewal::MachineListRenewal,
}

/// A live pass's place among passes: its number in start order, and the removal generation it
/// read. Only the newest pass to start publishes (`publish_live`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct LivePass {
    number: u64,
    registry_revision: u64,
}

/// Whether a live pass may write what it saw, decided under the projection lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LivePassVerdict {
    /// No newer pass published and no Remove landed: the pass writes.
    Publish,
    /// A pass that started later published first: its list is newer, this one writes nothing.
    Superseded,
    /// A Remove landed since the pass started: it writes nothing and runs again.
    RegistryChanged,
}

/// One client following the machine list; dropping it ends its demand.
pub(crate) struct RemoteInventoryWatch(Arc<std::sync::atomic::AtomicUsize>);

impl Drop for RemoteInventoryWatch {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

#[derive(Debug, Clone, Default)]
struct RemoteRelayInventoryProjectionState {
    remote_machines: Vec<RemoteMachineRecord>,
    offline_machines: Vec<crate::local::OfflineMachineRecord>,
    /// The Cloud directory the last live pass read; kept while the relay is down.
    directory: Option<String>,
    remote_kernels: Vec<RelayKernelPresence>,
    refreshed_at_ms: u64,
    refresh_requested_at_ms: u64,
    /// Kernels that were absent from the previous inventory and are present now, until taken.
    appeared_kernels: std::collections::BTreeSet<String>,
    /// The kernels the owner's Cloud directory still linked on its last two fresh reads, oldest first.
    directory_reads: std::collections::VecDeque<std::collections::BTreeSet<String>>,
    /// The machines those reads linked, with the directory read, same order; None for a read
    /// that could not tell.
    machine_directory_reads:
        std::collections::VecDeque<(String, Option<std::collections::BTreeSet<String>>)>,
    /// Passes started so far, live and disconnected; each is numbered when it starts.
    passes: u64,
    /// The number of the newest pass that published.
    published_pass: u64,
    /// The removal generation: raised by every Remove while the registry is held, so a live pass
    /// that started before a Remove does not publish (`publish_live`).
    registry_revision: u64,
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
            directory: state.directory.clone(),
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

    /// The machines the same read of Cloud directory `directory` linked; see `gone_machines`.
    pub(crate) fn record_directory_machines(
        &self,
        directory: &str,
        linked: Option<std::collections::BTreeSet<String>>,
    ) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state
            .machine_directory_reads
            .push_back((directory.to_string(), linked));
        while state.machine_directory_reads.len() > 2 {
            state.machine_directory_reads.pop_front();
        }
    }

    /// The machines among `machines` that neither of the last two fresh reads of Cloud directory
    /// `directory` linked: removed from the account (Remove unlinks them in Cloud), not merely
    /// offline. The same two-read rule as `gone_kernels`; a read of another directory, or one that
    /// could not tell, decides nothing.
    pub(crate) fn gone_machines<'a>(
        &self,
        directory: &str,
        machines: impl IntoIterator<Item = &'a String>,
    ) -> std::collections::BTreeSet<String> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let reads: Option<Vec<&std::collections::BTreeSet<String>>> = state
            .machine_directory_reads
            .iter()
            .map(|(read, linked)| linked.as_ref().filter(|_| read == directory))
            .collect();
        let Some(reads) = reads.filter(|reads| reads.len() == 2) else {
            return std::collections::BTreeSet::new();
        };
        machines
            .into_iter()
            .filter(|machine| reads.iter().all(|linked| !linked.contains(*machine)))
            .cloned()
            .collect()
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

    /// The renewal of the list that requests wait for (`machine_list_renewal`).
    pub(crate) fn renewal(&self) -> &super::machine_list_renewal::MachineListRenewal {
        &self.renewal
    }

    /// A client starts following the machine list.
    pub(crate) fn watch(&self) -> RemoteInventoryWatch {
        self.watchers
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        RemoteInventoryWatch(Arc::clone(&self.watchers))
    }

    /// Whether a local client follows the machine list now.
    pub(crate) fn watched(&self) -> bool {
        self.watchers.load(std::sync::atomic::Ordering::SeqCst) > 0
    }

    /// A live pass starts: its number orders it against every other pass, and the removal
    /// generation it reads tells it whether a Remove overtook it (`publish_live`).
    pub(crate) fn begin_live_pass(&self) -> LivePass {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.passes += 1;
        LivePass {
            number: state.passes,
            registry_revision: state.registry_revision,
        }
    }

    /// What `publish_live` would decide for `pass` now. A pass asks before it changes the
    /// registry, so a pass that will not publish changes nothing; the caller holds the registry,
    /// and every writer of the verdict's inputs does too, so the verdict holds until it publishes.
    pub(crate) fn live_pass_verdict(&self, pass: LivePass) -> LivePassVerdict {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .live_pass_verdict(pass)
    }

    /// The machines shown Online now: one missing from the next pass has been seen gone only
    /// from that pass on.
    pub(crate) fn online_machine_ids(&self) -> std::collections::BTreeSet<String> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remote_machines
            .iter()
            .filter(|machine| machine.presence == crate::local::RemoteMachinePresence::Online)
            .map(|machine| machine.machine_id.clone())
            .collect()
    }

    /// Publishes a live pass only if no pass that started after it published and no Remove
    /// landed since it started: the check and the write are one step under the lock. The caller
    /// holds the registry.
    pub(crate) fn publish_live(
        &self,
        pass: LivePass,
        inventory: RemoteMachineInventory,
        remote_kernels: Vec<RelayKernelPresence>,
    ) -> LivePassVerdict {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let verdict = state.live_pass_verdict(pass);
        if verdict != LivePassVerdict::Publish {
            return verdict;
        }
        state.published_pass = pass.number;
        state.offline_machines = inventory.offline_machines;
        state.directory = inventory.directory;
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
        LivePassVerdict::Publish
    }

    /// Publishes a live pass as of now, for tests that set the inventory directly.
    #[cfg(test)]
    pub(crate) fn update(
        &self,
        inventory: RemoteMachineInventory,
        remote_kernels: Vec<RelayKernelPresence>,
    ) {
        let pass = self.begin_live_pass();
        assert_eq!(
            self.publish_live(pass, inventory, remote_kernels),
            LivePassVerdict::Publish
        );
    }

    /// Approval changes trust only: a listed machine keeps what the live pass found, its kernels
    /// and their protocols among it; a machine no pass has listed yet joins with its live record.
    /// A live pass needs no compare and set against it: it reads the approval from the registry it
    /// holds when it publishes.
    pub(crate) fn approve_machine(&self, live: RemoteMachineRecord) -> RemoteMachineRecord {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let approved = match state
            .remote_machines
            .iter_mut()
            .find(|existing| existing.machine_id == live.machine_id)
        {
            Some(existing) => {
                existing.trust_status = crate::local::RemoteMachineTrustStatus::Approved;
                existing.pending = false;
                existing.clone()
            }
            None => {
                let machine = filter_remote_machine_product_providers(live);
                state.remote_machines.push(machine.clone());
                machine
            }
        };
        crate::local::provider_requests::sort_remote_machines(&mut state.remote_machines);
        approved
    }

    /// A removed machine leaves every list; the caller holds the registry it was removed from.
    pub(crate) fn remove_machine(&self, machine_id: &str) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.registry_revision += 1;
        state.remove_machine(machine_id);
    }

    /// Holds the projection so a test can stop a writer just before its write.
    #[cfg(test)]
    pub(crate) fn hold(&self) -> HeldRemoteRelayInventoryProjection<'_> {
        HeldRemoteRelayInventoryProjection(
            self.state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    /// The relay is unreachable: no kernel is live and the projection holds only what the kernel
    /// remembers (reconnecting and offline machines). It stays stale, so the next refresh asks the relay.
    /// It is a pass that starts and publishes at once: a live pass that started before it saw the
    /// relay before it went away, and does not publish after it.
    pub(crate) fn retain_remembered(&self, remembered: RemoteMachineInventory) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.passes += 1;
        state.published_pass = state.passes;
        state.remote_machines = remembered.machines;
        state.offline_machines = remembered.offline_machines;
        state.remote_kernels.clear();
        state.refreshed_at_ms = 0;
        state.refresh_requested_at_ms = 0;
        state.directory_reads.clear();
        state.machine_directory_reads.clear();
    }
}

impl RemoteRelayInventoryProjectionState {
    fn live_pass_verdict(&self, pass: LivePass) -> LivePassVerdict {
        if pass.number < self.published_pass {
            LivePassVerdict::Superseded
        } else if pass.registry_revision != self.registry_revision {
            LivePassVerdict::RegistryChanged
        } else {
            LivePassVerdict::Publish
        }
    }

    fn remove_machine(&mut self, machine_id: &str) {
        self.remote_machines
            .retain(|machine| machine.machine_id != machine_id);
        self.offline_machines
            .retain(|machine| machine.machine_id != machine_id);
        self.remote_kernels
            .retain(|kernel| kernel.machine_id != machine_id);
    }
}

#[cfg(test)]
pub(crate) struct HeldRemoteRelayInventoryProjection<'a>(
    std::sync::MutexGuard<'a, RemoteRelayInventoryProjectionState>,
);

#[cfg(test)]
impl HeldRemoteRelayInventoryProjection<'_> {
    pub(crate) fn remove_machine(&mut self, machine_id: &str) {
        self.0.remove_machine(machine_id);
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

    // Remove unlinks a machine in Cloud; every kernel that remembers it forgets it once two
    // consecutive directory reads omit it. One read is not two, and a read that names a linked
    // kernel without its machine decides nothing.
    #[test]
    fn a_machine_neither_of_the_last_two_directory_reads_links_is_gone() {
        let projection = RemoteRelayInventoryProjectionStore::default();
        let remembered = kernels(&["kept", "removed"]);
        let directory = "https://cloud.test/account-1/realm-1";
        projection.record_directory_machines(directory, Some(kernels(&["kept"])));
        assert!(
            projection.gone_machines(directory, &remembered).is_empty(),
            "one read is not two"
        );
        projection.record_directory_machines(directory, Some(kernels(&["kept"])));
        assert_eq!(
            projection.gone_machines(directory, &remembered),
            kernels(&["removed"])
        );
        assert!(
            projection
                .gone_machines("https://cloud.test/account-2/realm-1", &remembered)
                .is_empty(),
            "the reads of one directory say nothing of another's machines"
        );
        projection.record_directory_machines(directory, None);
        assert!(
            projection.gone_machines(directory, &remembered).is_empty(),
            "a read that could not tell decides nothing"
        );
        projection.record_directory_machines(directory, Some(kernels(&["kept", "removed"])));
        projection.record_directory_machines(directory, Some(kernels(&["kept"])));
        assert!(
            projection.gone_machines(directory, &remembered).is_empty(),
            "a machine either of the last two reads links stays"
        );
    }

    // Membership table, page side: a client compares a machine's `directory` with the directory
    // of the kernel that answered. The live pass sets it; a relay outage keeps it.
    #[test]
    fn the_list_names_the_directory_its_live_pass_read_and_keeps_it_while_the_relay_is_down() {
        let projection = RemoteRelayInventoryProjectionStore::default();
        projection.update(
            crate::local::RemoteMachineInventory {
                directory: Some("https://cloud.test/account-1/realm-1".into()),
                ..Default::default()
            },
            Vec::new(),
        );
        assert_eq!(
            projection.inventory().directory.as_deref(),
            Some("https://cloud.test/account-1/realm-1")
        );
        projection.retain_remembered(Default::default());
        assert_eq!(
            projection.inventory().directory.as_deref(),
            Some("https://cloud.test/account-1/realm-1")
        );
    }

    fn machines(ids: &[&str]) -> crate::local::RemoteMachineInventory {
        crate::local::RemoteMachineInventory {
            machines: ids
                .iter()
                .map(|id| crate::local::RemoteMachineRecord {
                    machine_id: id.to_string(),
                    machine_alias: None,
                    display_name: id.to_string(),
                    trust_status: crate::local::RemoteMachineTrustStatus::Approved,
                    presence: crate::local::RemoteMachinePresence::Online,
                    pending: false,
                    kernel_count: 1,
                    available_providers: Vec::new(),
                    provider_accounts: Vec::new(),
                    kernels: Vec::new(),
                    directory: None,
                })
                .collect(),
            ..Default::default()
        }
    }
    fn listed(projection: &RemoteRelayInventoryProjectionStore) -> Vec<String> {
        projection
            .inventory()
            .machines
            .into_iter()
            .map(|machine| machine.machine_id)
            .collect()
    }

    // #1474 review 5969964480: a pass that started before another never publishes after it,
    // whatever order they finish in; a disconnected pass counts as one.
    #[test]
    fn an_older_pass_that_finishes_after_a_newer_one_does_not_publish() {
        use super::LivePassVerdict;
        let projection = RemoteRelayInventoryProjectionStore::default();
        let older = projection.begin_live_pass();
        let newer = projection.begin_live_pass();
        assert_eq!(
            projection.publish_live(newer, machines(&["now"]), Vec::new()),
            LivePassVerdict::Publish
        );
        assert_eq!(
            projection.live_pass_verdict(older),
            LivePassVerdict::Superseded,
            "asked before it touches the registry"
        );
        assert_eq!(
            projection.publish_live(older, machines(&["before"]), Vec::new()),
            LivePassVerdict::Superseded
        );
        assert_eq!(listed(&projection), vec!["now"]);

        let before_the_drop = projection.begin_live_pass();
        projection.retain_remembered(machines(&["remembered"]));
        assert_eq!(
            projection.publish_live(before_the_drop, machines(&["before"]), Vec::new()),
            LivePassVerdict::Superseded
        );
        assert_eq!(listed(&projection), vec!["remembered"]);
    }

    #[test]
    fn of_two_passes_racing_to_publish_the_newer_list_stays() {
        for _ in 0..200 {
            let projection = RemoteRelayInventoryProjectionStore::default();
            let older = projection.begin_live_pass();
            let newer = projection.begin_live_pass();
            let start = std::sync::Arc::new(std::sync::Barrier::new(2));
            let race = |pass: super::LivePass, id: &'static str| {
                let projection = projection.clone();
                let start = std::sync::Arc::clone(&start);
                std::thread::spawn(move || {
                    start.wait();
                    projection.publish_live(pass, machines(&[id]), Vec::new());
                })
            };
            let threads = [race(older, "before"), race(newer, "now")];
            for thread in threads {
                thread.join().expect("publisher thread");
            }
            assert_eq!(listed(&projection), vec!["now"]);
        }
    }

    // A Remove that lands while a pass runs sends it round again; a newer pass does not.
    #[test]
    fn a_pass_a_remove_overtook_runs_again() {
        use super::LivePassVerdict;
        let projection = RemoteRelayInventoryProjectionStore::default();
        let pass = projection.begin_live_pass();
        projection.remove_machine("gone");
        assert_eq!(
            projection.publish_live(pass, machines(&["gone"]), Vec::new()),
            LivePassVerdict::RegistryChanged
        );
        let again = projection.begin_live_pass();
        assert_eq!(
            projection.publish_live(again, machines(&["kept"]), Vec::new()),
            LivePassVerdict::Publish
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
    fn remote_relay_inventory_projection_approves_machine_without_refresh() {
        use crate::local::{RemoteMachineRecord, RemoteMachineTrustStatus};

        let projection = RemoteRelayInventoryProjectionStore::default();
        projection.update(
            crate::local::RemoteMachineInventory {
                machines: vec![RemoteMachineRecord {
                    directory: None,
                    machine_id: "machine-1".to_string(),
                    machine_alias: Some("builder".to_string()),
                    display_name: "builder".to_string(),
                    trust_status: RemoteMachineTrustStatus::Pending,
                    presence: crate::local::RemoteMachinePresence::Online,
                    pending: true,
                    kernel_count: 1,
                    available_providers: vec!["dev-stub".to_string()],
                    provider_accounts: Vec::new(),
                    kernels: vec![crate::local::RemoteMachineKernel {
                        kernel_id: "kernel-1".to_string(),
                        local_protocol: Some(1012),
                    }],
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
                local_protocol: None,
            }],
        );
        let refreshed_at_ms = projection.refreshed_at_ms();

        // The approval's live record names no kernels; the listed machine keeps the ones it has.
        let approved = projection.approve_machine(RemoteMachineRecord {
            directory: None,
            machine_id: "machine-1".to_string(),
            machine_alias: Some("builder".to_string()),
            display_name: "builder".to_string(),
            trust_status: RemoteMachineTrustStatus::Approved,
            presence: crate::local::RemoteMachinePresence::Online,
            pending: false,
            kernel_count: 1,
            available_providers: vec!["dev-stub".to_string()],
            provider_accounts: Vec::new(),
            kernels: Vec::new(),
        });

        assert_eq!(projection.refreshed_at_ms(), refreshed_at_ms);

        let (machines, kernels) = projection.snapshot();
        assert_eq!(machines, vec![approved]);
        assert_eq!(machines[0].trust_status, RemoteMachineTrustStatus::Approved);
        assert!(!machines[0].pending);
        assert_eq!(
            machines[0].kernels,
            vec![crate::local::RemoteMachineKernel {
                kernel_id: "kernel-1".to_string(),
                local_protocol: Some(1012),
            }]
        );
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
                    directory: None,
                    machine_id: "machine-1".to_string(),
                    machine_alias: Some("builder".to_string()),
                    display_name: "builder".to_string(),
                    trust_status: RemoteMachineTrustStatus::Approved,
                    presence: crate::local::RemoteMachinePresence::Online,
                    pending: false,
                    kernel_count: 1,
                    available_providers: Vec::new(),
                    provider_accounts: Vec::new(),
                    kernels: Vec::new(),
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
                local_protocol: None,
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
