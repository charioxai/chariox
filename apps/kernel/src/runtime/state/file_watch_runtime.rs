//! Worker watches push bounded native invalidations to the home kernel. Leases release
//! them after an unclean home restart; renewals never stat or list an unchanged path.
use super::*;
use crate::machine_files::FileWatch;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use tokio::sync::watch;
use tokio::time::Instant;

const WATCH_LEASE: Duration = Duration::from_secs(90);
const RENEW_AFTER: Duration = Duration::from_secs(30);
const PEER_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_PEER_CLIENTS: usize = 16;

type WatchKey = (String, String);
struct WorkerWatch {
    paths: Vec<String>,
    renew: watch::Sender<Instant>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for WorkerWatch {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Default)]
pub(super) struct PeerFileWatches {
    clients: Arc<std::sync::Mutex<BTreeMap<String, usize>>>,
    workers: std::sync::Mutex<BTreeMap<WatchKey, WorkerWatch>>,
    homes: std::sync::Mutex<BTreeMap<WatchKey, watch::Sender<u64>>>,
}

impl PeerFileWatches {
    pub(super) fn shutdown(&self) {
        self.workers.lock().unwrap().clear();
        self.homes.lock().unwrap().clear();
    }
}

pub(crate) struct FileWatchPermit {
    clients: Arc<std::sync::Mutex<BTreeMap<String, usize>>>,
    client: String,
    count: usize,
}
impl Drop for FileWatchPermit {
    fn drop(&mut self) {
        let mut clients = self.clients.lock().unwrap();
        if let Some(count) = clients.get_mut(&self.client) {
            *count -= self.count;
            if *count == 0 {
                clients.remove(&self.client);
            }
        }
    }
}

/// Owned by the client's feed. Dropping it stops renewals and explicitly releases
/// the worker; a lost release is bounded by the worker's own lease.
pub(crate) struct RemoteFileWatch {
    runtime: KernelRuntimeState,
    owner: String,
    kernel: String,
    id: String,
    pub(crate) changes: watch::Receiver<u64>,
    renewal: tokio::task::JoinHandle<()>,
}
impl Drop for RemoteFileWatch {
    fn drop(&mut self) {
        self.renewal.abort();
        self.runtime
            .peer_file_watches
            .homes
            .lock()
            .unwrap()
            .remove(&(self.kernel.clone(), self.id.clone()));
        let (runtime, owner, kernel, id) = (
            self.runtime.clone(),
            self.owner.clone(),
            self.kernel.clone(),
            self.id.clone(),
        );
        tokio::spawn(async move {
            let _ = runtime
                .peer_send(
                    &owner,
                    &kernel,
                    RelayPeerRequest::UnwatchMachineFiles {
                        subscription_id: id,
                    },
                    PEER_TIMEOUT,
                )
                .await;
        });
    }
}

impl KernelRuntimeState {
    pub(crate) fn reserve_file_watches(
        &self,
        caller: &crate::runtime::command::KernelCaller,
        count: usize,
    ) -> Result<FileWatchPermit, String> {
        let client = format!(
            "{}:{}",
            caller.caller_id,
            caller.client_id.as_deref().unwrap_or("local")
        );
        let mut clients = self.peer_file_watches.clients.lock().unwrap();
        if clients.len() >= 256 && !clients.contains_key(&client) {
            return Err("too many filesystem watch clients".into());
        }
        let held = clients.entry(client.clone()).or_default();
        if *held + count > crate::machine_files::MAX_FILE_WATCH_PATHS {
            return Err(
                "watch at most 64 file paths per client; close an unused Files window".into(),
            );
        }
        *held += count;
        Ok(FileWatchPermit {
            clients: Arc::clone(&self.peer_file_watches.clients),
            client,
            count,
        })
    }

    pub(crate) async fn watch_remote_files(
        &self,
        kernel: &str,
        paths: Vec<String>,
    ) -> Result<RemoteFileWatch, DaemonError> {
        let owner = self.file_peer_owner()?;
        let kernel = self.resolve_peer_kernel(kernel).await?.ok_or_else(|| {
            crate::runtime::kernel_access::error("remote watch must name another kernel")
        })?;
        let id = format!("files-{:032x}", rand::random::<u128>());
        let (changed, changes) = watch::channel(0u64);
        let key = (kernel.clone(), id.clone());
        self.peer_file_watches
            .homes
            .lock()
            .unwrap()
            .insert(key.clone(), changed.clone());
        let request = RelayPeerRequest::WatchMachineFiles {
            subscription_id: id.clone(),
            paths,
        };
        match self
            .peer_send(&owner, &kernel, request.clone(), PEER_TIMEOUT)
            .await
        {
            Ok(RelayPeerResponse::MachineFilesWatched { .. }) => {}
            other => {
                self.peer_file_watches.homes.lock().unwrap().remove(&key);
                return Err(watch_failed(other));
            }
        }
        let (runtime, renew_owner, renew_kernel) = (self.clone(), owner.clone(), kernel.clone());
        let renewal = tokio::spawn(async move {
            loop {
                tokio::time::sleep(RENEW_AFTER).await;
                loop {
                    match runtime
                        .peer_send(&renew_owner, &renew_kernel, request.clone(), PEER_TIMEOUT)
                        .await
                    {
                        Ok(RelayPeerResponse::MachineFilesWatched { created }) => {
                            if created {
                                changed.send_modify(|v| *v = v.wrapping_add(1));
                            }
                            break;
                        }
                        _ => {
                            // Retry registration after a worker/relay restart. On success a fresh
                            // watch always invalidates what the disconnected client remembered.
                            tokio::time::sleep(Duration::from_millis(500)).await;
                        }
                    }
                }
            }
        });
        Ok(RemoteFileWatch {
            runtime: self.clone(),
            owner,
            kernel,
            id,
            changes,
            renewal,
        })
    }

