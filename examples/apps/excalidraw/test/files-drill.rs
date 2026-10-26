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
    let component = std::env::var("CHARIOX_EXCALIDRAW_COMPONENT")?;
    let root = Scratch(
        PathBuf::from(std::env::var("TMPDIR")?)
            .join(format!("excalidraw-files-{}", std::process::id())),
    );
    fs::create_dir(&root.0)?;
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
        .preopened_dir(&root.0, "data", FsPerms::ReadWrite)?;
    let mut store = Store::new(
        &engine,
        Host {
            wasi: wasi.build(),
            table: ResourceTable::new(),
            data: root.0.clone(),
            offers: vec![],
        },
    );
    let app = Drill::instantiate_async(&mut store, &component, &linker).await?;
    app.chariox_app_handlers()
        .call_register(&mut store)
        .await?
        .unwrap();
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
