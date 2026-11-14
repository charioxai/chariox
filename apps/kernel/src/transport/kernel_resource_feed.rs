//! The kernel resource feed (docs/PROTOCOL.md §4.1.1): a client names the
//! resources it shows and the revision it holds; the kernel answers each with
//! `unchanged` or a `snapshot`, then sends a snapshot for every later change,
//! so no client polls the kernel for them.
//!
//! A watch is read with the request it replaces, as the subscription's caller,
//! so it is authorised like that request and carries the same public
//! projection. Revisions are content hashes of that projection, so they
//! survive a kernel restart.
#[path = "kernel_file_feed.rs"]
mod files;

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::Instant;

use crate::error::DaemonError;
use crate::local::{
    GetCredentialVaultStatusRequest, KernelResourceRefusal, KernelResourceWatch,
    ListAppInstallationsRequest, ListCredentialsRequest, ListProviderAccountProfilesRequest,
    LocalDaemonRequest,
};
use crate::runtime::command::{KernelCaller, KernelCommand, KernelCommandSource};
use crate::runtime::router::CommandRouter;
use crate::transport::kernel_protocol::{map_kernel_error, KernelEvent, KernelResourceChange};

pub(crate) const KERNEL_RESOURCES_SUBSCRIPTION_SCOPE: &str = "kernel_resources";
pub(crate) const KERNEL_RESOURCES_SENTINEL_ID: &str = "__kernel_resources__";
/// Changes no request signals (a provider login finishing in its own CLI, an
/// agent's tool using up a one-time Vault unlock) reach clients within this.
const UNSIGNALLED_CHANGE_INTERVAL: Duration = Duration::from_secs(10);
/// A Vault lease is read again this long after it ends, so the lock is pushed.
const LEASE_END_GRACE: Duration = Duration::from_millis(50);
const APP_INSTALLATION_PAGE: u32 = 100;
const WATCH_INBOX_CAPACITY: usize = 8;

/// Command types that change a feed resource; reads among them change nothing.
const CHANGING_COMMANDS: [&str; 8] = [
    "credential",
    "vault.",
    "provider_account.",
    "provider.login.",
    "provider.logout",
    "app.install",
    "app.update",
    "app.uninstall",
];
const READ_COMMAND_SUFFIXES: [&str; 4] = [".list", ".get", ".status", ".copy_offers"];

/// Whether a request of this command type may change what a feed shows.
/// Changes no request makes reach clients on the unsignalled interval.
pub(crate) fn may_change_resources(command_type: &str) -> bool {
    CHANGING_COMMANDS
        .iter()
        .any(|prefix| command_type.starts_with(prefix))
        && !READ_COMMAND_SUFFIXES
            .iter()
            .any(|suffix| command_type.ends_with(suffix))
}

/// Bumped after every request that may change a watched resource.
#[derive(Clone)]
pub(crate) struct KernelResourceChanges(Arc<watch::Sender<u64>>);

impl Default for KernelResourceChanges {
    fn default() -> Self {
        Self(Arc::new(watch::channel(0).0))
    }
}

impl KernelResourceChanges {
    pub(crate) fn notify(&self) {
        self.0
            .send_modify(|sequence| *sequence = sequence.wrapping_add(1));
    }

    fn subscribe(&self) -> watch::Receiver<u64> {
        self.0.subscribe()
    }
}

/// A new watch set for one subscription, read as `caller`, the sender of
/// `WatchKernelResources`; `reply` receives the watches the kernel refused.
pub(crate) struct WatchUpdate {
    pub(crate) watches: Vec<KernelResourceWatch>,
    pub(crate) caller: KernelCaller,
    pub(crate) reply: oneshot::Sender<Vec<KernelResourceRefusal>>,
}

pub(crate) type KernelResourceWatchSender = mpsc::Sender<WatchUpdate>;

pub(crate) fn watch_inbox() -> (KernelResourceWatchSender, mpsc::Receiver<WatchUpdate>) {
    mpsc::channel(WATCH_INBOX_CAPACITY)
}

