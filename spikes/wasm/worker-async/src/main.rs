//! Spike: a Chariox App worker on the async (WASI 0.3, component-model async)
//! world. ONE instance serves every call at once, so in-memory App state is
//! shared between concurrent callers, as in today's Node worker. FD3/FD4 stay
//! the existing worker IPC on the kernel's own codec.
mod host;
mod ipc;

use futures::stream::{FuturesUnordered, StreamExt};
use host::{chariox::app::types::{Actor, ActorKind, Event, InvocationContext}, App, Host, Stats, Subscriptions};
use ipc::{error, wall_ms, Inbound, Ipc};
use chariox_app_runtime::wire::RemoteError;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::fd::FromRawFd;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};
use wasmtime::component::{Accessor, Component, FutureReader, InstancePre, Linker, ResourceTable, TaskGroupHook, TaskGroupId};
use wasmtime::{CallHook, Config, Engine, Store, Trap, UpdateDeadline};

const TICK: Duration = Duration::from_millis(10);
/// After a deadline or kernel cancel, the guest gets this long to return
/// before the worker answers for it.
const CANCEL_GRACE: Duration = Duration::from_millis(250);

fn engine() -> Engine {
    let mut config = Config::new();
    config.epoch_interruption(true);
    Engine::new(&config).expect("engine")
}

/// Install-time import allowlist: the App world plus direct WASI clocks,
/// random and stdio. No usable sockets, no ambient HTTP, no filesystem here.
const ALLOWED: &[&str] = &[
    "chariox:app/", "wasi:io/", "wasi:clocks/", "wasi:random/", "wasi:filesystem/",
    "wasi:cli/stdin", "wasi:cli/stdout", "wasi:cli/stderr", "wasi:cli/environment",
    "wasi:cli/exit", "wasi:cli/terminal-",
    // CPython (componentize-py) always imports sockets. They are linked, but
    // the WasiCtx denies every socket operation (and bwrap has no network).
    "wasi:sockets/",
];

fn check(engine: &Engine, component: &Component) -> Result<(), String> {
    for (name, _) in component.component_type().imports(engine) {
        if !ALLOWED.iter().any(|prefix| name.starts_with(prefix)) {
            return Err(format!("import not allowed: {name}"));
        }
    }
    Ok(())
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("missing {name}"))
}

fn metric(value: Value) {
    eprintln!("SPIKE_METRIC {value}");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("precompile") => {
            let engine = engine();
            let bytes = std::fs::read(&args[2]).unwrap();
            let started = Instant::now();
            let compiled = engine.precompile_component(&bytes).unwrap();
            let compile_ms = started.elapsed().as_secs_f64() * 1e3;
            check(&engine, &unsafe { Component::deserialize(&engine, &compiled) }.unwrap()).unwrap();
            std::fs::write(&args[3], &compiled).unwrap();
            println!("{}", json!({ "compileMs": compile_ms, "wasmBytes": bytes.len(), "cwasmBytes": compiled.len() }));
        }
        Some("check") => {
            let engine = engine();
            let component = Component::from_file(&engine, &args[2]).unwrap();
            match check(&engine, &component) {
                Ok(()) => println!("allowed"),
                Err(message) => { println!("{message}"); std::process::exit(1) }
            }
        }
        _ => {
            // IPC keeps flowing on the second thread while a guest slice runs.
            let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
            runtime.block_on(run());
        }
    }
}

/// FD4: identity record, then wait for the supervisor's continuation before
/// loading any App code (the native launcher's ABI 1 ordering).
fn ready_continue(generation: &str, installation: &str, digest: &str) {
    let mut startup = unsafe { std::fs::File::from_raw_fd(4) };
    let mut record = b"CXAWR001".to_vec();
    for value in [generation, installation, digest] {
        record.extend_from_slice(&(value.len() as u32).to_be_bytes());
        record.extend_from_slice(value.as_bytes());
    }
    startup.write_all(&record).unwrap();
    let mut answer = [0u8; 8];
    if startup.read_exact(&mut answer).is_err() || &answer != b"CXAWGO01" {
        std::process::exit(130);
    }
}

