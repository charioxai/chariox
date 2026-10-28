//! An instance is replaced as a unit: Wasmtime cannot safely detach a single
//! abandoned guest task from its shared memory and pending imports.
use crate::{
    attribution::Attribution,
    bindings::Worker,
    host::{Host, Registration},
    ipc::{error, Ipc},
    limits::{Limits, MemoryLimit, Stop},
    scope::Scope,
};
use chariox_app_runtime::wire::Message;
use std::{
    collections::{BTreeSet, HashMap, VecDeque},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::mpsc;
use wasmtime::{
    component::{Component, HasSelf, InstancePre, Linker},
    Engine, Store,
};

mod dispatch;
use dispatch::{serve, Active, Operation};
const TICK: Duration = Duration::from_millis(5);
#[derive(Clone, Default)]
pub struct RetryPolicy(BTreeSet<String>);
impl RetryPolicy {
    /// Call only with the manifest from the kernel's verified package.
    pub fn from_manifest(manifest: &chariox_app_package::v2::ManifestV2) -> Self {
        Self(manifest.idempotent_tools.iter().cloned().collect())
    }
    fn permits(&self, operation: &Operation) -> bool {
        matches!(operation,Operation::Tool { name,.. } if self.0.contains(name))
    }
}
pub struct WorkerEngine {
    engine: Engine,
    stop: Arc<AtomicBool>,
    ticker: Option<std::thread::JoinHandle<()>>,
}
impl WorkerEngine {
    pub fn new() -> wasmtime::Result<Self> {
        let engine = crate::engine::engine()?;
        let ticking = engine.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let ticker = std::thread::Builder::new()
            .name("app-epoch".into())
            .spawn(move || {
                while !stopped.load(Ordering::Relaxed) {
                    std::thread::sleep(TICK);
                    ticking.increment_epoch();
                }
            })?;
        Ok(Self {
            engine,
            stop,
            ticker: Some(ticker),
        })
    }
    pub fn engine(&self) -> &Engine {
        &self.engine
    }
    pub fn compile(&self, bytes: &[u8], world: &str) -> wasmtime::Result<Component> {
        chariox_app_wit::check_component(bytes, world)?;
        Component::new(&self.engine, bytes)
    }
    pub fn precompile(&self, bytes: &[u8], world: &str) -> wasmtime::Result<Vec<u8>> {
        chariox_app_wit::check_component(bytes, world)?;
        self.engine.precompile_component(bytes)
    }
}
impl Drop for WorkerEngine {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(ticker) = self.ticker.take() {
            let _ = ticker.join();
        }
    }
}