/// Hands a watch set to the subscription's feed and waits for its refusals.
pub(crate) async fn submit_watch_set(
    inbox: &KernelResourceWatchSender,
    watches: Vec<KernelResourceWatch>,
    caller: KernelCaller,
) -> Result<Vec<KernelResourceRefusal>, DaemonError> {
    let (reply, refused) = oneshot::channel();
    let ended = || DaemonError::MachineRequest {
        code: "kernel_resources_subscription_ended".into(),
        message: "the kernel_resources subscription ended; subscribe again, then watch".into(),
    };
    inbox
        .send(WatchUpdate {
            watches,
            caller,
            reply,
        })
        .await
        .map_err(|_| ended())?;
    refused.await.map_err(|_| ended())
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum FeedResource {
    VaultStatus,
    VaultEntries,
    ProviderAccounts,
    AppInstallations { session_id: Option<String> },
}

impl FeedResource {
    fn parse(resource: &str, params: &Value) -> Result<Self, (&'static str, String)> {
        let Some(params) = params.as_object() else {
            return Err(("invalid_params", "params must be an object".into()));
        };
        let allow_only =
            |names: &[&str]| match params.keys().find(|key| !names.contains(&key.as_str())) {
                Some(key) => Err((
                    "invalid_params",
                    format!("`{resource}` takes no param `{key}`"),
                )),
                None => Ok(()),
            };
        match resource {
            "vault_status" => allow_only(&[]).map(|()| Self::VaultStatus),
            "vault_entries" => allow_only(&[]).map(|()| Self::VaultEntries),
            "provider_accounts" => allow_only(&[]).map(|()| Self::ProviderAccounts),
            "app_installations" => {
                allow_only(&["session_id"])?;
                let session_id = match params.get("session_id") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(id)) => Some(id.clone()),
                    Some(_) => {
                        return Err(("invalid_params", "`session_id` must be a string".into()))
                    }
                };
                Ok(Self::AppInstallations { session_id })
            }
            _ => Err((
                "unknown_resource",
                format!("this kernel has no resource `{resource}`"),
            )),
        }
    }
}

struct Watched {
    resource: String,
    params: Value,
    revision: String,
    /// When the Vault lease this projection shows ends.
    lease_ends_at_ms: Option<u64>,
}

struct Feed {
    router: Arc<CommandRouter>,
    source: KernelCommandSource,
    caller: Option<KernelCaller>,
    watched: BTreeMap<FeedResource, Watched>,
    reads: u64,
}

impl Feed {
    async fn replace(
        &mut self,
        watches: Vec<KernelResourceWatch>,
        caller: KernelCaller,
    ) -> (Vec<KernelResourceRefusal>, Vec<KernelEvent>) {
        self.caller = Some(caller);
        let mut previous = std::mem::take(&mut self.watched);
        let mut refused = Vec::new();
        let mut events = Vec::new();
        for watch in watches {
            let resource = match FeedResource::parse(&watch.resource, &watch.params) {
                Ok(resource) => resource,
                Err((code, message)) => {
                    refused.push(refusal(watch, code.into(), message));
                    continue;
                }
            };
            if self.watched.contains_key(&resource) {
                continue;
            }
            if let Some(kept) = previous.remove(&resource) {
                if watch.known_revision.as_deref() == Some(kept.revision.as_str()) {
                    self.watched.insert(resource, kept);
                    continue;
                }
            }
            match self.read(&resource).await {
                Ok(data) => {
                    let revision = revision_of(&data);
                    let body = if watch.known_revision.as_deref() == Some(revision.as_str()) {
                        KernelResourceChange::Unchanged {}
                    } else {
                        KernelResourceChange::Snapshot { data: data.clone() }
                    };
                    events.push(KernelEvent::KernelResourceChanged {
                        resource: watch.resource.clone(),
                        params: watch.params.clone(),
                        revision: revision.clone(),
                        body,
                    });
                    self.watched.insert(
                        resource,
                        Watched {
                            resource: watch.resource,
                            params: watch.params,
                            revision,
                            lease_ends_at_ms: lease_end(&data),
                        },
                    );
                }
                Err(error) => {
                    let error = map_kernel_error(&error);
                    refused.push(refusal(watch, error.code, error.message));
                }
            }
        }
        (refused, events)
    }

