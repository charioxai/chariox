//! Per-worker stream ownership. A pending start has no socket and cannot
//! execute until the existing writer supplies its current-catalog transaction.
//! Public SDK decoding and connection/critical-receipt authority are separate.
mod ports;
mod requests;
pub(crate) use requests::HttpJob;

use super::{
    limits::{HttpLimits, LifetimeLease},
    policy::ApprovedTarget,
    transport::{self, Exchange, HttpTransport, ReceivePort, ResponseHead, UploadPort},
    HttpError, Result, STREAM_LIFETIME,
};
use crate::runtime::app_operation_budget::AppOperationBudget;
use chariox_app_runtime::{app_catalog::AppCatalog, worker_process::PrivateData};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, Weak},
};
use tokio::{
    runtime::Handle,
    sync::{watch, Mutex as AsyncMutex},
    task::JoinSet,
    time::Instant,
};

pub(super) use ports::ReadResult;
use ports::StreamPort;

fn stream_id() -> String {
    // Opaque v4-shaped handles use the kernel's existing random source.
    let bits = (rand::random::<u128>() & !((0xf_u128 << 76) | (0x3_u128 << 62)))
        | (4_u128 << 76)
        | (2_u128 << 62);
    let hex = format!("{bits:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

struct Scope {
    owner: String,
    catalog: Arc<AppCatalog>,
    // Derived only from the actual WorkerProcess. It pins that preparation
    // through tasks/readers even after the owning worker begins shutdown.
    _data: PrivateData,
}
struct State {
    closed: bool,
    entries: BTreeMap<String, Arc<Entry>>,
}
struct Inner {
    #[cfg(test)]
    fixture: Mutex<Option<Arc<super::fixture::NetworkFixture>>>,
    scope: Arc<Scope>,
    limits: Arc<HttpLimits>,
    runtime: Handle,
    state: Mutex<State>,
    tasks: AsyncMutex<JoinSet<()>>,
}
#[derive(Clone)]
pub(super) struct HttpStreams(Arc<Inner>);
pub(super) struct PendingStart {
    group: HttpStreams,
    id: String,
    entry: Arc<Entry>,
    exchange: Exchange,
    target: ApprovedTarget,
    transport: HttpTransport,
}
#[derive(Default)]
struct OperationGate {
    phase: std::sync::atomic::AtomicU8,
    wake: tokio::sync::Notify,
}
struct Entry {
    opening: Arc<OperationGate>,
    writing: Arc<OperationGate>,
    reading: Arc<OperationGate>,
    stop: watch::Sender<bool>,
    upload: AsyncMutex<UploadPort>,
    receive: AsyncMutex<Receive>,
    completed: watch::Receiver<Option<Result<()>>>,
    deadline: Instant,
    _lease: LifetimeLease,
    _scope: Arc<Scope>,
}
struct Receive {
    port: ReceivePort,
    head: Option<ResponseHead>,
}
struct TaskOwner {
    group: Weak<Inner>,
    id: String,
    stop: watch::Sender<bool>,
    _scope: Arc<Scope>,
    lease: LifetimeLease,
}
impl Drop for TaskOwner {
    fn drop(&mut self) {
        // Also runs if the executor aborts before the first poll or a transport
        // panics. A dead task cannot strand a handle and its unread buffers.
        retire(&self.group, &self.id, &self.stop);
    }
}

impl HttpStreams {
    pub(super) fn new(
        owner: String,
        catalog: Arc<AppCatalog>,
        data: PrivateData,
        limits: Arc<HttpLimits>,
        runtime: Handle,
    ) -> Result<Self> {
        if owner.is_empty()
            || data.installation_id() != catalog.installation_id()
            || data.generation() != catalog.generation()
            || data.release_digest() != catalog.package_digest()
        {
            return Err(HttpError::Provenance);
        }
        Ok(Self(Arc::new(Inner {
            #[cfg(test)]
            fixture: Mutex::new(None),
            scope: Arc::new(Scope {
                owner,
                catalog,
                _data: data,
            }),
            limits,
            runtime,
            state: Mutex::new(State {
                closed: false,
                entries: BTreeMap::new(),
            }),
            tasks: AsyncMutex::new(JoinSet::new()),
        })))
    }

    #[cfg(test)]
    pub(super) fn fixture_network(&self, fixture: Arc<super::fixture::NetworkFixture>) {
        *self.0.fixture.lock().unwrap() = Some(fixture);
    }

    /// Bounds allocation before writer admission. A dropped/expired pending
    /// start releases its buffers and reservation without ever opening a socket.
    pub(super) fn prepare(&self, target: ApprovedTarget, has_body: bool) -> Result<PendingStart> {
        if self
            .0
            .state
            .lock()
            .map_err(|_| HttpError::Cancelled)?
            .closed
        {
            return Err(HttpError::Cancelled);
        }
        let scope = &self.0.scope;
        target.require_catalog(&scope.catalog)?;
        // Read host DNS configuration only for an actual HTTP operation, outside
        // the durable writer. Offline hosts still start Apps without networking.
        let transport = HttpTransport::system()?;
        let lease = self
            .0
            .limits
            .acquire(&scope.owner, scope.catalog.installation_id())?
            .retain_worker(scope._data.clone())?;
        let (upload, receive, exchange) = transport::channels(has_body);
        let (stop, _) = watch::channel(false);
        // The sender is installed only when the writer starts the transport.
        let (_, completed) = watch::channel(None);
        let entry = Arc::new(Entry {
            opening: Arc::new(OperationGate::default()),
            writing: Arc::new(OperationGate::default()),
            reading: Arc::new(OperationGate::default()),
            stop,
            upload: AsyncMutex::new(upload),
            receive: AsyncMutex::new(Receive {
                port: receive,
                head: None,
            }),
            completed,
            deadline: Instant::now() + STREAM_LIFETIME,
            _lease: lease,
            _scope: self.0.scope.clone(),
        });
        Ok(PendingStart {
            group: self.clone(),
            id: stream_id(),
            entry,
            exchange,
            target,
            transport,
        })
    }

    /// Nonblocking host hook. Every existing operation sees cancellation before
    /// stored buffers are removed; outstanding readers retain their own lease.
    pub(super) fn begin_draining(&self) {
        let entries = {
            let mut state = self
                .0
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.closed = true;
            for entry in state.entries.values() {
                entry.stop.send_replace(true);
            }
            std::mem::take(&mut state.entries)
        };
        drop(entries);
    }

    /// The caller must await this before releasing the worker owner. Keeping
    /// the JoinSet behind its mutex makes cancelling this wait safe: a later
    /// join resumes ownership instead of detaching or discarding task handles.
    pub(super) async fn join(&self) {
        self.begin_draining();
        let mut tasks = self.0.tasks.lock().await;
        while tasks.join_next().await.is_some() {}
    }

    fn stored_entry(&self, id: &str) -> Result<Arc<Entry>> {
        if id.len() != 36 {
            return Err(HttpError::Invalid);
        }
        let state = self.0.state.lock().map_err(|_| HttpError::Cancelled)?;
        if state.closed {
            return Err(HttpError::Cancelled);
        }
        let entry = state.entries.get(id).cloned().ok_or(HttpError::Invalid)?;
        if *entry.stop.borrow() || Instant::now() >= entry.deadline {
            return Err(HttpError::Cancelled);
        }
        Ok(entry)
    }
    fn entry(&self, id: &str) -> Result<Arc<Entry>> {
        let entry = self.stored_entry(id)?;
        if entry
            .opening
            .phase
            .load(std::sync::atomic::Ordering::Acquire)
            != 0
        {
            return Err(HttpError::Busy);
        }
        Ok(entry)
    }
    pub(super) async fn await_publication(
        &self,
        command: &super::decode::Command,
        deadline: Instant,
        mut cancellation: chariox_app_runtime::worker_peer::BrokerCancellation,
    ) -> Result<()> {
        use super::decode::Command;
        let (id, write) = match command {
            Command::Write { id, .. } => (id, true),
            Command::Headers(id) | Command::Read(id) => (id, false),
            _ => return Ok(()),
        };
        let entry = self.stored_entry(id)?;
        let mut stopped = entry.stop.subscribe();
        for gate in [
            &entry.opening,
            if write {
                &entry.writing
            } else {
                &entry.reading
            },
        ] {
            loop {
                let notified = gate.wake.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                match gate.phase.load(std::sync::atomic::Ordering::Acquire) {
                    0 => break,
                    1 => return Err(HttpError::Busy),
                    _ => {}
                }
                tokio::select! { biased;
                    _ = super::cancelled(&mut stopped) => return Err(HttpError::Cancelled),
                    _ = cancellation.cancelled() => return Err(HttpError::Cancelled),
                    _ = tokio::time::sleep_until(deadline.min(entry.deadline)) => return Err(HttpError::Deadline),
                    _ = &mut notified => {},
                }
            }
        }
        Ok(())
    }
    fn port(&self, id: &str) -> Result<StreamPort> {
        Ok(StreamPort {
            group: Arc::downgrade(&self.0),
            id: id.into(),
            entry: self.entry(id)?,
        })
    }
    #[cfg(test)]
    pub(super) async fn write(
        &self,
        id: &str,
        bytes: bytes::Bytes,
        end: bool,
        deadline: Instant,
        cancellation: chariox_app_runtime::worker_peer::BrokerCancellation,
    ) -> Result<()> {
        self.port(id)?
            .write(bytes, end, deadline, cancellation)
            .await
    }
    #[cfg(test)]
    pub(super) async fn headers(
        &self,
        id: &str,
        deadline: Instant,
        cancellation: chariox_app_runtime::worker_peer::BrokerCancellation,
    ) -> Result<Option<ResponseHead>> {
        self.port(id)?.headers(deadline, cancellation).await
    }
    #[cfg(test)]
    pub(super) async fn read(
        &self,
        id: &str,
        deadline: Instant,
        cancellation: chariox_app_runtime::worker_peer::BrokerCancellation,
    ) -> Result<ReadResult> {
        self.port(id)?.read(deadline, cancellation).await
    }
    pub(super) fn cancel(&self, id: &str) -> Result<()> {
        let entry = self.entry(id)?;
        retire(&Arc::downgrade(&self.0), id, &entry.stop);
        Ok(())
    }
}

impl PendingStart {
    /// Called ONLY by the existing durable writer, on its read-only IMMEDIATE
    /// transaction. No request decoder receives a raw connection or this entry
    /// point. The transaction serializes the exact trust check with enqueueing;
    /// later cancellation cannot promise rollback of bytes already transmitted.
    pub(super) fn start_current(
        self,
        transaction: &rusqlite::Transaction<'_>,
        budget: &AppOperationBudget,
    ) -> Result<String> {
        let execution = budget.fork(|| false);
        self.start_with(
            transaction,
            budget,
            move |transport, target, exchange, stopped, lease, admitted| async move {
                execution.check().map_err(|_| HttpError::Cancelled)?;
                transport
                    .run(target, exchange, stopped, lease, admitted)
                    .await
            },
        )
    }
    // Private decomposition permits deterministic task/backpressure fixtures.
    // Every start, including those fixtures, goes through the same exact writer
    // trust/identity/budget fence below; production always uses HttpTransport.
    fn start_with<F, Fut>(
        mut self,
        transaction: &rusqlite::Transaction<'_>,
        budget: &AppOperationBudget,
        run: F,
    ) -> Result<String>
    where
        F: FnOnce(
                HttpTransport,
                ApprovedTarget,
                Exchange,
                watch::Receiver<bool>,
                LifetimeLease,
                Instant,
            ) -> Fut
            + Send
            + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        let inner = &self.group.0;
        budget.check().map_err(|_| HttpError::Cancelled)?;
        inner
            .scope
            .catalog
            .require_current(transaction, &inner.scope.owner)
            .map_err(|_| HttpError::Provenance)?;
        let mut state = inner.state.lock().map_err(|_| HttpError::Cancelled)?;
        if state.closed {
            return Err(HttpError::Cancelled);
        }
        let mut tasks = inner.tasks.try_lock().map_err(|_| HttpError::Busy)?;
        while tasks.try_join_next().is_some() {}
        budget.check().map_err(|_| HttpError::Cancelled)?;
        if Instant::now() >= self.entry.deadline {
            return Err(HttpError::Deadline);
        }
        if state.entries.contains_key(&self.id) {
            return Err(HttpError::Busy);
        }
        let (complete, completed) = watch::channel(None);
        Arc::get_mut(&mut self.entry)
            .ok_or(HttpError::Provenance)?
            .completed = completed;
        let stopped = self.entry.stop.subscribe();
        let lifetime = self.entry.deadline;
        let owner = TaskOwner {
            group: Arc::downgrade(inner),
            id: self.id.clone(),
            stop: self.entry.stop.clone(),
            _scope: inner.scope.clone(),
            lease: self.entry._lease.clone(),
        };
        state.entries.insert(self.id.clone(), self.entry);
        // A shutting-down executor may immediately drop the submitted future.
        // Its TaskOwner then retires the entry, so never hold that entry mutex
        // across spawn. The JoinSet lock still serializes submission with join;
        // a concurrent drain signals the inserted entry before execution.
        drop(state);
        tasks.spawn_on(
            async move {
                if super::stopped(&stopped) || Instant::now() >= lifetime {
                    drop(owner);
                    return;
                }
                let admitted = lifetime - STREAM_LIFETIME;
                let outcome = run(
                    self.transport,
                    self.target,
                    self.exchange,
                    stopped.clone(),
                    owner.lease.clone(),
                    admitted,
                )
                .await;
                complete.send_replace(Some(outcome));
                // Finished sockets may still have unread output. Retain admission
                // until EOF/cancel consumes it or the original lifetime expires.
                let mut stopped = stopped;
                tokio::select! {
                    _ = super::cancelled(&mut stopped) => {},
                    _ = tokio::time::sleep_until(lifetime) => {},
                }
                drop(owner);
            },
            &inner.runtime,
        );
        Ok(self.id)
    }
}

#[cfg(test)]
mod tests;

fn retire(group: &Weak<Inner>, id: &str, stop: &watch::Sender<bool>) {
    stop.send_replace(true);
    let Some(group) = group.upgrade() else {
        return;
    };
    let mut state = group
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if state
        .entries
        .get(id)
        .is_some_and(|entry| entry.stop.same_channel(stop))
    {
        state.entries.remove(id);
    }
}
impl Drop for Inner {
    fn drop(&mut self) {
        // Ordinary shutdown always joins; abort/panic fallback still retains
        // every task lease until the executor actually drops that task future.
        let state = self
            .state
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for entry in state.entries.values() {
            entry.stop.send_replace(true);
        }
        self.tasks.get_mut().abort_all();
    }
}