pub struct AppWorker {
    engine: WorkerEngine,
    pre: InstancePre<Host>,
    limits: Limits,
    retry: RetryPolicy,
    directories: Option<crate::wasi::InstallationDirectories>,
}
impl AppWorker {
    pub fn new(
        engine: WorkerEngine,
        component: Component,
        limits: Limits,
        retry: RetryPolicy,
    ) -> wasmtime::Result<Self> {
        limits.validate()?;
        let linker = Self::linker(engine.engine())?;
        Self::with_linker(engine, component, linker, limits, retry)
    }
    pub fn with_linker(
        engine: WorkerEngine,
        component: Component,
        linker: Linker<Host>,
        limits: Limits,
        retry: RetryPolicy,
    ) -> wasmtime::Result<Self> {
        limits.validate()?;
        let pre = linker.instantiate_pre(&component)?;
        Ok(Self {
            engine,
            pre,
            limits,
            retry,
            directories: None,
        })
    }
    /// Provide only the installation directories admitted by the supervisor.
    /// Without this call the worker exposes no filesystem preopens.
    pub fn with_directories(mut self, directories: crate::wasi::InstallationDirectories) -> Self {
        self.directories = Some(directories);
        self
    }
    /// The same allowlisted WASI/broker linker is used by CLI and library users.
    pub fn linker(engine: &Engine) -> wasmtime::Result<Linker<Host>> {
        let mut linker = Linker::new(engine);
        Worker::add_to_linker::<Host, HasSelf<Host>>(&mut linker, |host| host)?;
        crate::wasi::add_to_linker(&mut linker)?;
        Ok(linker)
    }
    async fn instantiate(&self, ipc: Ipc) -> wasmtime::Result<(Store<Host>, Worker, Attribution)> {
        let host = Host::new(ipc, self.limits.memory_bytes, self.directories.as_ref())?;
        let attribution = host.attribution.clone();
        let mut store = Store::new(self.engine.engine(), host);
        store.limiter(|host| &mut host.memory);
        store.task_group_hook(attribution.clone());
        store.epoch_deadline_trap();
        store.set_epoch_deadline(
            self.limits
                .slice_ms
                .min(self.limits.call_ms)
                .div_ceil(TICK.as_millis() as u64),
        );
        let instance = self.pre.instantiate_async(&mut store).await?;
        let worker = Worker::new(&mut store, &instance)?;
        worker
            .chariox_app_handlers()
            .call_register(&mut store)
            .await?
            .map_err(|error| {
                wasmtime::Error::msg(format!("App register failed: {}", error.code))
            })?;
        store.data_mut().sealed = true;
        crate::limits::invocation_limits(&mut store, self.limits);
        Ok((store, worker, attribution))
    }
    pub async fn run(
        &self,
        ipc: Ipc,
        incoming: &mut mpsc::Receiver<Message>,
    ) -> wasmtime::Result<()> {
        let mut accepted = HashMap::<String, Active>::new();
        let mut registration: Option<Registration> = None;
        let mut queued = VecDeque::new();
        let mut restart: Option<(Option<String>, &'static str)> = None;
        loop {
            let startup = tokio::time::timeout(
                Duration::from_millis(self.limits.call_ms),
                self.instantiate(ipc.clone()),
            )
            .await
            .unwrap_or_else(|_| {
                Err(wasmtime::Error::msg(
                    "APP_STARTUP_TIMEOUT: App startup exceeded its deadline",
                ))
            });
            let (mut store, worker, attribution) = match startup {
                Ok(value) => value,
                Err(failure) => {
                    for (id, _) in accepted.drain() {
                        ipc.respond(id, Err(error("APP_UNAVAILABLE", "App could not restart")))
                            .await?;
                    }
                    return Err(failure);
                }
            };
            let readiness = match &registration {
                None => {
                    let scope = Scope::new(
                        "worker-ready".into(),
                        tokio::time::Instant::now() + Duration::from_secs(30),
                    );
                    let params = serde_json::to_value(&store.data().registration)?;
                    let ipc = ipc.clone();
                    registration = Some(store.data().registration.clone());
                    Some(async move {
                        ipc.call(&scope, "worker.ready", params)
                            .await
                            .map_err(|error| {
                                wasmtime::Error::msg(format!("{}: {}", error.code, error.message))
                            })
                    })
                }
                Some(expected) => {
                    wasmtime::ensure!(
                        expected == &store.data().registration,
                        "App registration changed after instance restart"
                    );
                    None
                }
            };
            // Cancellation may arrive while the replacement is initializing.
            // Apply it before deciding whether any old invocation can replay.
            for _ in 0..crate::ipc::CAPACITY {
                let Ok(message) = incoming.try_recv() else {
                    break;
                };
                match message {
                    Message::Cancel { id, .. } if accepted.contains_key(&id) => {
                        let active = accepted.get_mut(&id).unwrap();
                        active.cancellation = Some("CANCELLED");
                        active.scope.cancel("CANCELLED");
                    }
                    message => queued.push_back(message),
                }
            }
            if let Some((culprit, code)) = restart.take() {
                let failed: Vec<_> = accepted.keys().cloned().collect();
                for id in failed {
                    let active = accepted.get_mut(&id).unwrap();
                    if Some(&id) != culprit.as_ref()
                        && active.cancellation.is_none()
                        && active.attempts == 0
                        && self.retry.permits(&active.request.operation)
                        && active.request.deadline > tokio::time::Instant::now()
                    {
                        active.attempts = 1;
                        active.scope = Scope::new(id, active.request.deadline);
                    } else {
                        let active = accepted.remove(&id).unwrap();
                        let code = if let Some(reason) = active.cancellation {
                            reason
                        } else if active.request.deadline <= tokio::time::Instant::now() {
                            "DEADLINE_EXCEEDED"
                        } else if Some(&id) == culprit.as_ref() {
                            code
                        } else {
                            "APP_INSTANCE_RESTARTED"
                        };
                        let mut failure = error(
                            code,
                            format!(
                                "App instance lost; culprit: {}",
                                culprit.as_deref().unwrap_or("unknown")
                            ),
                        );
                        failure.retryable = Some(
                            code == "APP_INSTANCE_RESTARTED"
                                && self.retry.permits(&active.request.operation),
                        );
                        ipc.respond(id, Err(failure)).await?;
                    }
                }
            }
            let result = store
                .run_concurrent(async |accessor| {
                    let dispatch = serve(
                        accessor,
                        &worker,
                        &ipc,
                        incoming,
                        &mut queued,
                        &mut accepted,
                        self.limits,
                    );
                    tokio::pin!(dispatch);
                    // The kernel holds the ready response until activation,
                    // after its local health check. Serve lifecycle requests
                    // while this request retains the activation budget.
                    if let Some(readiness) = readiness {
                        tokio::select! {
                            result = &mut dispatch => result,
                            ready = readiness => { ready?; dispatch.await }
                        }
                    } else {
                        dispatch.await
                    }
                })
                .await;
            let mut loss = match result {
                Ok(Ok(None)) => return Ok(()),
                Ok(Ok(Some(loss))) => loss,
                Ok(Err(error)) => return Err(error),
                Err(trap) => {
                    let code = if trap.downcast_ref::<MemoryLimit>().is_some() {
                        "APP_MEMORY_LIMIT"
                    } else if let Some(stop) = trap.downcast_ref::<Stop>() {
                        stop.0
                    } else {
                        "APP_TRAPPED"
                    };
                    (attribution.culprit(), code)
                }
            };
            if let Some(reason) = loss
                .0
                .as_ref()
                .and_then(|id| accepted.get(id))
                .and_then(|active| active.scope.reason())
            {
                loss.1 = reason;
            }
            eprintln!(
                "App instance stopped: {}; culprit: {}",
                loss.1,
                loss.0.as_deref().unwrap_or("unknown")
            );
            for active in accepted.values() {
                active.scope.cancel("APP_INSTANCE_RESTARTED");
            }
            // Dropping the store disposes all guest futures/streams before a
            // replacement is created. Only signed idempotent tools are replayed.
            drop(store);
            restart = Some(loss);
        }
    }
}

#[cfg(test)]
#[path = "worker/migration_tests.rs"]
mod migration_tests;