    /// A snapshot of every watched resource whose projection changed.
    async fn refresh(&mut self) -> Vec<KernelEvent> {
        let resources = self.watched.keys().cloned().collect::<Vec<_>>();
        let mut events = Vec::new();
        for resource in resources {
            let data = match self.read(&resource).await {
                Ok(data) => data,
                Err(error) => {
                    // The client keeps the copy it holds; the next change or interval reads again.
                    crate::logging::warn_with_fields(
                        "daemon.kernel_resource_feed",
                        "watched resource read failed",
                        serde_json::json!({ "resource": format!("{resource:?}"), "error": error.to_string() }),
                    );
                    continue;
                }
            };
            let revision = revision_of(&data);
            let Some(watched) = self.watched.get_mut(&resource) else {
                continue;
            };
            watched.lease_ends_at_ms = lease_end(&data);
            if watched.revision == revision {
                continue;
            }
            watched.revision = revision.clone();
            events.push(KernelEvent::KernelResourceChanged {
                resource: watched.resource.clone(),
                params: watched.params.clone(),
                revision,
                body: KernelResourceChange::Snapshot { data },
            });
        }
        events
    }

    fn next_lease_end(&self) -> Option<Instant> {
        let ends_at_ms = self
            .watched
            .values()
            .filter_map(|watched| watched.lease_ends_at_ms)
            .min()?;
        let left = ends_at_ms.saturating_sub(crate::session::unix_epoch_ms());
        Some(Instant::now() + Duration::from_millis(left) + LEASE_END_GRACE)
    }

    async fn read(&mut self, resource: &FeedResource) -> Result<Value, DaemonError> {
        match resource {
            FeedResource::VaultStatus => {
                self.read_request(LocalDaemonRequest::GetCredentialVaultStatus(
                    GetCredentialVaultStatusRequest,
                ))
                .await
            }
            FeedResource::VaultEntries => {
                self.read_request(LocalDaemonRequest::ListCredentials(ListCredentialsRequest))
                    .await
            }
            FeedResource::ProviderAccounts => {
                let mut accounts = self
                    .read_request(LocalDaemonRequest::ListProviderAccountProfiles(
                        ListProviderAccountProfilesRequest { provider: None },
                    ))
                    .await?;
                without_validation_times(&mut accounts);
                Ok(accounts)
            }
            FeedResource::AppInstallations { session_id } => {
                self.read_app_installations(session_id.clone()).await
            }
        }
    }

    /// Every page, so the snapshot is the whole list.
    async fn read_app_installations(
        &mut self,
        session_id: Option<String>,
    ) -> Result<Value, DaemonError> {
        let mut installations = Vec::new();
        let mut after = None;
        loop {
            let mut page = self
                .read_request(LocalDaemonRequest::ListAppInstallations(
                    ListAppInstallationsRequest {
                        session_id: session_id.clone(),
                        after,
                        limit: Some(APP_INSTALLATION_PAGE),
                    },
                ))
                .await?;
            if let Some(Value::Array(rows)) = page.get_mut("installations").map(Value::take) {
                installations.extend(rows);
            }
            match page.get("next_cursor").and_then(Value::as_str) {
                Some(cursor) => after = Some(cursor.to_owned()),
                None => break,
            }
        }
        Ok(serde_json::json!({ "installations": installations }))
    }

    async fn read_request(&mut self, request: LocalDaemonRequest) -> Result<Value, DaemonError> {
        let caller = self
            .caller
            .clone()
            .ok_or_else(|| DaemonError::MachineRequest {
                code: "kernel_resources_unwatched".into(),
                message: "send WatchKernelResources before reading".into(),
            })?;
        self.reads += 1;
        let command = KernelCommand::from_local_request_with_caller(
            format!(
                "kernel-resources-{}-{}",
                crate::session::unix_epoch_ms(),
                self.reads
            ),
            self.source.clone(),
            caller,
            None,
            None,
            &request,
        );
        self.router
            .authorize_relay_collaborator_request(&command, &request)?;
        let response = self.router.dispatch(command, request).await?;
        let mut value =
            serde_json::to_value(response).map_err(|error| DaemonError::LocalTransport {
                operation: "serialize kernel resource",
                message: error.to_string(),
            })?;
        crate::local::redact_client_response_value(&mut value);
        // A response is `{ "<Variant>": { fields } }`; the projection is its fields.
        Ok(match value {
            Value::Object(mut variant) if variant.len() == 1 => variant
                .values_mut()
                .next()
                .map(Value::take)
                .unwrap_or(Value::Null),
            other => other,
        })
    }
}

