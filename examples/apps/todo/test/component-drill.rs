//! The Todo App's real WASM component. State, wakes and the event broker are in-memory fixtures; the
//! kernel path is covered by the kernel's App tests. Build the component with the pinned SDK tools
//! (`node packages/app-sdk-tools/build-js.mjs examples/apps/todo/app.ts <out>`) and pass it as
//! `CHARIOX_TODO_COMPONENT`.
use serde_json::{json, Value};
use std::collections::BTreeMap;
use wasmtime::component::{
    Accessor, Component, FutureReader, HasSelf, Linker, ResourceTable, StreamReader,
};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

wasmtime::component::bindgen!({
    path: "../app-wit/wit",
    inline: r#"package chariox:todo-drill;
      world drill {
        import chariox:app/registry@1.0.0;
        import chariox:app/state@1.0.0;
        import chariox:app/events@1.0.0;
        export chariox:app/handlers@1.0.0;
      }"#,
    world: "drill",
    exports: { "chariox:app/handlers.register": async },
});
use chariox::app::{
    app_types::{Actor, ActorKind, AppError, InvocationContext, Occurrence, Receipt, Wake},
    events, registry, state,
};

#[derive(wasmtime::component::ComponentType, wasmtime::component::Lower)]
#[component(record)]
struct Datetime {
    seconds: u64,
    nanoseconds: u32,
}

struct Host {
    wasi: WasiCtx,
    table: ResourceTable,
    records: BTreeMap<String, state::StateRecord>,
    wakes: BTreeMap<String, Wake>,
    tools: Vec<String>,
    events: Vec<String>,
    occurrences: Vec<Occurrence>,
    /// The kernel refuses an occurrence of an automation that is not configured.
    unconfigured: bool,
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
        self.tools.push(name);
    }
    fn event(&mut self, name: String) {
        self.events.push(name);
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
        Ok(accessor.with(|mut access| access.get().records.get(&key).cloned()))
    }
    async fn transact(
        accessor: &Accessor<T, Self>,
        tx: state::Transaction,
    ) -> Result<state::Committed, AppError> {
        accessor.with(|mut access| {
            let host = access.get();
            if tx.schema_version != 0 {
                return Err(error(
                    "SCHEMA_MISMATCH",
                    "the package declares no migrations",
                ));
            }
            if !tx.occurrences.is_empty() && host.unconfigured {
                return Err(error("NOT_FOUND", "App automation was not found"));
            }
            for check in tx.checks {
                if host.records.get(&check.key).map(|record| record.version) != check.version {
                    return Err(error("CONFLICT", &check.key));
                }
            }
            for write in tx.writes {
                match write {
                    state::Write::Put((key, value)) => {
                        let version = host
                            .records
                            .get(&key)
                            .map_or(1, |record| record.version + 1);
                        host.records
                            .insert(key, state::StateRecord { value, version });
                    }
                    state::Write::Delete(key) => {
                        host.records.remove(&key);
                    }
                }
            }
            for change in tx.wakes {
                match change {
                    state::WakeChange::Set(wake) => {
                        host.wakes.insert(wake.id.clone(), wake);
                    }
                    state::WakeChange::Cancel(id) => {
                        host.wakes.remove(&id);
                    }
                }
            }
            host.occurrences.extend(tx.occurrences);
            Ok(state::Committed {
                revision: 1,
                receipts: vec![],
            })
        })
    }
}
impl events::Host for Host {
    fn occurrence_id(
        &mut self,
        source_key: String,
        occurred_at_ms: u64,
    ) -> Result<String, AppError> {
        Ok(format!("evt1.{occurred_at_ms}.{source_key}"))
    }
}
impl<T: Send> events::HostWithStore<T> for HasSelf<Host> {
    async fn emit(_: &Accessor<T, Self>, _: Occurrence) -> Result<Receipt, AppError> {
        panic!("a reminder is committed with its Todo, never emitted alone")
    }
    async fn status(_: &Accessor<T, Self>, _: String) -> Result<Receipt, AppError> {
        panic!("unexpected status")
    }
    async fn retry(_: &Accessor<T, Self>, _: String) -> Result<Receipt, AppError> {
        panic!("unexpected retry")
    }
    async fn automations(_: &Accessor<T, Self>) -> Result<Vec<events::Automation>, AppError> {
        Ok(vec![events::Automation {
            automation_id: "reminders".into(),
            event: "todo_due".into(),
            event_version: 1,
            state: "active".into(),
            last_receipt: Some(Receipt {
                receipt_id: "receipt-1".into(),
                state: "delivered".into(),
            }),
        }])
    }
    async fn subscribe(
        _: &Accessor<T, Self>,
        _: String,
    ) -> Result<StreamReader<events::LiveEvent>, AppError> {
        panic!("unexpected subscribe")
    }
}
fn error(code: &str, message: &str) -> AppError {
    AppError {
        code: code.into(),
        message: message.into(),
        retryable: None,
    }
}