#[derive(Clone, Copy)]
struct Limits { memory: usize, call: Duration, slice: Duration }

/// What the worker knows about one in-flight call. Whoever removes the entry
/// answers the kernel, so every call gets exactly one response.
struct Call { cancel: Option<oneshot::Sender<String>> }
type Calls = Arc<Mutex<HashMap<String, Call>>>;

struct Shared { ipc: Ipc, calls: Calls, subscriptions: Subscriptions, stats: Arc<Stats>, limits: Limits, running: Running }

/// Which call's task holds the CPU, so a trap is charged to that call only.
#[derive(Default)]
struct Attribution { starting: Option<String>, groups: HashMap<TaskGroupId, String>, current: Option<String> }
type Running = Arc<Mutex<Attribution>>;

struct Tracker(Running);

impl TaskGroupHook for Tracker {
    fn handle_start(&mut self, id: TaskGroupId) -> wasmtime::Result<()> {
        let mut running = self.0.lock().unwrap();
        if let Some(call) = running.starting.take() {
            running.groups.insert(id, call);
        }
        Ok(())
    }
    /// Kept after exit: a trap unwinds (exit, finish) before the worker
    /// asks who was running.
    fn handle_enter(&mut self, id: TaskGroupId) -> wasmtime::Result<()> {
        let mut running = self.0.lock().unwrap();
        running.current = running.groups.get(&id).cloned();
        Ok(())
    }
    fn handle_exit(&mut self, _id: TaskGroupId) -> wasmtime::Result<()> {
        Ok(())
    }
    fn handle_finish(&mut self, id: TaskGroupId) -> wasmtime::Result<()> {
        self.0.lock().unwrap().groups.remove(&id);
        Ok(())
    }
}

impl Attribution {
    fn culprit(&self) -> Option<String> {
        self.current.clone()
    }
}

fn store(engine: &Engine, shared: &Shared) -> Store<Host> {
    let mut wasi = wasmtime_wasi::WasiCtxBuilder::new();
    wasi.inherit_stderr().allow_tcp(false).allow_udp(false).allow_ip_name_lookup(false);
    let host = Host {
        ipc: shared.ipc.clone(),
        wasi: wasi.build(),
        table: ResourceTable::new(),
        limits: host::MemoryCap { bytes: shared.limits.memory, refused: false },
        subscriptions: shared.subscriptions.clone(),
        stats: shared.stats.clone(),
        slice: None,
        tools: Vec::new(),
        events: Vec::new(),
        lifecycle: Vec::new(),
    };
    let mut store = Store::new(engine, host);
    store.limiter(|host| &mut host.limits);
    *shared.running.lock().unwrap() = Attribution::default();
    store.task_group_hook(Tracker(shared.running.clone()));
    // A slice starts when the event loop hands the guest the CPU and ends when
    // the guest gives it back (await or return). Host calls made from inside
    // a slice do not reset it.
    store.call_hook(|mut store, hook| {
        match hook {
            CallHook::CallingWasm => { store.data_mut().slice.get_or_insert_with(Instant::now); }
            CallHook::ReturningFromWasm => store.data_mut().slice = None,
            _ => {}
        }
        Ok(())
    });
    store.epoch_deadline_trap();
    store.set_epoch_deadline(ticks(shared.limits.slice));
    store
}

fn ticks(budget: Duration) -> u64 {
    (budget.as_millis() / TICK.as_millis()).max(1) as u64
}

/// After `register` (a sync export): from here on a slice that keeps the CPU
/// past its budget traps, and a running slice yields every tick so timers
/// and IPC on this thread keep running.
fn time_slices(store: &mut Store<Host>, slice: Duration) {
    store.epoch_deadline_callback(move |store| {
        match store.data().slice {
            Some(started) if started.elapsed() > slice => Err(Trap::Interrupt.into()),
            _ => Ok(UpdateDeadline::Yield(1)),
        }
    });
    store.set_epoch_deadline(1);
}

