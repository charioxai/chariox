//! Spike: a Chariox App worker that runs a WebAssembly component on wasmtime
//! and speaks the existing worker IPC (FD3 SDK wire, FD4 Ready/Continue).
mod host;
mod ipc;

use host::{chariox::app::types::{Actor, ActorKind, InvocationContext}, App, Host};
use ipc::{error, wall_ms, Inbound, Ipc};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::os::fd::FromRawFd;
use chariox_app_runtime::wire::RemoteError;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use wasmtime::component::{Component, InstancePre, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store, Trap};

const TICK: Duration = Duration::from_millis(10);

fn engine() -> Engine {
    let mut config = Config::new();
    config.epoch_interruption(true);
    Engine::new(&config).expect("engine")
}

/// Install-time import allowlist: the App world plus direct WASI clocks,
/// random, stdio and the App's private filesystem. No sockets, no ambient HTTP.
const ALLOWED: &[&str] = &[
    "chariox:app/", "wasi:io/", "wasi:clocks/", "wasi:random/", "wasi:filesystem/",
    "wasi:cli/stdin", "wasi:cli/stdout", "wasi:cli/stderr", "wasi:cli/environment",
    "wasi:cli/exit", "wasi:cli/terminal-",
];

fn check(engine: &Engine, component: &Component) -> Result<(), String> {
    let ty = component.component_type();
    for (name, _) in ty.imports(engine) {
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
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
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
struct Limits { memory: usize, call: Duration }

fn store(engine: &Engine, ipc: &Ipc, limits: &Limits) -> Store<Host> {
    let mut wasi = wasmtime_wasi::WasiCtxBuilder::new();
    wasi.inherit_stderr().allow_tcp(false).allow_udp(false).allow_ip_name_lookup(false);
    let host = Host {
        ipc: ipc.clone(),
        wasi: wasi.build(),
        table: ResourceTable::new(),
        limits: host::MemoryCap { bytes: limits.memory, refused: false },
        tools: Vec::new(),
        events: Vec::new(),
        lifecycle: Vec::new(),
    };
    let mut store = Store::new(engine, host);
    store.limiter(|host| &mut host.limits);
    store.epoch_deadline_trap();
    store
}

struct Live { store: Store<Host>, app: App }

async fn instantiate(pre: &InstancePre<Host>, engine: &Engine, ipc: &Ipc, limits: &Limits) -> Result<Live, wasmtime::Error> {
    let mut store = store(engine, ipc, limits);
    store.set_epoch_deadline(ticks(limits.call));
    let instance = pre.instantiate_async(&mut store).await?;
    let app = App::new(&mut store, &instance)?;
    app.chariox_app_handlers().call_register(&mut store).await?
        .map_err(|e| wasmtime::Error::msg(format!("register failed: {}", e.code)))?;
    Ok(Live { store, app })
}

fn ticks(budget: Duration) -> u64 {
    (budget.as_millis() / TICK.as_millis()).max(1) as u64
}

async fn run() {
    let started = Instant::now();
    let generation = env("CHARIOX_APP_GENERATION");
    ready_continue(&generation, &env("CHARIOX_APP_INSTALLATION"), &env("CHARIOX_APP_RELEASE_DIGEST"));
    let limits = Limits {
        memory: std::env::var("CHARIOX_WASM_MEMORY_MB").ok().and_then(|v| v.parse().ok()).unwrap_or(64) << 20,
        call: Duration::from_millis(std::env::var("CHARIOX_WASM_CALL_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(30_000)),
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
    let pre = linker.instantiate_pre(&component).unwrap_or_else(|_| std::process::exit(131));

    let first = Instant::now();
    let live = instantiate(&pre, &engine, &ipc, &limits).await.unwrap_or_else(|_| std::process::exit(131));
    let instantiate_ms = first.elapsed().as_secs_f64() * 1e3;
    let declared = json!({ "tools": live.store.data().tools, "events": live.store.data().events, "lifecycle": live.store.data().lifecycle });
    if ipc.call("worker.ready", declared).await.is_err() {
        std::process::exit(131);
    }
    metric(json!({ "loadMs": load_ms, "instantiateMs": instantiate_ms, "readyMs": started.elapsed().as_secs_f64() * 1e3 }));

    // One instance runs one call at a time (the 1.x world is sync).
    // CHARIOX_WASM_POOL=N lets N calls run on separate instances.
    let size = std::env::var("CHARIOX_WASM_POOL").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
    let pool = Arc::new(Pool { pre, engine, ipc, limits, idle: Mutex::new(vec![live]), slots: Semaphore::new(size) });
    while let Some(request) = requests.recv().await {
        let pool = pool.clone();
        tokio::spawn(async move { pool.serve(request).await });
    }
}

struct Pool {
    pre: InstancePre<Host>,
    engine: Engine,
    ipc: Ipc,
    limits: Limits,
    idle: Mutex<Vec<Live>>,
    slots: Semaphore,
}

impl Pool {
    async fn serve(&self, request: Inbound) {
        let Inbound { id, method, params, deadline_ms, context } = request;
        let outcome = match method.as_str() {
            "tools.invoke" if params["name"] == "__warm" => {
                let rounds = params["input"]["rounds"].as_u64().unwrap_or(20);
                Ok(json!({ "samplesMs": warm(&self.pre, &self.engine, &self.ipc, &self.limits, rounds).await }))
            }
            "tools.invoke" => self.call_tool(&params, deadline_ms, context.as_ref()).await,
            "lifecycle.dispatch" => Ok(Value::Null),
            _ => Err(error("METHOD_NOT_FOUND", "unknown method")),
        };
        self.ipc.respond(id, outcome).await;
    }

    async fn call_tool(&self, params: &Value, deadline_ms: u64, context: Option<&Value>) -> Result<Value, RemoteError> {
        let _slot = self.slots.acquire().await.map_err(|_| error("APP_UNAVAILABLE", "worker stopping"))?;
        let idle = self.idle.lock().unwrap().pop();
        let mut live = match idle {
            Some(live) => live,
            None => instantiate(&self.pre, &self.engine, &self.ipc, &self.limits).await
                .map_err(|_| error("APP_TRAPPED", "App instance could not start"))?,
        };
        let budget = Duration::from_millis(deadline_ms.saturating_sub(wall_ms())).min(self.limits.call);
        live.store.set_epoch_deadline(ticks(budget));
        live.store.data_mut().limits.refused = false;
        let name = params["name"].as_str().unwrap_or_default();
        let input = params["input"].to_string();
        let called = live.app.chariox_app_handlers().call_call_tool(&mut live.store, name, &input, &invocation(deadline_ms, context)).await;
        match called {
            Ok(result) => {
                self.idle.lock().unwrap().push(live);
                match result {
                    Ok(text) => serde_json::from_str(&text).map_err(|_| error("INVALID_OUTPUT", "result is not JSON")),
                    Err(app) => Err(host::remote_error(app)),
                }
            }
            // A trapped instance is dropped, never re-entered; the next call
            // gets a fresh one.
            Err(trap) => {
                let code = match trap.downcast_ref::<Trap>() {
                    Some(Trap::Interrupt) => "APP_CALL_TIMEOUT",
                    _ if live.store.data().limits.refused => "APP_MEMORY_LIMIT",
                    _ => "APP_TRAPPED",
                };
                metric(json!({ "trap": code }));
                Err(error(code, "App code was stopped; the App was restarted"))
            }
        }
    }
}

/// Spike probe: fresh store + instance from the already-loaded component.
async fn warm(pre: &InstancePre<Host>, engine: &Engine, ipc: &Ipc, limits: &Limits, rounds: u64) -> Vec<f64> {
    let mut samples = Vec::new();
    for _ in 0..rounds {
        let at = Instant::now();
        let mut store = store(engine, ipc, limits);
        store.set_epoch_deadline(ticks(limits.call));
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
