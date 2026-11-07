//! MP-08/MP-09/MP-10/MP-11: the GitHub events App's real WASM component.
//! State and the event broker are in-memory fixtures; the kernel path is covered by
//! `app_event_path_tests`. Build the component with the pinned SDK tools and pass it
//! as `CHARIOX_GITHUB_EVENTS_COMPONENT`.
use std::{
    collections::{hash_map::DefaultHasher, BTreeMap},
    hash::{Hash, Hasher},
};
use wasmtime::component::{
    Accessor, Component, FutureReader, HasSelf, Linker, ResourceTable, StreamReader,
};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

wasmtime::component::bindgen!({
    path: "../app-wit/wit",
    inline: r#"package chariox:github-events-drill;
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
    app_types::{AppError, InvocationContext, Occurrence, Receipt},
    events, registry, state,
};

struct Host {
    wasi: WasiCtx,
    table: ResourceTable,
    records: BTreeMap<String, state::StateRecord>,
    tools: Vec<String>,
    events: Vec<String>,
    emitted: Vec<Occurrence>,
    refused: Option<String>,
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
        let mut hasher = DefaultHasher::new();
        source_key.hash(&mut hasher);
        Ok(format!("evt1.{occurred_at_ms}.{:016x}", hasher.finish()))
    }
}
impl<T: Send> events::HostWithStore<T> for HasSelf<Host> {
    async fn emit(
        accessor: &Accessor<T, Self>,
        occurrence: Occurrence,
    ) -> Result<Receipt, AppError> {
        accessor.with(|mut access| {
            let host = access.get();
            let automation = occurrence.automation_id.clone();
            host.emitted.push(occurrence);
            if host.refused.as_deref() == Some(automation.as_str()) {
                return Err(error("NOT_FOUND", "Configure the automation"));
            }
            Ok(Receipt {
                receipt_id: format!("receipt-{}", host.emitted.len()),
                state: "accepted".into(),
            })
        })
    }
    async fn status(_: &Accessor<T, Self>, _: String) -> Result<Receipt, AppError> {
        panic!("unexpected status")
    }
    async fn retry(_: &Accessor<T, Self>, _: String) -> Result<Receipt, AppError> {
        panic!("unexpected retry")
    }
    async fn automations(_: &Accessor<T, Self>) -> Result<Vec<events::Automation>, AppError> {
        panic!("unexpected automations")
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
    let component =
        Component::from_file(&engine, std::env::var("CHARIOX_GITHUB_EVENTS_COMPONENT")?)?;
    let mut linker = Linker::<Host>::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
    wasmtime_wasi::p3::add_to_linker(&mut linker)?;
    Drill::add_to_linker::<Host, HasSelf<Host>>(&mut linker, |host| host)?;
    linker.define_unknown_imports_as_traps(&component)?;
    let mut store = Store::new(
        &engine,
        Host {
            wasi: WasiCtx::builder().inherit_stderr().build(),
            table: ResourceTable::new(),
            records: BTreeMap::new(),
            tools: vec![],
            events: vec![],
            emitted: vec![],
            refused: None,
        },
    );
    let app = Drill::instantiate_async(&mut store, &component, &linker).await?;
    app.chariox_app_handlers()
        .call_register(&mut store)
        .await?
        .unwrap();
    Ok((store, app))
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

/// `deliver` is an incoming event; otherwise a tool call.
async fn call(
    store: &mut Store<Host>,
    app: &Drill,
    deliver: Option<&str>,
    name: &str,
    input: &str,
) -> wasmtime::Result<Result<String, AppError>> {
    store
        .run_concurrent(async |accessor| {
            let cancel = accessor.with(|mut access| {
                FutureReader::new(
                    &mut access,
                    std::future::pending::<wasmtime::Result<String>>(),
                )
            })?;
            let handlers = app.chariox_app_handlers();
            match deliver {
                Some(occurrence) => {
                    handlers
                        .call_deliver_event(
                            accessor,
                            name.into(),
                            occurrence.into(),
                            input.into(),
                            context(),
                            cancel,
                        )
                        .await
                }
                None => {
                    handlers
                        .call_call_tool(accessor, name.into(), input.into(), context(), cancel)
                        .await
                }
            }
        })
        .await?
}

fn delivery(event_type: &str, text: &str) -> String {
    serde_json::json!({
        "source": {
            "generator_id": "dev.chariox.github",
            "connection_id": "github-connection",
            "event_type": event_type,
            "event_type_version": 1
        },
        "occurred_at": "2026-11-06T13:00:00.000Z",
        "text": text,
        "metadata": {"repository": {"full_name": "charioxai/chariox"}, "pull_request": {"number": 42}},
        "artifacts": []
    })
    .to_string()
}

#[tokio::test(flavor = "current_thread")]
async fn one_delivery_is_emitted_to_every_configured_automation() -> wasmtime::Result<()> {
    let (mut store, app) = instance().await?;
    let host = store.data();
    let mut events = host.events.clone();
    events.sort();
    assert_eq!(
        events,
        [
            "issue_opened",
            "pull_request_opened",
            "pull_request_updated",
            "review_requested",
            "workflow_run_completed"
        ]
    );
    let mut tools = host.tools.clone();
    tools.sort();
    assert_eq!(tools, ["configure_automations", "get_automations"]);

    let payload = delivery("pull_request.synchronize", "Review charioxai/chariox #42");
    let unconfigured = call(
        &mut store,
        &app,
        Some("source-1"),
        "pull_request_updated",
        &payload,
    )
    .await?
    .unwrap_err();
    assert_eq!(unconfigured.code, "NOT_CONFIGURED");
    assert!(store.data().emitted.is_empty());

    let setting = serde_json::json!({"automations": {
        "pull_request_updated_notification": ["review-workflow", "review-agent"]
    }});
    call(
        &mut store,
        &app,
        None,
        "configure_automations",
        &setting.to_string(),
    )
    .await?
    .unwrap();
    let saved = call(&mut store, &app, None, "get_automations", "{}")
        .await?
        .unwrap();
    assert_eq!(serde_json::from_str::<serde_json::Value>(&saved)?, setting);

    call(
        &mut store,
        &app,
        Some("source-1"),
        "pull_request_updated",
        &payload,
    )
    .await?
    .unwrap();
    let emitted = &store.data().emitted;
    assert_eq!(
        emitted
            .iter()
            .map(|o| o.automation_id.as_str())
            .collect::<Vec<_>>(),
        ["review-workflow", "review-agent"]
    );
    assert_eq!(emitted[0].occurrence_id, emitted[1].occurrence_id);
    for occurrence in emitted {
        assert_eq!(occurrence.event_version, 1);
        assert_eq!(occurrence.occurred_at_ms, 1_793_970_000_000);
        assert_eq!(occurrence.invocation.prompt, "Review charioxai/chariox #42");
        assert!(occurrence.invocation.artifacts.is_empty());
        assert_eq!(occurrence.schedule_revision, None);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&occurrence.payload)?,
            serde_json::from_str::<serde_json::Value>(&payload)?
        );
    }
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn a_refused_target_does_not_stop_the_others_and_a_foreign_event_is_refused(
) -> wasmtime::Result<()> {
    let (mut store, app) = instance().await?;
    let setting = serde_json::json!({"automations": {
        "pull_request_updated_notification": ["gone", "kept"]
    }});
    call(
        &mut store,
        &app,
        None,
        "configure_automations",
        &setting.to_string(),
    )
    .await?
    .unwrap();
    store.data_mut().refused = Some("gone".into());
    let payload = delivery("pull_request.synchronize", "Review");
    let refused = call(
        &mut store,
        &app,
        Some("source-2"),
        "pull_request_updated",
        &payload,
    )
    .await?
    .unwrap_err();
    assert_eq!(refused.code, "NOT_FOUND");
    assert_eq!(
        store
            .data()
            .emitted
            .iter()
            .map(|o| o.automation_id.as_str())
            .collect::<Vec<_>>(),
        ["gone", "kept"]
    );
    let foreign = delivery("issues.opened", "Not an update");
    let wrong = call(
        &mut store,
        &app,
        Some("source-3"),
        "pull_request_updated",
        &foreign,
    )
    .await?
    .unwrap_err();
    assert_eq!(wrong.code, "INVALID_ARGUMENT");
    assert_eq!(store.data().emitted.len(), 2);
    Ok(())
}
