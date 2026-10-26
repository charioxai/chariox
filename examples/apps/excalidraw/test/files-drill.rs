//! MP-08/MP-09/MP-10/MP-11: real component and WASI data directory.
//! The file broker is a fixture; this is supplementary to hosted acceptance.
use std::{
    fs,
    path::PathBuf,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};
use wasmtime::component::{
    Accessor, Component, FutureReader, HasSelf, Linker, ResourceTable, Source, StreamConsumer,
    StreamReader, StreamResult,
};
use wasmtime::{Config, Engine, Store, StoreContextMut};
use wasmtime_wasi::{FsPerms, WasiCtx, WasiCtxView, WasiView};

wasmtime::component::bindgen!({
    path: "../app-wit/wit",
    inline: r#"package chariox:excalidraw-drill;
      world drill {
        import chariox:app/registry@1.0.0;
        import chariox:app/files@1.0.0;
        import chariox:app/state@1.0.0;
        import chariox:app/host@1.0.0;
        export chariox:app/handlers@1.0.0;
      }"#,
    world: "drill",
    exports: { "chariox:app/handlers.register": async },
});
use chariox::app::{
    app_types::{AppError, InvocationContext},
    files, host, registry, state,
};
struct Host {
    wasi: WasiCtx,
    table: ResourceTable,
    data: PathBuf,
    offers: Vec<(String, Vec<u8>)>,
    export_barrier: Option<Arc<tokio::sync::Barrier>>,
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
    fn tool(&mut self, _: String) {}
    fn event(&mut self, _: String) {
        panic!("unexpected event")
    }
    fn lifecycle(&mut self, _: String) {
        panic!("unexpected lifecycle")
    }
}
impl state::Host for Host {}
impl<T: Send> state::HostWithStore<T> for HasSelf<Host> {
    async fn get(_: &Accessor<T, Self>, _: String) -> Result<Option<state::StateRecord>, AppError> {
        panic!("unexpected state read")
    }
    async fn transact(
        _: &Accessor<T, Self>,
        _: state::Transaction,
    ) -> Result<state::Committed, AppError> {
        panic!("unexpected state write")
    }
}
impl host::Host for Host {}
impl<T: Send> host::HostWithStore<T> for HasSelf<Host> {
    async fn notify(
        _: &Accessor<T, Self>,
        _: String,
        _: Option<String>,
    ) -> Result<String, AppError> {
        panic!("unexpected notification")
    }
    async fn open_link(_: &Accessor<T, Self>, _: String) -> Result<host::Offer, AppError> {
        panic!("unexpected link")
    }
    async fn write_clipboard(_: &Accessor<T, Self>, _: String) -> Result<host::Offer, AppError> {
        panic!("unexpected clipboard")
    }
    async fn pick_file(_: &Accessor<T, Self>, _: host::Pick) -> Result<Vec<String>, AppError> {
        panic!("unexpected picker")
    }
}
struct Contents(
    Arc<Mutex<Vec<u8>>>,
    Option<tokio::sync::oneshot::Sender<()>>,
);
impl Drop for Contents {
    fn drop(&mut self) {
        if let Some(done) = self.1.take() {
            let _ = done.send(());
        }
    }
}
impl<D> StreamConsumer<D> for Contents {
    type Item = u8;
    fn poll_consume(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        mut store: StoreContextMut<D>,
        mut source: Source<'_, u8>,
        finish: bool,
    ) -> Poll<wasmtime::Result<StreamResult>> {
        let mut bytes = Vec::with_capacity(4096);
        source.read(&mut store, &mut bytes)?;
        self.0.lock().unwrap().extend(bytes);
        if finish {
            if let Some(done) = self.get_mut().1.take() {
                let _ = done.send(());
            }
        }
        Poll::Ready(Ok(StreamResult::Completed))
    }
}
impl files::Host for Host {}
impl<T: Send> files::HostWithStore<T> for HasSelf<Host> {
    async fn atomic_replace(
        accessor: &Accessor<T, Self>,
        path: String,
        contents: StreamReader<u8>,
    ) -> Result<u64, AppError> {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let (done, completed) = tokio::sync::oneshot::channel();
        accessor
            .with(|mut access| contents.pipe(&mut access, Contents(bytes.clone(), Some(done))))
            .unwrap();
        completed.await.unwrap();
        let bytes = bytes.lock().unwrap();
        accessor
            .with(|mut access| fs::write(access.get().data.join(path), &*bytes))
            .unwrap();
        Ok(bytes.len() as u64)
    }
    async fn export(accessor: &Accessor<T, Self>, path: String) -> Result<String, AppError> {
        let barrier = accessor.with(|mut access| access.get().export_barrier.clone());
        if let Some(barrier) = barrier {
            barrier.wait().await;
        }
        accessor.with(|mut access| {
            let host = access.get();
            let bytes = fs::read(host.data.join(&path)).unwrap();
            host.offers.push((path, bytes));
        });
        Ok("settled-offer".into())
    }
    async fn import(
        _: &Accessor<T, Self>,
        _: String,
        _: String,
    ) -> Result<(u64, String), AppError> {
        panic!("unexpected import")
    }
    async fn snapshot(
        _: &Accessor<T, Self>,
        _: String,
        _: files::Consistency,
    ) -> Result<files::SnapshotInfo, AppError> {
        panic!("unexpected snapshot")
    }
}
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[tokio::test(flavor = "current_thread")]
async fn offered_copies_leave_no_private_files_and_named_writes_persist() -> wasmtime::Result<()> {
    let root = Scratch(
        PathBuf::from(std::env::var("TMPDIR")?)
            .join(format!("excalidraw-files-{}", std::process::id())),
    );
    fs::create_dir(&root.0)?;
    let (mut store, app) = runtime(&root.0).await?;
    let scene = r#"{"type":"excalidraw","version":2,"elements":[],"appState":{},"files":{}}"#;
    for (name, offer) in [
        ("diagram-copy-1.excalidraw", true),
        ("diagram-copy-2.excalidraw", true),
        ("diagram-copy-3.excalidraw", true),
        ("explicit.excalidraw", false),
    ] {
        let input = serde_json::json!({"name":name,"text":scene,"offer":offer}).to_string();
        store
            .run_concurrent(async |accessor| {
                let cancel = accessor.with(|mut access| {
                    FutureReader::new(
                        &mut access,
                        std::future::pending::<wasmtime::Result<String>>(),
                    )
                })?;
                let context = InvocationContext {
                    deadline_ms: u64::MAX,
                    installation_id: None,
                    room_id: None,
                    operation_id: None,
                    agent_id: None,
                    task_id: None,
                    turn_id: None,
                    actor: None,
                };
                app.chariox_app_handlers()
                    .call_call_tool(accessor, "write_file".into(), input, context, cancel)
                    .await?
                    .unwrap();
                Ok::<_, wasmtime::Error>(())
            })
            .await??;
    }
    assert_eq!(store.data().offers.len(), 3);
    for (_, bytes) in &store.data().offers {
        assert_eq!(
            bytes,
            scene.as_bytes(),
            "offers own complete bytes after staging cleanup"
        );
    }
    let mut names = fs::read_dir(&root.0)?
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(
        names,
        ["explicit.excalidraw"],
        "offered UI copies must not accumulate in private data"
    );
    assert_eq!(
        fs::read(root.0.join("explicit.excalidraw"))?,
        scene.as_bytes()
    );
    Ok(())
}

