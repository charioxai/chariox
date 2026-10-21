//! Machine names to live kernels, kept current from the inventory source.
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chariox_relay::protocol::RelayKernelPresence;

use super::error::GatewayError;
use super::inventory::InventorySource;
use crate::local::{GatewayMachine, GatewayMachineStatus};

/// The capability a kernel advertises when it enforces `on_behalf_of`.
const GATEWAY_CAPABILITY: &str = "machine_gateway_v1";
const REFRESH_TIMEOUT: Duration = Duration::from_secs(10);

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

    pub(super) async fn machines(&self) -> Result<Vec<GatewayMachine>, GatewayError> {
        self.refresh().await?;
        let via = self.source.via();
        let seen = self.seen.lock().expect("directory poisoned");
        Ok(seen
            .values()
            .map(|seen| GatewayMachine {
                name: display_name(&seen.kernel),
                kernel_id: seen.kernel.kernel_id.clone(),
                machine_id: seen.kernel.machine_id.clone(),
                machine_alias: seen.kernel.machine_alias.clone(),
                status: if seen.online {
                    GatewayMachineStatus::Online
                } else {
                    GatewayMachineStatus::Offline
                },
                last_seen_ms: seen.last_seen_ms,
                gateway_ready: is_gateway_ready(&seen.kernel),
                via,
            })
            .collect())
    }

    /// The one live kernel `name` means; unknown, ambiguous and offline are errors.
    pub(super) async fn resolve(&self, name: &str) -> Result<RelayKernelPresence, GatewayError> {
        // A request is never held for the machine list: a machine the last list showed live is
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
        let matches: Vec<&Seen> = seen
            .values()
            .filter(|seen| {
                names_of(&seen.kernel)
                    .iter()
                    .any(|candidate| candidate == name)
            })
            .collect();
        match matches.as_slice() {
            [] => Err(GatewayError::Unknown { name: name.into() }),
            [one] if one.online => Ok(one.kernel.clone()),
            [_] => Err(GatewayError::Offline { name: name.into() }),
            several => Err(GatewayError::Ambiguous {
                name: name.into(),
                candidates: several
                    .iter()
                    .map(|seen| seen.kernel.kernel_id.clone())
                    .collect(),
            }),
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
            tokio::time::timeout(REFRESH_TIMEOUT, self.source.refresh())
                .await
                .map_err(|_| {
                    unreachable_inventory("the machine list did not refresh in time".into())
                })?
                .map_err(|error| unreachable_inventory(error.to_string()))?;
        }
        self.merge();
        Ok(())
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
    GatewayError::Unreachable {
        name: "machine list".into(),
        detail,
    }
}

pub(super) fn display_name(kernel: &RelayKernelPresence) -> String {
    kernel
        .kernel_alias
        .clone()
        .or_else(|| kernel.relay_alias.clone())
        .or_else(|| kernel.machine_alias.clone())
        .unwrap_or_else(|| kernel.kernel_id.clone())
}

pub(super) fn names_of(kernel: &RelayKernelPresence) -> Vec<String> {
    [
        Some(kernel.kernel_id.clone()),
        kernel.kernel_alias.clone(),
        kernel.relay_alias.clone(),
        kernel.machine_alias.clone(),
    ]
    .into_iter()
    .flatten()
    .collect()
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
        LocalDaemonRequest::RoomTerminal(_) if !advertises(kernel, ROOM_TERMINAL_CAPABILITY) => {
            Some("it predates Room terminals".into())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::GatewayMachineVia;
    use std::future::Future;
    use std::pin::Pin;

    struct Fixed(Mutex<Vec<RelayKernelPresence>>);

    impl InventorySource for Fixed {
        fn via(&self) -> GatewayMachineVia {
            GatewayMachineVia::Hosted
        }
        fn kernels(&self) -> Vec<RelayKernelPresence> {
            self.0.lock().unwrap().clone()
        }
        fn is_fresh(&self) -> bool {
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
            machine_id: format!("machine-{id}"),
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
        fn via(&self) -> GatewayMachineVia {
            GatewayMachineVia::Hosted
        }
        fn kernels(&self) -> Vec<RelayKernelPresence> {
            self.listed.clone()
        }
        fn is_fresh(&self) -> bool {
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

    #[tokio::test]
    async fn a_name_means_one_live_kernel_and_every_other_case_is_a_typed_error() {
        let (directory, source) = directory(vec![
            kernel("k-b3", Some("b3"), "ovh-b3", true),
            kernel("k-old", Some("old"), "shared-host", false),
            kernel("k-x", None, "shared-host", true),
        ]);
        assert_eq!(directory.resolve("b3").await.unwrap().kernel_id, "k-b3");
        assert_eq!(directory.resolve("k-b3").await.unwrap().kernel_id, "k-b3");
        assert_eq!(
            directory.resolve("nope").await.unwrap_err().code(),
            "machine_unknown"
        );
        let ambiguous = directory.resolve("shared-host").await.unwrap_err();
        assert_eq!(ambiguous.code(), "machine_ambiguous");
        assert!(ambiguous.message().contains("k-old") && ambiguous.message().contains("k-x"));
        // The machine disappears from the live list: it stays known, offline, with its last-seen time.
        source.0.lock().unwrap().remove(0);
        assert_eq!(
            directory.resolve("b3").await.unwrap_err().code(),
            "machine_offline"
        );
        let listed = directory.machines().await.unwrap();
        let b3 = listed.iter().find(|m| m.name == "b3").unwrap();
        assert_eq!(b3.status, GatewayMachineStatus::Offline);
        assert!(b3.last_seen_ms > 0 && b3.gateway_ready);
        let old = listed.iter().find(|m| m.name == "old").unwrap();
        assert!(!old.gateway_ready && old.status == GatewayMachineStatus::Online);
        assert!(
            listed
                .iter()
                .any(|m| m.name == "shared-host" && m.kernel_id == "k-x"),
            "no kernel alias falls back to the machine alias"
        );
    }
}