fn ms(value: &str, default: u64) -> u64 {
    std::env::var(value).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

async fn run() {
    let started = Instant::now();
    let generation = env("CHARIOX_APP_GENERATION");
    ready_continue(&generation, &env("CHARIOX_APP_INSTALLATION"), &env("CHARIOX_APP_RELEASE_DIGEST"));
    let limits = Limits {
        memory: (ms("CHARIOX_WASM_MEMORY_MB", 64) as usize) << 20,
        call: Duration::from_millis(ms("CHARIOX_WASM_CALL_MS", 30_000)),
        slice: Duration::from_millis(ms("CHARIOX_WASM_SLICE_MS", 1_000)),
    };
    let stream = unsafe { std::os::unix::net::UnixStream::from_raw_fd(3) };
    stream.set_nonblocking(true).unwrap();
    let (ipc, mut requests) = Ipc::start(tokio::net::UnixStream::from_std(stream).unwrap(), generation);

    let engine = engine();
    let ticker = engine.clone();
    std::thread::spawn(move || loop { std::thread::sleep(TICK); ticker.increment_epoch(); });

    let path = format!("{}/{}", env("CHARIOX_APP_PACKAGE"), env("CHARIOX_WASM_ENTRY"));
    let load = Instant::now();
    // A .cwasm is the install-time AOT artifact, produced by this exact worker
    // build and engine configuration, and held immutable by the release store.
    let component = if path.ends_with(".cwasm") {
        unsafe { Component::deserialize_file(&engine, &path) }
    } else {
        Component::from_file(&engine, &path)
    }.unwrap_or_else(|_| std::process::exit(131));
    let load_ms = load.elapsed().as_secs_f64() * 1e3;
    let mut linker = Linker::<Host>::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker).unwrap();
    App::add_to_linker::<Host, wasmtime::component::HasSelf<Host>>(&mut linker, |host| host).unwrap();
    let pre = linker.instantiate_pre(&component).unwrap_or_else(|e| { eprintln!("link: {e:?}"); std::process::exit(131) });

    let shared = Shared { ipc: ipc.clone(), calls: Calls::default(), subscriptions: Subscriptions::default(), stats: Arc::default(), limits, running: Running::default() };
    let mut first = true;
    loop {
        let at = Instant::now();
        let (mut store, app) = match instantiate(&pre, &engine, &shared).await {
            Ok(live) => live,
            Err(e) => { eprintln!("instantiate: {e:?}"); std::process::exit(131) }
        };
        if first {
            let instantiate_ms = at.elapsed().as_secs_f64() * 1e3;
            let declared = json!({ "tools": store.data().tools, "events": store.data().events, "lifecycle": store.data().lifecycle });
            if ipc.call("worker.ready", declared).await.is_err() {
                std::process::exit(131);
            }
            metric(json!({ "loadMs": load_ms, "instantiateMs": instantiate_ms, "readyMs": started.elapsed().as_secs_f64() * 1e3 }));
            first = false;
        }
        let served = store.run_concurrent(async |accessor| serve(accessor, &app, &shared, &pre, &engine, &mut requests).await).await;
        match served {
            Ok(()) => return,
            // A trap ends the instance and every call on it: they share its
            // memory, and wasmtime cannot remove one task from a store.
            Err(trap) => {
                let refused = store.data().limits.refused;
                drop(store);
                fail_all(&shared, &trap, refused).await;
            }
        }
    }
}

async fn instantiate(pre: &InstancePre<Host>, engine: &Engine, shared: &Shared) -> wasmtime::Result<(Store<Host>, App)> {
    let mut store = store(engine, shared);
    let instance = pre.instantiate_async(&mut store).await?;
    let app = App::new(&mut store, &instance)?;
    app.chariox_app_handlers().call_register(&mut store).await?
        .map_err(|e| wasmtime::Error::msg(format!("register failed: {}", e.code)))?;
    time_slices(&mut store, shared.limits.slice);
    Ok((store, app))
}