async fn runtime(root: &std::path::Path) -> wasmtime::Result<(Store<Host>, Drill)> {
    let component = std::env::var("CHARIOX_EXCALIDRAW_COMPONENT")?;
    let mut config = Config::new();
    config.wasm_component_model_async(true);
    let engine = Engine::new(&config)?;
    let component = Component::from_file(&engine, component)?;
    let mut linker = Linker::<Host>::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
    wasmtime_wasi::p3::add_to_linker(&mut linker)?;
    Drill::add_to_linker::<Host, HasSelf<Host>>(&mut linker, |host| host)?;

    let mut wasi = WasiCtx::builder();
    wasi.inherit_stderr()
        .preopened_dir(root, "data", FsPerms::ReadWrite)?;
    let mut store = Store::new(
        &engine,
        Host {
            wasi: wasi.build(),
            table: ResourceTable::new(),
            data: root.to_path_buf(),
            offers: vec![],
            export_barrier: None,
        },
    );
    let app = Drill::instantiate_async(&mut store, &component, &linker).await?;
    app.chariox_app_handlers()
        .call_register(&mut store)
        .await?
        .unwrap();
    Ok((store, app))
}

async fn invoke(
    store: &mut Store<Host>,
    app: &Drill,
    name: &str,
    input: serde_json::Value,
) -> wasmtime::Result<serde_json::Value> {
    let result = store
        .run_concurrent(async |accessor| {
            let cancel = accessor.with(|mut access| {
                FutureReader::new(
                    &mut access,
                    std::future::pending::<wasmtime::Result<String>>(),
                )
            })?;
            let context = InvocationContext {
                deadline_ms: u64::MAX,
                installation_id: None,
                room_id: None,
                operation_id: None,
                agent_id: None,
                task_id: None,
                turn_id: None,
                actor: None,
            };
            app.chariox_app_handlers()
                .call_call_tool(accessor, name.into(), input.to_string(), context, cancel)
                .await
        })
        .await??;
    Ok(serde_json::from_str(&result.unwrap()).unwrap())
}

