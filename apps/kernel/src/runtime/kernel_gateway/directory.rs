//! Kernel names to live kernels, kept current from the inventory source.
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chariox_relay::protocol::RelayKernelPresence;

use super::error::GatewayError;
use super::inventory::{InventorySource, UpdateNeededKernel};
use super::names;
use crate::local::{GatewayKernel, GatewayKernelStatus, GatewayKernelUpdate};

/// The capability a kernel advertises when it enforces `on_behalf_of`.
const GATEWAY_CAPABILITY: &str = "kernel_gateway_v1";
const REFRESH_TIMEOUT: Duration = Duration::from_secs(10);
/// How old the list may be when `chariox kernels` shows it.
const LISTING_MAX_AGE_MS: u64 = 1_000;

struct Seen {
    kernel: RelayKernelPresence,
    last_seen_ms: u64,
    online: bool,
}

pub(super) struct Directory {
    source: Arc<dyn InventorySource>,
    seen: Mutex<BTreeMap<String, Seen>>,
    /// Kernels the relay refused for their age: listed, never routed to. Only their machine, id
    /// and protocol are known.
    refused: Mutex<BTreeMap<String, Seen>>,
    /// A refresh behind a request is running; one at a time.
    refreshing: Arc<AtomicBool>,
}

impl Directory {
    pub(super) fn new(source: Arc<dyn InventorySource>) -> Self {
        Self {
            source,
            seen: Mutex::new(BTreeMap::new()),
            refused: Mutex::new(BTreeMap::new()),
            refreshing: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(super) async fn kernels(&self) -> Result<Vec<GatewayKernel>, GatewayError> {
        // The list is shown as the truth, so it is renewed unless it was taken a moment ago.
        if !self.source.refreshed_within(LISTING_MAX_AGE_MS) {
            self.refresh_now().await?;
        }
        self.merge();
        let via = self.source.via();
        let seen = self.seen.lock().expect("directory poisoned");
        let refused = self.refused.lock().expect("directory poisoned");
        let now_ms = crate::session::unix_epoch_ms();
        Ok(names::grouped(
            seen.values()
                .chain(refused.values())
                .filter(|seen| {
                    seen.online
                        || crate::runtime::machine_presence::still_reconnecting(
                            seen.last_seen_ms,
                            now_ms,
                        )
                })
                .map(|seen| GatewayKernel {
                    name: names::display_name(&seen.kernel),
                    label: names::label(&seen.kernel),
                    kernel_id: seen.kernel.kernel_id.clone(),
                    machine_id: seen.kernel.machine_id.clone(),
                    machine_alias: seen.kernel.machine_alias.clone(),
                    status: if seen.online {
                        GatewayKernelStatus::Online
                    } else {
                        GatewayKernelStatus::Offline
                    },
                    last_seen_ms: seen.last_seen_ms,
                    gateway_ready: is_gateway_ready(&seen.kernel),
                    via,
                    protocol: seen.kernel.local_protocol,
                    update: super::update_needed::update_state(seen.kernel.local_protocol),
                })
                .collect(),
        ))
    }

    /// The one live kernel `name` means; unknown, ambiguous and offline are errors.
    pub(super) async fn resolve(&self, name: &str) -> Result<RelayKernelPresence, GatewayError> {
        // A request is never held for the kernel list: a kernel any earlier list showed live
        // is used at once and the list is renewed behind the request, however old that list
        // is. The relay knows whether the kernel is connected now and answers `not_connected`
        // at once when it is not. Only an unknown or offline name, or no list at all, asks
        // for a fresh list before answering.
        self.merge();
        if let Ok(kernel) = self.lookup(name) {
            self.refresh_behind();
            return Ok(kernel);
        }
        self.refresh().await?;
        self.lookup(name).map_err(|error| match error {
            GatewayError::Unknown { .. } => self.refused_refusal(name).unwrap_or(error),
            other => other,
        })
    }

    /// A name that only a kernel the relay refused for its age carries: say why it cannot be used,
    /// as the refusal of any too-old target does, instead of "unknown" for a kernel the list shows.
    fn refused_refusal(&self, name: &str) -> Option<GatewayError> {
        let refused = self.refused.lock().expect("directory poisoned");
        let known: Vec<&RelayKernelPresence> = refused.values().map(|seen| &seen.kernel).collect();
        let kernel = names::find(&known, name).ok()?;
        Some(match kernel.local_protocol {
            Some(kernel_protocol)
                if super::update_needed::update_state(Some(kernel_protocol))
                    == GatewayKernelUpdate::UpdateNeeded =>
            {
                GatewayError::UpdateNeeded {
                    name: name.into(),
                    kernel_protocol,
                    needed_protocol: super::update_needed::MINIMUM_TARGET_PROTOCOL,
                }
            }
            _ => GatewayError::Offline { name: name.into() },
        })
    }

    /// The one live kernel `name` means; unknown, ambiguous and offline are errors.
    fn lookup(&self, name: &str) -> Result<RelayKernelPresence, GatewayError> {
        let seen = self.seen.lock().expect("directory poisoned");
        let known: Vec<&RelayKernelPresence> = seen.values().map(|seen| &seen.kernel).collect();
        let kernel = names::find(&known, name)?;
        if seen[&kernel.kernel_id].online {
            Ok(kernel.clone())
        } else {
            Err(GatewayError::Offline { name: name.into() })
        }
    }

    pub(super) fn known_kernel_id(&self, name: &str) -> Option<String> {
        let seen = self.seen.lock().expect("directory poisoned");
        let known: Vec<&RelayKernelPresence> = seen.values().map(|seen| &seen.kernel).collect();
        names::find(&known, name)
            .ok()
            .map(|kernel| kernel.kernel_id.clone())
    }

    fn refresh_behind(&self) {
        if self.source.is_fresh() || self.refreshing.swap(true, Ordering::AcqRel) {
            return;
        }
        let (source, refreshing) = (Arc::clone(&self.source), Arc::clone(&self.refreshing));
        tokio::spawn(async move {
            // A failed refresh leaves the last list in use; the next request tries again.
            let _ = tokio::time::timeout(REFRESH_TIMEOUT, source.refresh()).await;
            refreshing.store(false, Ordering::Release);
        });
    }

    async fn refresh(&self) -> Result<(), GatewayError> {
        if !self.source.is_fresh() {
            self.refresh_now().await?;
        }
        self.merge();
        Ok(())
    }

    async fn refresh_now(&self) -> Result<(), GatewayError> {
        tokio::time::timeout(REFRESH_TIMEOUT, self.source.refresh())
            .await
            .map_err(|_| unreachable_inventory("the kernel list did not refresh in time".into()))?
            .map_err(|error| {
                unreachable_inventory(super::error::plain_cause(
                    "the relay did not answer the request for the kernel list",
                    &error,
                ))
            })
    }

    /// Takes the source's latest list as what is live now.
    fn merge(&self) {
        let now = crate::session::unix_epoch_ms();
        let live = self.source.kernels();
        let mut seen = self.seen.lock().expect("directory poisoned");
        for seen in seen.values_mut() {
            seen.online = false;
        }
        for kernel in live {
            seen.insert(
                kernel.kernel_id.clone(),
                Seen {
                    kernel,
                    last_seen_ms: now,
                    online: true,
                },
            );
        }
        let mut refused = self.refused.lock().expect("directory poisoned");
        // A kernel that updated and registered is live, not refused.
        refused.retain(|id, _| !seen.get(id).is_some_and(|seen| seen.online));
        for kernel in self.source.update_needed_kernels() {
            if seen.get(&kernel.kernel_id).is_some_and(|seen| seen.online) {
                continue;
            }
            // One entry per kernel: the refusal is the newer word than an earlier live sighting.
            seen.remove(&kernel.kernel_id);
            refused.insert(
                kernel.kernel_id.clone(),
                Seen {
                    kernel: stub(&kernel),
                    last_seen_ms: now,
                    online: false,
                },
            );
        }
    }
}

/// What the list knows of a kernel the relay refused: where it is and the protocol it reported.
fn stub(kernel: &UpdateNeededKernel) -> RelayKernelPresence {
    RelayKernelPresence {
        kernel_id: kernel.kernel_id.clone(),
        machine_id: kernel.machine_id.clone(),
        machine_alias: kernel.machine_alias.clone(),
        relay_alias: None,
        kernel_alias: None,
        available_providers: Vec::new(),
        provider_accounts: Vec::new(),
        capabilities: Vec::new(),
        accepting_remote_leases: false,
        leased_agent_count: 0,
        local_session_count: 0,
        public_key: String::new(),
        local_protocol: kernel.protocol,
    }
}

fn unreachable_inventory(detail: String) -> GatewayError {
    GatewayError::ListUnavailable { detail }
}

pub(super) fn is_gateway_ready(kernel: &RelayKernelPresence) -> bool {
    advertises(kernel, GATEWAY_CAPABILITY)
}

/// A kernel that reads a request's bytes as bytes (`framed_plaintext`) says so; an older one
/// reads only JSON, so a request to it keeps its bytes in base64.
pub(super) fn reads_framed_plaintext(kernel: &RelayKernelPresence) -> bool {
    advertises(kernel, crate::transport::framed_plaintext::CAPABILITY)
}

/// A kernel that runs the Room terminal service says so; an older one would only answer
/// that it cannot read the request.
const ROOM_TERMINAL_CAPABILITY: &str = "room_terminal_v1";
const KERNEL_TERMINAL_CAPABILITY: &str = "kernel_terminal_v1";

fn advertises(kernel: &RelayKernelPresence, capability: &str) -> bool {
    kernel.capabilities.iter().any(|value| value == capability)
}

/// Why this request cannot go to this kernel, in words that say what to do, or `None`.
pub(super) fn incompatibility(
    kernel: &RelayKernelPresence,
    request: &crate::local::LocalDaemonRequest,
) -> Option<String> {
    use crate::local::LocalDaemonRequest;
    match request {
        LocalDaemonRequest::RespondToInteraction(_) if !is_gateway_ready(kernel) => Some(format!(
            "it predates the {GATEWAY_CAPABILITY} contract that classifies approvals itself"
        )),
        // A kernel without the gateway contract keeps no durable receipt, so a retry after a
        // crash could run the request twice: only requests that may simply run again go there.
        _ if !is_gateway_ready(kernel) && !super::ledger::is_repeatable(request) => Some(format!(
            "it predates the {GATEWAY_CAPABILITY} contract whose durable receipts make a retry safe"
        )),
        LocalDaemonRequest::RoomTerminal(request)
            if request.operation.is_exec() && !advertises(kernel, "room_exec_v1") =>
        {
            Some("it does not support binary Room exec; update the target kernel".to_string())
        }
        LocalDaemonRequest::RoomTerminal(_) if !advertises(kernel, ROOM_TERMINAL_CAPABILITY) => {
            Some("it predates Room terminals".into())
        }
        LocalDaemonRequest::KernelTerminal(_)
            if !advertises(kernel, KERNEL_TERMINAL_CAPABILITY) =>
        {
            Some("it predates the kernel's own terminals; update the target kernel".into())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::{GatewayKernelUpdate, GatewayKernelVia};
    use std::future::Future;
    use std::pin::Pin;

    struct Fixed(Mutex<Vec<RelayKernelPresence>>);

    impl InventorySource for Fixed {
        fn via(&self) -> GatewayKernelVia {
            GatewayKernelVia::Hosted
        }
        fn kernels(&self) -> Vec<RelayKernelPresence> {
            self.0.lock().unwrap().clone()
        }
        fn is_fresh(&self) -> bool {
            true
        }
        fn refreshed_within(&self, _max_age_ms: u64) -> bool {
            true
        }
        fn refresh(
            &self,
        ) -> Pin<Box<dyn Future<Output = Result<(), crate::error::DaemonError>> + Send + '_>>
        {
            Box::pin(async { Ok(()) })
        }
    }

    fn kernel(
        id: &str,
        relay_alias: Option<&str>,
        machine_alias: &str,
        ready: bool,
    ) -> RelayKernelPresence {
        RelayKernelPresence {
            kernel_id: id.into(),
            machine_id: format!("machine-{machine_alias}"),
            machine_alias: Some(machine_alias.into()),
            relay_alias: relay_alias.map(Into::into),
            kernel_alias: None,
            available_providers: Vec::new(),
            provider_accounts: Vec::new(),
            capabilities: if ready {
                vec![GATEWAY_CAPABILITY.into()]
            } else {
                Vec::new()
            },
            accepting_remote_leases: false,
            leased_agent_count: 0,
            local_session_count: 0,
            public_key: format!("key-{id}"),
            local_protocol: None,
        }
    }

    fn directory(kernels: Vec<RelayKernelPresence>) -> (Directory, Arc<Fixed>) {
        let source = Arc::new(Fixed(Mutex::new(kernels)));
        (Directory::new(source.clone()), source)
    }

    /// A list that is old answers at once and is renewed behind the request.
    struct Stale {
        listed: Vec<RelayKernelPresence>,
        refreshes: std::sync::atomic::AtomicU32,
    }

    impl InventorySource for Stale {
        fn via(&self) -> GatewayKernelVia {
            GatewayKernelVia::Hosted
        }
        fn kernels(&self) -> Vec<RelayKernelPresence> {
            self.listed.clone()
        }
        fn is_fresh(&self) -> bool {
            false
        }
        fn refreshed_within(&self, _max_age_ms: u64) -> bool {
            false
        }
        fn refresh(
            &self,
        ) -> Pin<Box<dyn Future<Output = Result<(), crate::error::DaemonError>> + Send + '_>>
        {
            self.refreshes.fetch_add(1, Ordering::SeqCst);
            // The Cloud answers slowly: a request that waited for this would stall.
            Box::pin(async {
                tokio::time::sleep(Duration::from_secs(30)).await;
                Ok(())
            })
        }
    }

    /// A list that is good enough to route by (fresh) but taken seconds ago.
    struct Aging {
        refreshes: std::sync::atomic::AtomicU32,
    }

    impl InventorySource for Aging {
        fn via(&self) -> GatewayKernelVia {
            GatewayKernelVia::Hosted
        }
        fn kernels(&self) -> Vec<RelayKernelPresence> {
            Vec::new()
        }
        fn is_fresh(&self) -> bool {
            true
        }
        fn refreshed_within(&self, _max_age_ms: u64) -> bool {
            false
        }
        fn refresh(
            &self,
        ) -> Pin<Box<dyn Future<Output = Result<(), crate::error::DaemonError>> + Send + '_>>
        {
            self.refreshes.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        }
    }

    #[tokio::test]
    async fn the_kernel_listing_renews_a_list_that_is_only_good_enough_to_route_by() {
        let source = Arc::new(Aging {
            refreshes: std::sync::atomic::AtomicU32::new(0),
        });
        let directory = Directory::new(source.clone());
        directory.kernels().await.unwrap();
        assert_eq!(source.refreshes.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_request_is_not_held_for_the_machine_list() {
        let source = Arc::new(Stale {
            listed: vec![kernel("k-b3", Some("b3"), "ovh-b3", true)],
            refreshes: std::sync::atomic::AtomicU32::new(0),
        });
        let directory = Directory::new(source.clone());
        let found = tokio::time::timeout(Duration::from_secs(2), directory.resolve("b3"))
            .await
            .expect("an old list answers without waiting for the refresh")
            .unwrap();
        assert_eq!(found.kernel_id, "k-b3");
        // Only one refresh runs behind the requests, however many arrive.
        for _ in 0..3 {
            directory.resolve("b3").await.unwrap();
        }
        tokio::task::yield_now().await;
        assert_eq!(source.refreshes.load(Ordering::SeqCst), 1);
        // A name the list does not show still asks for a fresh list before saying no.
        let unknown =
            tokio::time::timeout(Duration::from_millis(300), directory.resolve("nope")).await;
        assert!(unknown.is_err(), "an unknown name waits for the fresh list");
    }

    /// The relay cannot renew the list: every renewal opens a new socket to it, and a link
    /// busy with a bulk transfer lets each one time out.
    struct Unrenewable {
        listed: Vec<RelayKernelPresence>,
    }

    impl InventorySource for Unrenewable {
        fn via(&self) -> GatewayKernelVia {
            GatewayKernelVia::Hosted
        }
        fn kernels(&self) -> Vec<RelayKernelPresence> {
            self.listed.clone()
        }
        fn is_fresh(&self) -> bool {
            false
        }
        fn refreshed_within(&self, _max_age_ms: u64) -> bool {
            false
        }
        fn refresh(
            &self,
        ) -> Pin<Box<dyn Future<Output = Result<(), crate::error::DaemonError>> + Send + '_>>
        {
            Box::pin(std::future::pending())
        }
    }

    // MP-08 / MP-09 / MP-10 / MP-11 (relayflood, 2026-11-09 06:13-06:19Z): a bulk download
    // kept every list renewal from reaching the relay; five minutes later every call to a
    // kernel the last list showed failed with "the kernel list did not refresh in time".
    #[tokio::test(start_paused = true)]
    async fn a_list_the_relay_cannot_renew_still_routes_to_a_kernel_it_showed() {
        let source = Arc::new(Unrenewable {
            listed: vec![kernel("k-b3", Some("b3"), "ovh-b3", true)],
        });
        let directory = Directory::new(source.clone());
        assert_eq!(directory.resolve("b3").await.unwrap().kernel_id, "k-b3");
        // The incident: no renewal reached the relay for more than five minutes.
        tokio::time::advance(Duration::from_secs(6 * 60)).await;
        let found = tokio::time::timeout(Duration::from_secs(2), directory.resolve("b3"))
            .await
            .expect("a kernel the last list showed is routed to while the list cannot be renewed")
            .expect("a kernel the last list showed stays routable");
        assert_eq!(found.kernel_id, "k-b3");
    }

    #[test]
    fn a_machine_without_room_terminals_is_incompatible_before_anything_is_sent() {
        use crate::local::{LocalDaemonRequest, RoomTerminalOperation, RoomTerminalRequest};
        let terminal = LocalDaemonRequest::RoomTerminal(RoomTerminalRequest {
            session_id: "s".into(),
            acting_agent: None,
            host_agent: None,
            operation: RoomTerminalOperation::List,
        });
        let older = kernel("k-old", None, "old", true);
        assert!(incompatibility(&older, &terminal)
            .unwrap()
            .contains("Room terminals"));
        let mut current = older.clone();
        current.capabilities.push(ROOM_TERMINAL_CAPABILITY.into());
        assert_eq!(incompatibility(&current, &terminal), None);
        let list = LocalDaemonRequest::ListSessions(crate::local::ListSessionsRequest);
        assert_eq!(incompatibility(&older, &list), None);
    }

    // A kernel without durable receipts is never sent a state-changing request: a retry after
    // its crash could run it twice.
    #[test]
    fn a_kernel_without_durable_receipts_only_gets_requests_that_may_run_again() {
        use crate::local::LocalDaemonRequest;
        let legacy = kernel("k-legacy", None, "old", false);
        let create = LocalDaemonRequest::CreateSession(crate::session::CreateSessionRequest::new(
            "workspace",
            "worktree",
        ));
        assert!(incompatibility(&legacy, &create)
            .unwrap()
            .contains("durable receipts"));
        let list = LocalDaemonRequest::ListSessions(crate::local::ListSessionsRequest);
        assert_eq!(incompatibility(&legacy, &list), None);
        assert_eq!(
            incompatibility(&kernel("k-new", None, "new", true), &create),
            None
        );
    }

    fn with_protocol(
        mut kernel: RelayKernelPresence,
        protocol: Option<u32>,
    ) -> RelayKernelPresence {
        kernel.local_protocol = protocol;
        kernel
    }

    // Row 490: `chariox kernels` and the agent's kernel list say which protocol each kernel runs
    // and whether it needs an update, from the kernel's own registration; no client decides.
    #[tokio::test]
    async fn the_list_carries_each_kernels_protocol_and_whether_it_needs_an_update() {
        let current = super::super::update_needed::MINIMUM_TARGET_PROTOCOL;
        let (directory, _) = directory(vec![
            with_protocol(kernel("k-new", Some("new"), "m-new", true), Some(current)),
            with_protocol(
                kernel("k-old", Some("old"), "m-old", true),
                Some(current - 1),
            ),
            with_protocol(kernel("k-none", Some("none"), "m-none", false), None),
        ]);
        let listed = directory.kernels().await.unwrap();
        let of = |name: &str| listed.iter().find(|kernel| kernel.name == name).unwrap();
        assert_eq!(of("new").protocol, Some(current));
        assert_eq!(of("new").update, GatewayKernelUpdate::Current);
        assert_eq!(of("old").protocol, Some(current - 1));
        assert_eq!(of("old").update, GatewayKernelUpdate::UpdateNeeded);
        assert_eq!(of("none").protocol, None);
        assert_eq!(of("none").update, GatewayKernelUpdate::UpdateNeeded);
        // The CLI prints this JSON as it is: null and a state, no error text.
        let printed = serde_json::to_value(of("none")).unwrap();
        assert_eq!(printed["protocol"], serde_json::Value::Null);
        assert_eq!(printed["update"], "update_needed");
        assert_eq!(
            serde_json::to_value(of("new")).unwrap()["protocol"],
            current
        );
    }

    #[tokio::test]
    async fn an_update_changes_the_next_list_and_an_offline_kernel_keeps_its_last_values() {
        let current = super::super::update_needed::MINIMUM_TARGET_PROTOCOL;
        let (directory, source) = directory(vec![with_protocol(
            kernel("k-b3", Some("b3"), "ovh-b3", true),
            Some(current - 1),
        )]);
        let before = directory.kernels().await.unwrap();
        assert_eq!(before[0].update, GatewayKernelUpdate::UpdateNeeded);
        // Hot reload on the new build: the kernel registers again with the new protocol.
        source.0.lock().unwrap()[0].local_protocol = Some(current);
        let after = directory.kernels().await.unwrap();
        assert_eq!(after[0].protocol, Some(current));
        assert_eq!(after[0].update, GatewayKernelUpdate::Current);
        // It stops: listed offline for the reconnecting window with what it last reported.
        source.0.lock().unwrap().clear();
        let gone = directory.kernels().await.unwrap();
        assert_eq!(gone[0].status, GatewayKernelStatus::Offline);
        assert_eq!(gone[0].protocol, Some(current));
        assert_eq!(gone[0].update, GatewayKernelUpdate::Current);
    }

    /// A relay that refused one kernel for its age: the machine list names it, the live list
    /// does not.
    struct Refusing {
        live: Mutex<Vec<RelayKernelPresence>>,
        refused: Mutex<Vec<UpdateNeededKernel>>,
    }

    impl InventorySource for Refusing {
        fn via(&self) -> GatewayKernelVia {
            GatewayKernelVia::Hosted
        }
        fn kernels(&self) -> Vec<RelayKernelPresence> {
            self.live.lock().unwrap().clone()
        }
        fn update_needed_kernels(&self) -> Vec<UpdateNeededKernel> {
            self.refused.lock().unwrap().clone()
        }
        fn is_fresh(&self) -> bool {
            true
        }
        fn refreshed_within(&self, _max_age_ms: u64) -> bool {
            true
        }
        fn refresh(
            &self,
        ) -> Pin<Box<dyn Future<Output = Result<(), crate::error::DaemonError>> + Send + '_>>
        {
            Box::pin(async { Ok(()) })
        }
    }

    #[tokio::test]
    async fn a_kernel_the_relay_refused_for_its_age_is_listed_offline_and_needing_an_update() {
        let source = Arc::new(Refusing {
            live: Mutex::new(vec![with_protocol(
                kernel("k-b3", Some("b3"), "ovh-b3", true),
                Some(super::super::update_needed::MINIMUM_TARGET_PROTOCOL),
            )]),
            refused: Mutex::new(vec![
                UpdateNeededKernel {
                    kernel_id: "k-ancient".into(),
                    machine_id: "machine-pc".into(),
                    machine_alias: Some("pc".into()),
                    protocol: Some(1011),
                },
                UpdateNeededKernel {
                    kernel_id: "k-silent".into(),
                    machine_id: "machine-nas".into(),
                    machine_alias: None,
                    protocol: None,
                },
            ]),
        });
        let directory = Directory::new(source.clone());
        let listed = directory.kernels().await.unwrap();
        let ancient = listed
            .iter()
            .find(|kernel| kernel.kernel_id == "k-ancient")
            .unwrap();
        assert_eq!(ancient.name, "pc");
        assert_eq!(ancient.status, GatewayKernelStatus::Offline);
        assert_eq!(ancient.protocol, Some(1011));
        assert_eq!(ancient.update, GatewayKernelUpdate::UpdateNeeded);
        assert!(!ancient.gateway_ready);
        let silent = listed
            .iter()
            .find(|kernel| kernel.kernel_id == "k-silent")
            .unwrap();
        assert_eq!(
            (silent.protocol, silent.update),
            (None, GatewayKernelUpdate::UpdateNeeded)
        );
        assert_eq!(listed.len(), 3);
        // A listed refused kernel is never a route; asking for it says why, not "unknown".
        let refusal = directory.resolve("pc").await.unwrap_err();
        assert_eq!(refusal.code(), "kernel_update_needed");
        assert!(refusal.message().contains("1011"), "{}", refusal.message());
        assert_eq!(
            directory.resolve("k-silent").await.unwrap_err().code(),
            "kernel_offline"
        );
        // It is updated and registers: the refused entry is gone, the live one shows it current.
        source.refused.lock().unwrap().clear();
        source.live.lock().unwrap().push(with_protocol(
            kernel("k-ancient", Some("pc"), "pc", true),
            Some(super::super::update_needed::MINIMUM_TARGET_PROTOCOL),
        ));
        let listed = directory.kernels().await.unwrap();
        let pc: Vec<_> = listed
            .iter()
            .filter(|kernel| kernel.kernel_id == "k-ancient")
            .collect();
        assert_eq!(pc.len(), 1, "{listed:?}");
        assert_eq!(pc[0].status, GatewayKernelStatus::Online);
        assert_eq!(pc[0].update, GatewayKernelUpdate::Current);
    }

    #[tokio::test]
    async fn a_name_means_one_live_kernel_and_every_other_case_is_a_typed_error() {
        // Registrations name each kernel of a computer apart (`shared-host`, `shared-host-2`);
        // a kernel that registered without a relay name is known by its computer's name.
        let (directory, source) = directory(vec![
            kernel("k-b3", Some("b3"), "ovh-b3", true),
            kernel("k-old", Some("shared-host-2"), "shared-host", false),
            kernel("k-x", Some("shared-host"), "shared-host", true),
            kernel("k-y", None, "twin", true),
            kernel("k-z", None, "twin", true),
        ]);
        assert_eq!(directory.resolve("b3").await.unwrap().kernel_id, "k-b3");
        assert_eq!(directory.resolve("ovh-b3").await.unwrap().kernel_id, "k-b3");
        assert_eq!(directory.resolve("k-b3").await.unwrap().kernel_id, "k-b3");
        assert_eq!(
            directory.resolve("shared-host").await.unwrap().kernel_id,
            "k-x"
        );
        assert_eq!(
            directory.resolve("shared-host-2").await.unwrap().kernel_id,
            "k-old"
        );
        assert_eq!(
            directory.resolve("nope").await.unwrap_err().code(),
            "kernel_unknown"
        );
        let ambiguous = directory.resolve("twin").await.unwrap_err();
        assert_eq!(ambiguous.code(), "kernel_ambiguous");
        let message = ambiguous.message();
        assert!(
            message.contains("k-y") && message.contains("k-z"),
            "{message}"
        );
        assert_eq!(directory.resolve("k-z").await.unwrap().kernel_id, "k-z");
        // The kernel disappears from the live list: it stays known, offline, with its last-seen time.
        source.0.lock().unwrap().remove(0);
        assert_eq!(
            directory.resolve("b3").await.unwrap_err().code(),
            "kernel_offline"
        );
        let listed = directory.kernels().await.unwrap();
        let b3 = listed.iter().find(|m| m.name == "b3").unwrap();
        assert_eq!(b3.status, GatewayKernelStatus::Offline);
        assert!(b3.last_seen_ms > 0 && b3.gateway_ready);
        assert_eq!(b3.label, "b3");
        let old = listed.iter().find(|m| m.name == "shared-host-2").unwrap();
        assert!(!old.gateway_ready && old.status == GatewayKernelStatus::Online);
        assert!(
            listed
                .iter()
                .any(|m| m.name == "twin" && m.kernel_id == "k-y"),
            "a kernel without a relay name is shown by its computer's name"
        );
        // Gone for longer than the reconnecting window it is offline, and no list shows it.
        let gone_ms = crate::runtime::machine_presence::RECONNECTING_WINDOW_MS + 1;
        directory
            .seen
            .lock()
            .unwrap()
            .get_mut("k-b3")
            .unwrap()
            .last_seen_ms = crate::session::unix_epoch_ms() - gone_ms;
        let listed = directory.kernels().await.unwrap();
        assert!(listed.iter().all(|m| m.name != "b3"), "{listed:?}");
        assert!(listed.iter().any(|m| m.name == "shared-host"));
    }
}