    pub(crate) async fn serve_file_watch(
        &self,
        sender: &str,
        request: RelayPeerRequest,
    ) -> Result<RelayPeerResponse, DaemonError> {
        let response = match request {
            RelayPeerRequest::WatchMachineFiles {
                subscription_id,
                paths,
            } => self.register_worker_file_watch(sender, subscription_id, paths),
            RelayPeerRequest::UnwatchMachineFiles { subscription_id } => {
                self.peer_file_watches
                    .workers
                    .lock()
                    .unwrap()
                    .remove(&(sender.into(), subscription_id));
                Ok(RelayPeerResponse::MachineFilesWatched { created: false })
            }
            RelayPeerRequest::MachineFilesChanged { subscription_id } => {
                if let Some(changed) = self
                    .peer_file_watches
                    .homes
                    .lock()
                    .unwrap()
                    .get(&(sender.into(), subscription_id))
                {
                    changed.send_modify(|v| *v = v.wrapping_add(1));
                }
                Ok(RelayPeerResponse::MachineFilesWatched { created: false })
            }
            _ => unreachable!("file watch requests only"),
        };
        Ok(response.unwrap_or_else(|reason| RelayPeerResponse::FileRefused { reason }))
    }

    fn register_worker_file_watch(
        &self,
        sender: &str,
        id: String,
        paths: Vec<String>,
    ) -> Result<RelayPeerResponse, String> {
        if id.is_empty() || id.len() > 128 {
            return Err("invalid file watch subscription id".into());
        }
        let key = (sender.to_owned(), id.clone());
        let mut watches = self.peer_file_watches.workers.lock().unwrap();
        watches.retain(|_, watch| !watch.task.is_finished());
        if let Some(watch) = watches.get(&key) {
            if watch.paths != paths {
                return Err("replace a file watch with a new subscription id".into());
            }
            watch.renew.send_replace(Instant::now() + WATCH_LEASE);
            return Ok(RelayPeerResponse::MachineFilesWatched { created: false });
        }
        if watches
            .keys()
            .filter(|(kernel, _)| kernel == sender)
            .count()
            >= MAX_PEER_CLIENTS
            || watches.len() >= 256
        {
            return Err(
                "too many file watch clients on this worker; close an unused Files window".into(),
            );
        }
        let mut native = FileWatch::new(&paths, self.machine_state_guard())?;
        let (renew, mut lease) = watch::channel(Instant::now() + WATCH_LEASE);
        let (runtime, sender, owner) = (
            self.clone(),
            sender.to_owned(),
            self.file_peer_owner().map_err(|e| e.to_string())?,
        );
        let task = tokio::spawn(async move {
            loop {
                let until = *lease.borrow_and_update();
                tokio::select! {
                    changed = native.changed() => {
                        // Any backend error invalidates the remembered view as well. The next
                        // registration recreates the native watch rather than falling back to polling.
                        let failed = changed.is_err();
                        loop {
                            match runtime.peer_send(&owner, &sender, RelayPeerRequest::MachineFilesChanged { subscription_id: id.clone() }, PEER_TIMEOUT).await {
                                Ok(RelayPeerResponse::MachineFilesWatched { .. }) => break,
                                _ if Instant::now() < *lease.borrow() => tokio::time::sleep(Duration::from_millis(500)).await,
                                _ => return,
                            }
                        }
                        if failed { return; }
                    },
                    _ = tokio::time::sleep_until(until) => return,
                    renewed = lease.changed() => if renewed.is_err() { return },
                }
            }
        });
        watches.insert(key, WorkerWatch { paths, renew, task });
        Ok(RelayPeerResponse::MachineFilesWatched { created: true })
    }
}

fn watch_failed(answer: Result<RelayPeerResponse, DaemonError>) -> DaemonError {
    let message = match answer {
        Ok(RelayPeerResponse::FileRefused { reason }) => reason,
        Err(error) => error.to_string(),
        _ => "worker returned an unexpected file watch response".into(),
    };
    crate::runtime::kernel_access::error(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn client_file_watch_budget_is_shared_and_released_with_its_feed() {
        let app = crate::DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).unwrap();
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
            Arc::new(tokio::sync::Mutex::new(app)),
            4,
        );
        let runtime = router.runtime_state();
        let caller = crate::runtime::command::KernelCaller::default();
        let first = runtime.reserve_file_watches(&caller, 40).unwrap();
        assert!(runtime.reserve_file_watches(&caller, 25).is_err());
        let second = runtime.reserve_file_watches(&caller, 24).unwrap();
        drop(first);
        assert!(runtime.reserve_file_watches(&caller, 40).is_ok());
        drop(second);
        assert!(runtime.peer_file_watches.clients.lock().unwrap().is_empty());
        runtime.shutdown_cleanup().await.unwrap();
    }
}