fn scratch(label: &str) -> wasmtime::Result<Scratch> {
    let root = Scratch(
        PathBuf::from(std::env::var("TMPDIR")?)
            .join(format!("excalidraw-{label}-{}", std::process::id())),
    );
    fs::create_dir(&root.0)?;
    Ok(root)
}

#[tokio::test(flavor = "current_thread")]
async fn same_named_export_preserves_the_earlier_private_save() -> wasmtime::Result<()> {
    let root = scratch("private-export")?;
    let (mut store, app) = runtime(&root.0).await?;
    let earlier = serde_json::json!({"type":"excalidraw","version":2,"elements":[],"appState":{"name":"earlier"},"files":{}});
    let mut current = earlier.clone();
    current["appState"]["name"] = "current".into();
    invoke(
        &mut store,
        &app,
        "write_file",
        serde_json::json!({"name":"diagram.excalidraw","text":earlier.to_string(),"offer":false}),
    )
    .await?;
    invoke(
        &mut store,
        &app,
        "write_file",
        serde_json::json!({"name":"diagram.excalidraw","text":current.to_string(),"offer":true}),
    )
    .await?;
    let opened = invoke(
        &mut store,
        &app,
        "read_file",
        serde_json::json!({"name":"diagram.excalidraw"}),
    )
    .await?;
    assert_eq!(
        opened["scene"], earlier,
        "export must preserve the named save's earlier content"
    );
    assert_eq!(store.data().offers[0].1, current.to_string().as_bytes());
    assert_eq!(
        std::path::Path::new(&store.data().offers[0].0)
            .file_name()
            .unwrap(),
        "diagram.excalidraw",
        "the broker must still suggest the requested filename"
    );
    assert_eq!(
        fs::read_dir(&root.0)?.count(),
        1,
        "only the private save remains"
    );
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn concurrent_same_named_exports_have_independent_staging() -> wasmtime::Result<()> {
    let root = scratch("concurrent-export")?;
    let (mut first, first_app) = runtime(&root.0).await?;
    let (mut second, second_app) = runtime(&root.0).await?;
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    first.data_mut().export_barrier = Some(barrier.clone());
    second.data_mut().export_barrier = Some(barrier);
    let scene = |label| {
        serde_json::json!({"type":"excalidraw","version":2,"elements":[],"appState":{"name":label},"files":{}}).to_string()
    };
    let request =
        |label| serde_json::json!({"name":"diagram.excalidraw","text":scene(label),"offer":true});
    let (a, b) = tokio::join!(
        invoke(&mut first, &first_app, "write_file", request("first")),
        invoke(&mut second, &second_app, "write_file", request("second"))
    );
    a?;
    b?;
    assert_ne!(
        first.data().offers[0].0,
        second.data().offers[0].0,
        "concurrent exports must reserve different paths"
    );
    assert_eq!(first.data().offers[0].1, scene("first").as_bytes());
    assert_eq!(second.data().offers[0].1, scene("second").as_bytes());
    assert_eq!(
        fs::read_dir(&root.0)?.count(),
        0,
        "successful offers remove all staging files"
    );
    Ok(())
}
