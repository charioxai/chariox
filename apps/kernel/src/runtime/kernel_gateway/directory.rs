//! Kernel names to live kernels, kept current from the inventory source.
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chariox_relay::protocol::RelayKernelPresence;

use super::error::GatewayError;
use super::inventory::InventorySource;
use super::names;
use crate::local::{GatewayKernel, GatewayKernelStatus};

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
    /// A refresh behind a request is running; one at a time.
    refreshing: Arc<AtomicBool>,
}

impl Directory {
    pub(super) fn new(source: Arc<dyn InventorySource>) -> Self {
        Self {
            source,
            seen: Mutex::new(BTreeMap::new()),
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
        let known: Vec<&RelayKernelPresence> = seen.values().map(|seen| &seen.kernel).collect();
        Ok(names::grouped(
            seen.values()
                .map(|seen| GatewayKernel {
                    name: names::display_name(&seen.kernel, &known),
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
                })
                .collect(),
        ))
    }

    /// The one live kernel `name` means; unknown, ambiguous and offline are errors.
    pub(super) async fn resolve(&self, name: &str) -> Result<RelayKernelPresence, GatewayError> {
        // A request is never held for the kernel list: a kernel the last list showed live is
        // used at once and the list is renewed behind the request. Only an unknown or offline
        // name, or no list at all, asks for a fresh list before answering.
        if self.source.is_usable() {
            self.merge();
            if let Ok(kernel) = self.lookup(name) {
                self.refresh_behind();
                return Ok(kernel);
            }
        }
        self.refresh().await?;
        self.lookup(name)
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
    }
}

fn unreachable_inventory(detail: String) -> GatewayError {
    GatewayError::ListUnavailable { detail }
}

pub(super) fn is_gateway_ready(kernel: &RelayKernelPresence) -> bool {
    advertises(kernel, GATEWAY_CAPABILITY)
}

/// A kernel that runs the Room terminal service says so; an older one would only answer
/// that it cannot read the request.
const ROOM_TERMINAL_CAPABILITY: &str = "room_terminal_v1";

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
        LocalDaemonRequest::RoomTerminal(_) if !advertises(kernel, ROOM_TERMINAL_CAPABILITY) => {
            Some("it predates Room terminals".into())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::GatewayKernelVia;
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
        fn is_usable(&self) -> bool {
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
        kernel_alias: Option<&str>,
        machine_alias: &str,
        ready: bool,
    ) -> RelayKernelPresence {
        RelayKernelPresence {
            kernel_id: id.into(),
            machine_id: format!("machine-{machine_alias}"),
            machine_alias: Some(machine_alias.into()),
            relay_alias: None,
            kernel_alias: kernel_alias.map(Into::into),
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
        }
    }

    fn directory(kernels: Vec<RelayKernelPresence>) -> (Directory, Arc<Fixed>) {
        let source = Arc::new(Fixed(Mutex::new(kernels)));
        (Directory::new(source.clone()), source)
    }

    /// A list that is old but usable answers at once and is renewed behind the request.
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
        fn is_usable(&self) -> bool {
            true
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
        fn is_usable(&self) -> bool {
            true
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
            .expect("a usable list answers without waiting for the refresh")
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

    #[tokio::test]
    async fn a_name_means_one_live_kernel_and_every_other_case_is_a_typed_error() {
        let (directory, source) = directory(vec![
            kernel("k-b3", Some("b3"), "ovh-b3", true),
            kernel("k-old", Some("old"), "shared-host", false),
            kernel("k-x", None, "shared-host", true),
        ]);
        assert_eq!(directory.resolve("b3").await.unwrap().kernel_id, "k-b3");
        assert_eq!(directory.resolve("ovh-b3").await.unwrap().kernel_id, "k-b3");
        assert_eq!(directory.resolve("k-b3").await.unwrap().kernel_id, "k-b3");
        assert_eq!(
            directory.resolve("nope").await.unwrap_err().code(),
            "kernel_unknown"
        );
        // Two of the user's kernels on one machine: the machine's name fits both.
        let ambiguous = directory.resolve("shared-host").await.unwrap_err();
        assert_eq!(ambiguous.code(), "kernel_ambiguous");
        let message = ambiguous.message();
        assert!(
            message.contains("shared-host/old") && message.contains("shared-host/k-x"),
            "{message}"
        );
        assert_eq!(
            directory
                .resolve("shared-host/k-x")
                .await
                .unwrap()
                .kernel_id,
            "k-x"
        );
        // The kernel disappears from the live list: it stays known, offline, with its last-seen time.
        source.0.lock().unwrap().remove(0);
        assert_eq!(
            directory.resolve("b3").await.unwrap_err().code(),
            "kernel_offline"
        );
        let listed = directory.kernels().await.unwrap();
        let b3 = listed.iter().find(|m| m.name == "ovh-b3").unwrap();
        assert_eq!(b3.status, GatewayKernelStatus::Offline);
        assert!(b3.last_seen_ms > 0 && b3.gateway_ready);
        assert_eq!(b3.label, "b3");
        let old = listed.iter().find(|m| m.name == "shared-host/old").unwrap();
        assert!(!old.gateway_ready && old.status == GatewayKernelStatus::Online);
        assert!(
            listed
                .iter()
                .any(|m| m.name == "shared-host/k-x" && m.kernel_id == "k-x"),
            "a kernel without an alias is labelled by its id"
        );
    }
}