async fn instance() -> wasmtime::Result<(Store<Host>, Drill)> {
    let mut config = Config::new();
    config.wasm_component_model_async(true);
    let engine = Engine::new(&config)?;
    let component = Component::from_file(&engine, std::env::var("CHARIOX_TODO_COMPONENT")?)?;
    let mut linker = Linker::<Host>::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
    wasmtime_wasi::p3::add_to_linker(&mut linker)?;
    // `Date.now()` in the component reads the preview 2 wall clock the product worker links; the trap stubs
    // below would otherwise shadow the semver-compatible definition under its exact name.
    let mut clock = linker.instance("wasi:clocks/wall-clock@0.2.0")?;
    clock.func_wrap("now", |_, (): ()| {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?;
        Ok((Datetime {
            seconds: now.as_secs(),
            nanoseconds: now.subsec_nanos(),
        },))
    })?;
    Drill::add_to_linker::<Host, HasSelf<Host>>(&mut linker, |host| host)?;
    linker.define_unknown_imports_as_traps(&component)?;
    let mut store = Store::new(
        &engine,
        Host {
            wasi: WasiCtx::builder().inherit_stderr().build(),
            table: ResourceTable::new(),
            records: BTreeMap::new(),
            wakes: BTreeMap::new(),
            tools: vec![],
            events: vec![],
            occurrences: vec![],
            unconfigured: false,
        },
    );
    let app = Drill::instantiate_async(&mut store, &component, &linker).await?;
    app.chariox_app_handlers()
        .call_register(&mut store)
        .await?
        .unwrap();
    Ok((store, app))
}

fn context(actor: ActorKind) -> InvocationContext {
    InvocationContext {
        deadline_ms: u64::MAX,
        installation_id: None,
        room_id: None,
        operation_id: None,
        agent_id: None,
        task_id: None,
        turn_id: None,
        actor: Some(Actor {
            kind: actor,
            id: "caller".into(),
        }),
    }
}

enum Call<'a> {
    Tool(&'a str, Value),
    Event(&'a str, &'a str, Value),
    Wake(Wake),
}

async fn call(
    store: &mut Store<Host>,
    app: &Drill,
    actor: ActorKind,
    call: Call<'_>,
) -> wasmtime::Result<Result<Value, AppError>> {
    let reply = store
        .run_concurrent(async |accessor| {
            let cancel = accessor.with(|mut access| {
                FutureReader::new(
                    &mut access,
                    std::future::pending::<wasmtime::Result<String>>(),
                )
            })?;
            let handlers = app.chariox_app_handlers();
            match call {
                Call::Tool(name, input) => {
                    handlers
                        .call_call_tool(
                            accessor,
                            name.into(),
                            input.to_string(),
                            context(actor),
                            cancel,
                        )
                        .await
                }
                Call::Event(name, occurrence, payload) => {
                    handlers
                        .call_deliver_event(
                            accessor,
                            name.into(),
                            occurrence.into(),
                            payload.to_string(),
                            context(actor),
                            cancel,
                        )
                        .await
                }
                Call::Wake(wake) => handlers
                    .call_deliver_wake(accessor, wake, false, context(actor), cancel)
                    .await
                    .map(|done| done.map(|()| "null".to_owned())),
            }
        })
        .await??;
    Ok(reply.map(|text| serde_json::from_str(&text).unwrap()))
}