fn refusal(watch: KernelResourceWatch, code: String, message: String) -> KernelResourceRefusal {
    KernelResourceRefusal {
        resource: watch.resource,
        params: watch.params,
        code,
        message,
    }
}

/// When the kernel last checked an account is drawn by no client, and every check moves it (each kernel start runs one): left in the
/// projection, a client's remembered accounts would never be answered `unchanged` after a restart.
fn without_validation_times(accounts: &mut Value) {
    if let Some(Value::Array(profiles)) = accounts.get_mut("profiles") {
        for profile in profiles.iter_mut().filter_map(Value::as_object_mut) {
            profile.remove("last_validated_at_ms");
        }
    }
}

fn revision_of(data: &Value) -> String {
    let digest = Sha256::digest(serde_json::to_vec(data).unwrap_or_default());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn lease_end(data: &Value) -> Option<u64> {
    let status = data.get("status")?;
    if status.get("unlocked")?.as_bool()? {
        status.get("expires_at_ms")?.as_u64()
    } else {
        None
    }
}

/// Serves one `kernel_resources` subscription until its inbox closes or
/// `emit` reports the subscriber gone.
pub(crate) async fn run_kernel_resource_feed<E, F>(
    router: Arc<CommandRouter>,
    source: KernelCommandSource,
    mut updates: mpsc::Receiver<WatchUpdate>,
    heartbeat_interval: Duration,
    mut emit: E,
) where
    E: FnMut(KernelEvent) -> F,
    F: Future<Output = bool>,
{
    let (file_events, mut file_changes) = mpsc::channel(1);
    let mut file_watch = None;
    let mut changes = router.kernel_resource_changes().subscribe();
    changes.borrow_and_update();
    let mut feed = Feed {
        router,
        source,
        caller: None,
        watched: BTreeMap::new(),
        reads: 0,
    };
    let mut next_heartbeat = Instant::now();
    let mut next_unsignalled = Instant::now() + UNSIGNALLED_CHANGE_INTERVAL;
    loop {
        let lease_end = feed.next_lease_end();
        let wake = [Some(next_heartbeat), Some(next_unsignalled), lease_end]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(next_heartbeat);
        let events = tokio::select! {
            update = updates.recv() => {
                let Some(update) = update else { return };
                let (files, resources): (Vec<_>, Vec<_>) = update.watches.into_iter().partition(|watch| watch.resource == "file_changes");
                file_watch = None;
                let (mut refused, mut events) = feed.replace(resources, update.caller.clone()).await;
                if files.len() > 1 {
                    refused.extend(files.into_iter().map(|watch| refusal(watch, "file_watch_limit".into(), "send one file_changes watch set per client".into())));
                } else if let Some(watch) = files.into_iter().next() {
                    match files::start(Arc::clone(&feed.router), feed.source.clone(), update.caller, &watch, file_events.clone()).await {
                        Ok((running, initial)) => { file_watch = Some(running); events.push(initial); },
                        Err(message) => refused.push(refusal(watch, "file_watch_refused".into(), message)),
                    }
                }
                let _ = update.reply.send(refused);
                events
            }
            Some(event) = file_changes.recv(), if file_watch.is_some() => vec![event],
            changed = changes.changed() => {
                if changed.is_err() {
                    return;
                }
                feed.refresh().await
            }
            () = tokio::time::sleep_until(wake) => {
                let now = Instant::now();
                let mut events = Vec::new();
                if now >= next_heartbeat {
                    next_heartbeat = now + heartbeat_interval;
                    events.push(KernelEvent::Heartbeat {
                        session_id: KERNEL_RESOURCES_SENTINEL_ID.to_string(),
                    });
                }
                if now >= next_unsignalled || lease_end.is_some_and(|end| now >= end) {
                    next_unsignalled = now + UNSIGNALLED_CHANGE_INTERVAL;
                    events.extend(feed.refresh().await);
                }
                events
            }
        };
        for event in events {
            if !emit(event).await {
                return;
            }
        }
    }
}

#[cfg(test)]
#[path = "kernel_resource_feed_tests.rs"]
mod tests;
