//! An instance is replaced as a unit: Wasmtime cannot safely detach a single
//! abandoned guest task from its shared memory and pending imports.
use crate::{
    attribution::Attribution,
    bindings::Worker,
    host::{Host, Registration},
    ipc::{error, Ipc},
    limits::{Limits, Memory, MemoryLimit, Stop},
    scope::Scope,
    streams::Subscriptions,
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
    Engine, Store, UpdateDeadline,
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
    #[cfg(target_os = "macos")]
    code: Arc<crate::macos_code::JitCode>,
    stop: Arc<AtomicBool>,
    ticker: Option<std::thread::JoinHandle<()>>,
}
impl WorkerEngine {
    pub fn new() -> wasmtime::Result<Self> {
        #[cfg(not(target_os = "macos"))]
        let engine = crate::engine::engine()?;
        #[cfg(target_os = "macos")]
        let (engine, code) = crate::macos_code::engine()?;
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
            #[cfg(target_os = "macos")]
            code,
            stop,
            ticker: Some(ticker),
        })
    }
    pub fn engine(&self) -> &Engine {
        &self.engine
    }
    pub fn compile(&self, bytes: &[u8], world: &str) -> wasmtime::Result<Component> {
        chariox_app_wit::check_component(bytes, world)?;
        #[cfg(target_os = "macos")]
        {
            let compiled = self.engine.precompile_component(bytes)?;
            // The bytes were just produced by this engine, never supplied as
            // native code by an App package.
            return unsafe { self.code.component(&self.engine, &compiled) };
        }
        #[cfg(not(target_os = "macos"))]
        Component::new(&self.engine, bytes)
    }
    /// Load a module produced by this exact engine/configuration for native probes.
    ///
    /// # Safety
    /// The caller must authenticate the artifact's compiler/configuration and
    /// integrity. App package bytes must never enter this native-code API.
    #[cfg(feature = "test-support")]
    pub unsafe fn deserialize_module(&self, bytes: &[u8]) -> wasmtime::Result<wasmtime::Module> {
        #[cfg(target_os = "macos")]
        return unsafe { self.code.module(&self.engine, bytes) };
        #[cfg(not(target_os = "macos"))]
        unsafe {
            wasmtime::Module::deserialize(&self.engine, bytes)
        }
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
}
impl AppWorker {
    pub fn new(
        engine: WorkerEngine,
        component: Component,
        limits: Limits,
        retry: RetryPolicy,
    ) -> wasmtime::Result<Self> {
        limits.validate()?;
        let mut linker = Linker::new(engine.engine());
        Worker::add_to_linker::<Host, HasSelf<Host>>(&mut linker, |host| host)?;
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
        })
    }
    /// R6 supplies its broker-backed WASI bindings through this same linker.
    pub fn linker(engine: &Engine) -> wasmtime::Result<Linker<Host>> {
        let mut linker = Linker::new(engine);
        Worker::add_to_linker::<Host, HasSelf<Host>>(&mut linker, |host| host)?;
        Ok(linker)
    }
    async fn instantiate(&self, ipc: Ipc) -> wasmtime::Result<(Store<Host>, Worker, Attribution)> {
        let attribution = Attribution::default();
        let host = Host {
            ipc,
            attribution: attribution.clone(),
            memory: Memory::new(self.limits.memory_bytes),
            registration: Registration::default(),
            sealed: false,
            subscriptions: Subscriptions::default(),
            stream_slots: Arc::new(tokio::sync::Semaphore::new(128)),
        };
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
        let slice = Duration::from_millis(self.limits.slice_ms);
        store.epoch_deadline_callback(move |store| {
            let attribution = &store.data().attribution;
            if let Some(scope) = attribution.current() {
                if let Some(reason) = scope
                    .reason()
                    .filter(|reason| *reason != "APP_CALL_FINISHED")
                {
                    return Err(Stop(reason).into());
                }
                if tokio::time::Instant::now() >= scope.deadline() {
                    return Err(Stop("DEADLINE_EXCEEDED").into());
                }
            }
            if attribution
                .slice_started()
                .is_some_and(|started| started.elapsed() >= slice)
            {
                return Err(Stop("APP_CPU_SLICE").into());
            }
            Ok(UpdateDeadline::Yield(1))
        });
        store.set_epoch_deadline(1);
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
            match &registration {
                None => {
                    let scope = Scope::new(
                        "worker-ready".into(),
                        tokio::time::Instant::now() + Duration::from_secs(30),
                    );
                    ipc.call(
                        &scope,
                        "worker.ready",
                        serde_json::to_value(&store.data().registration)?,
                    )
                    .await
                    .map_err(|error| {
                        wasmtime::Error::msg(format!("{}: {}", error.code, error.message))
                    })?;
                    registration = Some(store.data().registration.clone());
                }
                Some(expected) => wasmtime::ensure!(
                    expected == &store.data().registration,
                    "App registration changed after instance restart"
                ),
            }
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
                    serve(
                        accessor,
                        &worker,
                        &ipc,
                        incoming,
                        &mut queued,
                        &mut accepted,
                        self.limits,
                    )
                    .await
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