/// Answers every call of a trapped instance. The call whose task held the
/// CPU gets the stop reason; the others were healthy and may be retried.
async fn fail_all(shared: &Shared, trap: &wasmtime::Error, refused: bool) {
    let cause = match trap.downcast_ref::<Trap>() {
        Some(Trap::Interrupt) => "APP_CALL_TIMEOUT",
        _ if refused => "APP_MEMORY_LIMIT",
        _ => "APP_TRAPPED",
    };
    let culprit = shared.running.lock().unwrap().culprit();
    let calls: Vec<(String, Call)> = shared.calls.lock().unwrap().drain().collect();
    let mut answered = Vec::new();
    for (id, _) in calls {
        let code = if Some(&id) == culprit.as_ref() { cause } else { "APP_INSTANCE_RESTARTED" };
        let mut failure = error(code, "App instance was stopped and restarted");
        failure.retryable = Some(code == "APP_INSTANCE_RESTARTED");
        answered.push(code);
        shared.ipc.respond(id, Err(failure)).await;
    }
    shared.subscriptions.lock().unwrap().clear();
    let detail: String = format!("{}", trap.root_cause()).chars().take(300).collect();
    metric(json!({ "trap": cause, "detail": detail, "culpritKnown": culprit.is_some(), "inFlightAnswered": answered }));
}

async fn serve(
    accessor: &Accessor<Host>,
    app: &App,
    shared: &Shared,
    pre: &InstancePre<Host>,
    engine: &Engine,
    requests: &mut mpsc::Receiver<Inbound>,
) {
    let mut inflight = FuturesUnordered::new();
    loop {
        tokio::select! {
            request = requests.recv() => match request {
                None => return,
                Some(Inbound::Cancel { id }) => cancel(shared, &id, "cancelled by the kernel"),
                Some(Inbound::Request { id, method, params, deadline_ms, context }) => match method.as_str() {
                    "tools.invoke" if params["name"] == "__warm" => {
                        let rounds = params["input"]["rounds"].as_u64().unwrap_or(20);
                        let samples = warm(pre, engine, shared, rounds).await;
                        shared.ipc.respond(id, Ok(json!({ "samplesMs": samples }))).await;
                    }
                    "tools.invoke" if params["name"] == "__stats" => {
                        let stats = &shared.stats;
                        let result = json!({
                            "streamsOpened": stats.streams_opened.load(Ordering::Relaxed),
                            "streamsReleased": stats.streams_released.load(Ordering::Relaxed),
                            "chunksBufferedMax": stats.chunks_buffered_max.load(Ordering::Relaxed),
                            "inFlight": shared.calls.lock().unwrap().len(),
                        });
                        shared.ipc.respond(id, Ok(result)).await;
                    }
                    "tools.invoke" => inflight.push(call_tool(accessor, app, shared, id, params, deadline_ms, context)),
                    "events.deliver" => deliver(shared, id, params),
                    "lifecycle.dispatch" => shared.ipc.respond(id, Ok(Value::Null)).await,
                    _ => shared.ipc.respond(id, Err(error("METHOD_NOT_FOUND", "unknown method"))).await,
                },
            },
            Some(()) = inflight.next(), if !inflight.is_empty() => {}
        }
    }
}

fn cancel(shared: &Shared, id: &str, reason: &str) {
    if let Some(sender) = shared.calls.lock().unwrap().get_mut(id).and_then(|call| call.cancel.take()) {
        let _ = sender.send(reason.into());
    }
}

/// The kernel's event delivery is answered once the event sits in the
/// subscriber's bounded buffer: a slow reader slows the kernel down.
fn deliver(shared: &Shared, id: String, params: Value) {
    let name = params["name"].as_str().unwrap_or_default().to_string();
    let sender = shared.subscriptions.lock().unwrap().get(&name).cloned();
    let (ipc, subscriptions) = (shared.ipc.clone(), shared.subscriptions.clone());
    tokio::spawn(async move {
        let event = Event { name: name.clone(), data: params["data"].to_string() };
        let delivered = match sender {
            Some(sender) => sender.send(event).await.is_ok(),
            None => false,
        };
        if !delivered {
            subscriptions.lock().unwrap().remove(&name);
            let mut failure = error("NO_SUBSCRIBER", "No open subscription for this event");
            failure.retryable = Some(true);
            ipc.respond(id, Err(failure)).await;
            return;
        }
        ipc.respond(id, Ok(Value::Null)).await;
    });
}

