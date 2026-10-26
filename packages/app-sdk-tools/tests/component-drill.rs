//! Plain Wasmtime evidence for compiler ABI, endpoint ownership and dispatch.
//! This is not a kernel, broker, installer or containment acceptance drill.
use std::{
    collections::BTreeMap,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, Poll},
    time::Duration,
};

use wasmtime::component::{
    Accessor, Component, Destination, FutureReader, HasSelf, Linker, ResourceTable, StreamProducer,
    StreamReader, StreamResult, VecBuffer,
};
use wasmtime::{Config, Engine, Store, StoreContextMut};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

wasmtime::component::bindgen!({
    path: "../app-wit/wit",
    inline: r#"package chariox:sdk-drill;
      world drill {
        import chariox:app/registry@1.0.0;
        import chariox:app/state@1.0.0;
        import chariox:app/http@1.0.0;
        export chariox:app/handlers@1.0.0;
      }"#,
    world: "drill",
    exports: { "chariox:app/handlers.register": async },
});

use chariox::app::{
    app_types::{AppError, InvocationContext},
    http, registry, state,
};

#[derive(Default)]
struct Counts {
    opened: AtomicUsize,
    closed: AtomicUsize,
    cancel_closed: AtomicUsize,
    closed_when_returned: AtomicUsize,
    transactions: AtomicUsize,
}
#[derive(Default)]
struct StateGate {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
struct Lease(Arc<Counts>);
impl Lease {
    fn new(counts: Arc<Counts>) -> Self {
        counts.opened.fetch_add(1, Ordering::SeqCst);
        Self(counts)
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.0.closed.fetch_add(1, Ordering::SeqCst);
    }
}
struct CancelLease(Arc<Counts>);
impl Drop for CancelLease {
    fn drop(&mut self) {
        self.0.cancel_closed.fetch_add(1, Ordering::SeqCst);
    }
}

struct Host {
    wasi: WasiCtx,
    table: ResourceTable,
    counts: Arc<Counts>,
    records: BTreeMap<String, state::StateRecord>,
    registered: Vec<String>,
    slow_state: bool,
    state_gate: Option<Arc<StateGate>>,
    late_response: Arc<tokio::sync::Notify>,
}
impl WasiView for Host {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}
impl chariox::app::app_types::Host for Host {}
impl registry::Host for Host {
    fn tool(&mut self, name: String) {
        self.registered.push(name);
    }
    fn event(&mut self, _: String) {
        panic!("unexpected event registration");
    }
    fn lifecycle(&mut self, _: String) {
        panic!("unexpected lifecycle registration");
    }
}
impl state::Host for Host {}
impl<T: Send> state::HostWithStore<T> for HasSelf<Host> {
    async fn get(
        accessor: &Accessor<T, Self>,
        key: String,
    ) -> Result<Option<state::StateRecord>, AppError> {
        let (lease, gate) = accessor.with(|mut access| {
            let host = access.get();
            (
                (host.slow_state || host.state_gate.is_some())
                    .then(|| Lease::new(host.counts.clone())),
                host.state_gate.clone(),
            )
        });
        if let Some(gate) = gate {
            gate.entered.notify_one();
            gate.release.notified().await;
        } else if lease.is_some() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let _lease = lease;
        Ok(accessor.with(|mut access| access.get().records.get(&key).cloned()))
    }
    async fn transact(
        accessor: &Accessor<T, Self>,
        tx: state::Transaction,
    ) -> Result<state::Committed, AppError> {
        assert_eq!(tx.schema_version, 0, "template has no declared migration");
        accessor.with(|mut access| {
            let host = access.get();
            host.counts.transactions.fetch_add(1, Ordering::SeqCst);
            for check in tx.checks {
                if host.records.get(&check.key).map(|r| r.version) != check.version {
                    return Err(AppError {
                        code: "CONFLICT".into(),
                        message: check.key,
                        retryable: Some(true),
                    });
                }
            }
            for write in tx.writes {
                match write {
                    state::Write::Put((key, value)) => {
                        let version = host.records.get(&key).map_or(1, |r| r.version + 1);
                        host.records
                            .insert(key, state::StateRecord { value, version });
                    }
                    state::Write::Delete(key) => {
                        host.records.remove(&key);
                    }
                }
            }
            Ok(state::Committed {
                revision: 1,
                receipts: vec![],
            })
        })
    }
}

struct Body {
    _lease: Lease,
    sent: bool,
}
impl<D> StreamProducer<D> for Body {
    type Item = u8;
    type Buffer = VecBuffer<u8>;
    fn poll_produce<'a>(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        _: StoreContextMut<'a, D>,
        mut destination: Destination<'a, u8, Self::Buffer>,
        finish: bool,
    ) -> Poll<wasmtime::Result<StreamResult>> {
        let this = self.get_mut();
        if finish {
            return Poll::Ready(Ok(StreamResult::Cancelled));
        }
        if !this.sent {
            this.sent = true;
            destination.set_buffer(VecBuffer::from(b"hello".to_vec()));
            Poll::Ready(Ok(StreamResult::Completed))
        } else {
            Poll::Pending
        }
    }
}
impl http::Host for Host {}
impl<T: Send> http::HostWithStore<T> for HasSelf<Host> {
    async fn fetch(
        accessor: &Accessor<T, Self>,
        request: http::Request,
        _: Option<StreamReader<u8>>,
    ) -> Result<(http::ResponseHead, StreamReader<u8>), AppError> {
        let lease = Lease::new(accessor.with(|mut access| access.get().counts.clone()));
        if request.url.ends_with("/late") {
            let gate = accessor.with(|mut access| access.get().late_response.clone());
            gate.notified().await;
        } else {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let stream = accessor
            .with(|mut access| {
                StreamReader::new(
                    &mut access,
                    Body {
                        _lease: lease,
                        sent: false,
                    },
                )
            })
            .unwrap();
        Ok((
            http::ResponseHead {
                status: 200,
                headers: vec![],
                url: "https://example.com/".into(),
            },
            stream,
        ))
    }
}

fn context() -> InvocationContext {
    InvocationContext {
        deadline_ms: u64::MAX,
        installation_id: None,
        room_id: None,
        operation_id: None,
        agent_id: None,
        task_id: None,
        turn_id: None,
        actor: None,
    }
}

async fn instance(file: &str) -> wasmtime::Result<(Store<Host>, Drill, Arc<Counts>)> {
    let mut config = Config::new();
    config.wasm_component_model_async(true);
    let engine = Engine::new(&config)?;
    let component = Component::from_file(&engine, file)?;
    let mut linker = Linker::<Host>::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
    wasmtime_wasi::p3::add_to_linker(&mut linker)?;
    Drill::add_to_linker::<Host, HasSelf<Host>>(&mut linker, |host| host)?;
    linker.define_unknown_imports_as_traps(&component)?;
    let counts = Arc::new(Counts::default());
    let mut store = Store::new(
        &engine,
        Host {
            wasi: WasiCtx::builder().inherit_stderr().build(),
            table: ResourceTable::new(),
            counts: counts.clone(),
            records: BTreeMap::new(),
            registered: vec![],
            slow_state: false,
            state_gate: None,
            late_response: Arc::new(tokio::sync::Notify::new()),
        },
    );
    let app = Drill::instantiate_async(&mut store, &component, &linker).await?;
    app.chariox_app_handlers()
        .call_register(&mut store)
        .await?
        .unwrap();
    Ok((store, app, counts))
}

async fn call(
    store: &mut Store<Host>,
    app: &Drill,
    name: &str,
    input: &str,
    cancel_after: Option<Duration>,
) -> wasmtime::Result<Result<String, AppError>> {
    store
        .run_concurrent(async |accessor| {
            let counts = accessor.with(|mut access| access.get().counts.clone());
            let lease = CancelLease(counts);
            let signal = accessor.with(|mut access| {
                FutureReader::new(&mut access, async move {
                    let _lease = lease;
                    if let Some(delay) = cancel_after {
                        tokio::time::sleep(delay).await;
                        Ok::<String, wasmtime::Error>("stopped".to_string())
                    } else {
                        std::future::pending().await
                    }
                })
            });
            let signal = signal?;
            let result = app
                .chariox_app_handlers()
                .call_call_tool(
                    accessor,
                    name.to_owned(),
                    input.to_owned(),
                    context(),
                    signal,
                )
                .await;
            accessor.with(|mut access| {
                let host = access.get();
                host.counts
                    .closed_when_returned
                    .store(host.counts.closed.load(Ordering::SeqCst), Ordering::SeqCst);
                if name == "late" {
                    host.late_response.notify_one();
                }
            });
            tokio::time::sleep(Duration::from_millis(100)).await;
            result
        })
        .await?
}

#[tokio::test(flavor = "current_thread")]
async fn todo_templates_run_in_one_shared_instance() -> wasmtime::Result<()> {
    let directory = std::env::var("CHARIOX_SDK_COMPONENTS")?;
    for language in ["js", "rust"] {
        let (mut store, app, counts) = instance(&format!("{directory}/{language}-1.wasm")).await?;
        assert_eq!(store.data().registered, ["list", "add"]);
        call(&mut store, &app, "add", r#"{"title":"Todo 🍋"}"#, None)
            .await?
            .unwrap();
        let output = call(&mut store, &app, "list", "{}", None).await?.unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&output)?,
            serde_json::json!([{ "title": "Todo 🍋" }])
        );
        assert_eq!(
            counts.cancel_closed.load(Ordering::SeqCst),
            2,
            "{language}: cancel futures must be released after normal completion"
        );
    }
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn cancelled_todo_add_never_starts_a_transaction() -> wasmtime::Result<()> {
    let directory = std::env::var("CHARIOX_SDK_COMPONENTS")?;
    for language in ["js", "rust"] {
        let (mut store, app, counts) = instance(&format!("{directory}/{language}-1.wasm")).await?;
        let gate = Arc::new(StateGate::default());
        store.data_mut().state_gate = Some(gate.clone());
        let result = store
            .run_concurrent(async |accessor| {
                let entered = gate.clone();
                let cancel = accessor.with(|mut access| {
                    FutureReader::new(&mut access, async move {
                        entered.entered.notified().await;
                        Ok::<_, wasmtime::Error>("cancel pending Todo read".to_string())
                    })
                })?;
                let result = app
                    .chariox_app_handlers()
                    .call_call_tool(
                        accessor,
                        "add".into(),
                        r#"{"title":"must not be written"}"#.into(),
                        context(),
                        cancel,
                    )
                    .await?;
                assert_eq!(result.as_ref().unwrap_err().code, "CANCELLED", "{language}");
                // Release only after cancellation has returned to the caller.
                // Keep driving the instance so a detached JS continuation runs.
                gate.release.notify_one();
                tokio::time::sleep(Duration::from_millis(100)).await;
                Ok::<_, wasmtime::Error>(result)
            })
            .await??;
        assert_eq!(result.unwrap_err().code, "CANCELLED");
        assert_eq!(counts.opened.load(Ordering::SeqCst), 1, "{language}");
        assert_eq!(counts.closed.load(Ordering::SeqCst), 1, "{language}");
        assert_eq!(
            counts.transactions.load(Ordering::SeqCst),
            0,
            "{language}: cancelled add started a transaction after its read completed"
        );
        assert!(
            store.data().records.is_empty(),
            "{language}: cancelled add wrote state"
        );
    }
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn quickjs_abandoned_import_and_response_release_host_resources() -> wasmtime::Result<()> {
    let directory = std::env::var("CHARIOX_SDK_COMPONENTS")?;
    let (mut store, app, counts) = instance(&format!("{directory}/resources.wasm")).await?;
    for (name, delay) in [
        ("abandon", Some(Duration::from_millis(5))),
        ("abandon", None),
        ("consume", None),
        ("late", Some(Duration::from_millis(5))),
    ] {
        let result = call(&mut store, &app, name, "{}", delay)
            .await
            .map_err(|error| {
                error.context(format!(
                    "QuickJS resource case {name}, cancelled={}",
                    delay.is_some()
                ))
            })?;
        if delay.is_some() {
            assert_eq!(result.unwrap_err().code, "CANCELLED");
        } else {
            result.unwrap();
        }
        if name == "late" {
            assert_eq!(
                counts.closed_when_returned.load(Ordering::SeqCst) + 1,
                counts.opened.load(Ordering::SeqCst),
                "response must still be pending when cancellation returns"
            );
        }
        eprintln!(
            "resource case {name}, cancelled={}: opened={}, closed={}, closedAtReturn={}",
            delay.is_some(),
            counts.opened.load(Ordering::SeqCst),
            counts.closed.load(Ordering::SeqCst),
            counts.closed_when_returned.load(Ordering::SeqCst)
        );
        assert_eq!(
            counts.opened.load(Ordering::SeqCst),
            counts.closed.load(Ordering::SeqCst),
            "host resource leaked after {name}, cancelled={}",
            delay.is_some()
        );
    }
    let encoded = call(&mut store, &app, "encoding", "{}", None)
        .await?
        .unwrap();
    assert_eq!(encoded, "\"Todo 🍋 日本語\"");
    assert_eq!(counts.cancel_closed.load(Ordering::SeqCst), 5);
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn npm_spike_entries_run_through_the_production_sdk() -> wasmtime::Result<()> {
    let directory = std::env::var("CHARIOX_SDK_COMPONENTS")?;
    let expected: BTreeMap<String, serde_json::Value> =
        serde_json::from_slice(&std::fs::read(format!("{directory}/npm-expected.json"))?)?;
    for (name, expected) in expected {
        let (mut store, app, _) = instance(&format!("{directory}/npm-{name}.wasm")).await?;
        let output = call(&mut store, &app, "run", "{}", None).await?.unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&output)?,
            expected,
            "npm compatibility: {name}"
        );
    }
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn rust_cancellation_drops_its_pending_import() -> wasmtime::Result<()> {
    let directory = std::env::var("CHARIOX_SDK_COMPONENTS")?;
    let (mut store, app, counts) = instance(&format!("{directory}/rust-1.wasm")).await?;
    store.data_mut().slow_state = true;
    let error = call(
        &mut store,
        &app,
        "list",
        "{}",
        Some(Duration::from_millis(5)),
    )
    .await?
    .unwrap_err();
    assert_eq!(error.code, "CANCELLED");
    assert_eq!(counts.opened.load(Ordering::SeqCst), 1);
    assert_eq!(counts.closed.load(Ordering::SeqCst), 1);
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn quickjs_timer_cleanup_preserves_concurrent_invocations() -> wasmtime::Result<()> {
    let directory = std::env::var("CHARIOX_SDK_COMPONENTS")?;
    let (mut store, app, _) = instance(&format!("{directory}/resources.wasm")).await?;
    let (slow, fast) = store
        .run_concurrent(async |accessor| {
            let slow_signal = accessor.with(|mut access| {
                FutureReader::new(
                    &mut access,
                    std::future::pending::<wasmtime::Result<String>>(),
                )
            })?;
            let fast_signal = accessor.with(|mut access| {
                FutureReader::new(
                    &mut access,
                    std::future::pending::<wasmtime::Result<String>>(),
                )
            })?;
            let slow = app.chariox_app_handlers().call_call_tool(
                accessor,
                "timer".into(),
                "{}".into(),
                context(),
                slow_signal,
            );
            let fast = async {
                tokio::time::sleep(Duration::from_millis(5)).await;
                app.chariox_app_handlers()
                    .call_call_tool(
                        accessor,
                        "encoding".into(),
                        "{}".into(),
                        context(),
                        fast_signal,
                    )
                    .await
            };
            let (slow, fast) = tokio::join!(slow, fast);
            Ok::<_, wasmtime::Error>((slow?, fast?))
        })
        .await??;
    assert_eq!(slow.unwrap(), "0");
    fast.unwrap();
    assert_eq!(
        call(&mut store, &app, "timerHits", "{}", None)
            .await?
            .unwrap(),
        "0",
        "detached timer must not fire after its invocation completes"
    );
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn quickjs_randomness_is_fresh_in_separate_instances() -> wasmtime::Result<()> {
    let directory = std::env::var("CHARIOX_SDK_COMPONENTS")?;
    let mut values = vec![];
    for _ in 0..2 {
        let (mut store, app, _) = instance(&format!("{directory}/resources.wasm")).await?;
        let result = call(&mut store, &app, "random", "{}", None).await?.unwrap();
        let value: f64 = serde_json::from_str(&result)?;
        assert!((0.0..1.0).contains(&value));
        values.push(value);
    }
    assert_ne!(
        values[0], values[1],
        "snapshot must not freeze Math.random's state"
    );
    Ok(())
}