#[tokio::test(flavor = "current_thread")]
async fn the_component_registers_serves_and_reminds() -> wasmtime::Result<()> {
    let (mut store, app) = instance().await?;
    let mut tools = store.data().tools.clone();
    tools.sort();
    assert_eq!(
        tools,
        [
            "complete_todo",
            "create_todo",
            "delete_todo",
            "list_todos",
            "reminder_status",
            "update_todo",
            "view_changes"
        ]
    );
    assert_eq!(store.data().events, ["todo_requested"]);

    let agent_todo = call(
        &mut store,
        &app,
        ActorKind::Agent,
        Call::Tool(
            "create_todo",
            json!({"title": "Agent Todo", "due_at_ms": 5000}),
        ),
    )
    .await?
    .unwrap();
    let person_todo = call(
        &mut store,
        &app,
        ActorKind::Human,
        Call::Tool("create_todo", json!({"title": "Person Todo"})),
    )
    .await?
    .unwrap();
    assert_eq!(agent_todo["created_by"], "agent");
    assert_eq!(person_todo["created_by"], "person");
    let wake_id = format!("todo-{}", agent_todo["id"].as_str().unwrap());
    let wake = store
        .data()
        .wakes
        .get(&wake_id)
        .cloned()
        .expect("a due Todo holds a wake");
    assert_eq!(
        (wake.due_at_ms, wake.revision.as_deref()),
        (5000, Some("1"))
    );

    // An agent deletes only what an agent added.
    let refused = call(
        &mut store,
        &app,
        ActorKind::Agent,
        Call::Tool("delete_todo", json!({"id": person_todo["id"]})),
    )
    .await?
    .unwrap_err();
    assert_eq!(refused.code, "FORBIDDEN");

    // The window's feed carries both creations, in order.
    let feed = call(
        &mut store,
        &app,
        ActorKind::Human,
        Call::Tool("view_changes", json!({"since": 0})),
    )
    .await?
    .unwrap();
    assert_eq!(feed["latest"], 2);
    assert_eq!(feed["dropped"], false);
    assert_eq!(feed["changes"][0]["delta"]["todo"]["title"], "Agent Todo");
    assert_eq!(feed["changes"][1]["delta"]["todo"]["title"], "Person Todo");

    // The due wake commits one todo_due occurrence with the reminded mark, once.
    for _ in 0..2 {
        call(
            &mut store,
            &app,
            ActorKind::Background,
            Call::Wake(wake.clone()),
        )
        .await?
        .unwrap();
    }
    let occurrences = &store.data().occurrences;
    assert_eq!(occurrences.len(), 1);
    assert_eq!(occurrences[0].automation_id, "reminders");
    assert_eq!(occurrences[0].occurred_at_ms, 5000);
    assert_eq!(
        occurrences[0].occurrence_id,
        format!("evt1.5000.todo:{}:1", agent_todo["id"].as_str().unwrap())
    );
    let listed = call(
        &mut store,
        &app,
        ActorKind::Human,
        Call::Tool("list_todos", json!({})),
    )
    .await?
    .unwrap();
    assert_eq!(listed["todos"][0]["reminded"], true);

    // An incoming request creates one Todo, however often it is delivered.
    for _ in 0..2 {
        call(
            &mut store,
            &app,
            ActorKind::Background,
            Call::Event("todo_requested", "occ-1", json!({"title": "From Slack"})),
        )
        .await?
        .unwrap();
    }
    let listed = call(
        &mut store,
        &app,
        ActorKind::Human,
        Call::Tool("list_todos", json!({})),
    )
    .await?
    .unwrap();
    assert_eq!(listed["todos"].as_array().unwrap().len(), 3);

    let status = call(
        &mut store,
        &app,
        ActorKind::Agent,
        Call::Tool("reminder_status", json!({})),
    )
    .await?
    .unwrap();
    assert_eq!(
        status,
        json!({"state": "active", "last_delivery": "delivered"})
    );
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn a_reminder_is_recorded_when_the_kernel_refuses_the_occurrence() -> wasmtime::Result<()> {
    let (mut store, app) = instance().await?;
    store.data_mut().unconfigured = true;
    let todo = call(
        &mut store,
        &app,
        ActorKind::Human,
        Call::Tool(
            "create_todo",
            json!({"title": "Water plants", "due_at_ms": 1000}),
        ),
    )
    .await?
    .unwrap();
    let wake = store.data().wakes[&format!("todo-{}", todo["id"].as_str().unwrap())].clone();
    call(&mut store, &app, ActorKind::Background, Call::Wake(wake))
        .await?
        .unwrap();
    assert!(store.data().occurrences.is_empty());
    let listed = call(
        &mut store,
        &app,
        ActorKind::Human,
        Call::Tool("list_todos", json!({})),
    )
    .await?
    .unwrap();
    assert_eq!(listed["todos"][0]["reminded"], true);
    Ok(())
}