async fn call_tool(
    accessor: &Accessor<Host>,
    app: &App,
    shared: &Shared,
    id: String,
    params: Value,
    deadline_ms: u64,
    context: Option<Value>,
) {
    let budget = Duration::from_millis(deadline_ms.saturating_sub(wall_ms())).min(shared.limits.call);
    let (sender, receiver) = oneshot::channel::<String>();
    shared.calls.lock().unwrap().insert(id.clone(), Call { cancel: Some(sender) });
    // Resolves only on cancel; a finished call simply drops it.
    let signal = async move {
        match receiver.await {
            Ok(reason) => Ok::<String, wasmtime::Error>(reason),
            Err(_) => std::future::pending().await,
        }
    };
    let cancelled = match accessor.with(|mut access| FutureReader::new(&mut access, signal)) {
        Ok(cancelled) => cancelled,
        Err(_) => return finish(shared, id, Err(error("APP_UNAVAILABLE", "handle table full"))).await,
    };
    let name = params["name"].as_str().unwrap_or_default().to_string();
    let input = params["input"].to_string();
    let invocation = invocation(deadline_ms, context.as_ref());
    // The task group starts on the first poll of `call`, right below.
    shared.running.lock().unwrap().starting = Some(id.clone());
    let call = app.chariox_app_handlers().call_call_tool(accessor, name, input, invocation, cancelled);
    let deadline = async {
        tokio::time::sleep(budget).await;
        cancel(shared, &id, "deadline exceeded");
        tokio::time::sleep(CANCEL_GRACE).await;
    };
    let outcome = tokio::select! {
        called = call => match called {
            Ok(Ok(text)) => serde_json::from_str(&text).map_err(|_| error("INVALID_OUTPUT", "result is not JSON")),
            // A GC guest (QuickJS) turns a refused grow into its own
            // exception; the instance and its shared state live on.
            Ok(Err(_)) if accessor.with(|mut access| std::mem::take(&mut access.get().limits.refused)) => {
                metric(json!({ "trap": "APP_MEMORY_LIMIT", "guestHandled": true }));
                Err(error("APP_MEMORY_LIMIT", "App call exceeded the App memory limit"))
            }
            Ok(Err(app)) => Err(host::remote_error(app)),
            Err(_) => return, // the instance trapped: fail_all answers
        },
        // The guest ignored its cancel signal; its task stays parked until the
        // instance is recycled (wasmtime cannot drop one task, #11833).
        () = deadline => Err(error("APP_CALL_TIMEOUT", "App call exceeded its deadline")),
    };
    finish(shared, id, outcome).await;
}

async fn finish(shared: &Shared, id: String, outcome: Result<Value, RemoteError>) {
    if shared.calls.lock().unwrap().remove(&id).is_some() {
        shared.ipc.respond(id, outcome).await;
    }
}

/// Spike probe: fresh store + instance from the already-loaded component.
async fn warm(pre: &InstancePre<Host>, engine: &Engine, shared: &Shared, rounds: u64) -> Vec<f64> {
    let mut samples = Vec::new();
    for _ in 0..rounds {
        let at = Instant::now();
        let mut store = store(engine, shared);
        let instance = pre.instantiate_async(&mut store).await.unwrap();
        App::new(&mut store, &instance).unwrap();
        samples.push(at.elapsed().as_secs_f64() * 1e3);
    }
    samples
}

fn invocation(deadline_ms: u64, context: Option<&Value>) -> InvocationContext {
    let text = |key: &str| context.and_then(|c| c[key].as_str()).map(String::from);
    let actor = context.and_then(|c| c.get("actor")).and_then(|actor| {
        let kind = match actor["kind"].as_str()? { "human" => ActorKind::Human, "agent" => ActorKind::Agent, _ => ActorKind::Background };
        Some(Actor { kind, id: actor["id"].as_str()?.into() })
    });
    InvocationContext {
        deadline_ms,
        installation_id: text("installation_id"),
        room_id: text("room_id"),
        operation_id: text("operation_id"),
        agent_id: text("agent_id"),
        task_id: text("task_id"),
        turn_id: text("turn_id"),
        actor,
    }
}
