//! MP-08/MP-09/MP-10/MP-11: real component and WASI data directory.
//! The file broker is a fixture; this is supplementary to hosted acceptance.
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    task::{Context, Poll},
};
use wasmtime::component::{
    Accessor, Component, FutureReader, HasSelf, Linker, ResourceTable, Source, StreamConsumer,
    StreamReader, StreamResult,
};
use wasmtime::{Config, Engine, Store, StoreContextMut};
use wasmtime_wasi::{FsPerms, WasiCtx, WasiCtxView, WasiView};

#[path = "../../../../packages/app-sdk-tools/tests/support/catalog_path.rs"]
mod catalog_path;

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
use chariox::app1_0_0::{
    app_types::{AppError, InvocationContext},
    files, host, registry, state,
};
struct Host {
    wasi: WasiCtx,
    table: ResourceTable,
    data: PathBuf,
    offers: Vec<(String, Vec<u8>)>,
    export_barrier: Option<Arc<tokio::sync::Barrier>>,
    fault: Option<(FileStage, bool, Arc<tokio::sync::Notify>)>,
    /// The App's state, with the kernel's versions: every key a transaction writes takes its revision.
    records: BTreeMap<String, state::StateRecord>,
    revision: u64,
    /// A scene that another window commits right after the next transaction, before any later read.
    rival: Option<String>,
    /// The person's choice of a diagram: how many asks still find it unanswered, and how many requests were made.
    unanswered: u32,
    requests: u32,
}
impl Host {
    fn commit(&mut self, tx: state::Transaction) -> Result<state::Committed, AppError> {
        for check in &tx.checks {
            if self.records.get(&check.key).map(|r| r.version) != check.version {
                return Err(AppError {
                    code: "CONFLICT".into(),
                    message: check.key.clone(),
                    retryable: None,
                });
            }
        }
        self.revision += 1;
        for write in tx.writes {
            match write {
                state::Write::Put((key, value)) => {
                    let version = self.revision;
                    self.records
                        .insert(key, state::StateRecord { value, version });
                }
                state::Write::Delete(key) => {
                    self.records.remove(&key);
                }
            }
        }
        let committed = state::Committed {
            revision: self.revision,
            receipts: vec![],
        };
        if let Some(scene) = self.rival.take() {
            self.revision += 1;
            let version = self.revision;
            self.records.insert(
                "scene".into(),
                state::StateRecord {
                    value: scene,
                    version,
                },
            );
        }
        Ok(committed)
    }
}
impl WasiView for Host {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}
impl chariox::app1_0_0::app_types::Host for Host {}
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
        accessor.with(|mut access| access.get().commit(tx))
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
        panic!("a call that waits for the person cannot outlast the kernel's 30 s")
    }
    async fn request_file(
        accessor: &Accessor<T, Self>,
        request: host::Pick,
    ) -> Result<host::Picked, AppError> {
        assert!(!request.multiple && request.accept.contains(&".excalidraw".to_owned()));
        Ok(accessor.with(|mut access| {
            access.get().requests += 1;
            access.get().choice()
        }))
    }
    async fn await_file(
        accessor: &Accessor<T, Self>,
        operation_id: String,
        wait_ms: u32,
    ) -> Result<host::Picked, AppError> {
        assert_eq!(operation_id, "pick-1");
        assert!(
            wait_ms <= 25_000,
            "a call waits less than the kernel's limit"
        );
        Ok(accessor.with(|mut access| access.get().choice()))
    }
}
impl Host {
    /// Where the person's choice stands after one more ask.
    fn choice(&mut self) -> host::Picked {
        if self.unanswered == 0 {
            return host::Picked::Granted(vec!["grant-1".into()]);
        }
        self.unanswered -= 1;
        host::Picked::Pending("pick-1".into())
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
#[derive(Clone, Copy, PartialEq)]
enum FileStage {
    Replace,
    Export,
}
async fn file_fault<T: Send>(
    accessor: &Accessor<T, HasSelf<Host>>,
    stage: FileStage,
) -> Result<(), AppError> {
    let fault = accessor.with(|mut access| access.get().fault.clone());
    if let Some((at, cancel, reached)) = fault {
        if at == stage {
            if cancel {
                reached.notify_one();
                return std::future::pending().await;
            }
            return Err(AppError {
                code: "FILE_TEST_FAILURE".into(),
                message: "file broker refused".into(),
                retryable: None,
            });
        }
    }
    Ok(())
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
        file_fault(accessor, FileStage::Replace).await?;
        let bytes = bytes.lock().unwrap();
        accessor
            .with(|mut access| fs::write(access.get().data.join(path), &*bytes))
            .unwrap();
        Ok(bytes.len() as u64)
    }
    async fn export(accessor: &Accessor<T, Self>, path: String) -> Result<String, AppError> {
        file_fault(accessor, FileStage::Export).await?;
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

/// A directory only this test uses: tests of one process run in parallel, so
/// the process id alone does not keep two scratch roots apart.
fn scratch(label: &str) -> wasmtime::Result<Scratch> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = Scratch(std::env::temp_dir().join(format!(
        "excalidraw-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    fs::create_dir(&root.0)?;
    Ok(root)
}

#[tokio::test(flavor = "current_thread")]
async fn offered_copies_leave_no_private_files_and_named_writes_persist() -> wasmtime::Result<()> {
    let root = scratch("files")?;
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
                app.chariox_app1_0_0_handlers()
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
            fault: None,
            records: BTreeMap::new(),
            revision: 0,
            rival: None,
            unanswered: 0,
            requests: 0,
        },
    );
    let app = Drill::instantiate_async(&mut store, &component, &linker).await?;
    app.chariox_app1_0_0_handlers()
        .call_register(&mut store)
        .await?
        .unwrap();
    Ok((store, app))
}

/// The App's answer to a tool call: the result text, or the error it refused with.
async fn try_invoke(
    store: &mut Store<Host>,
    app: &Drill,
    name: &str,
    input: serde_json::Value,
) -> wasmtime::Result<Result<String, AppError>> {
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
            app.chariox_app1_0_0_handlers()
                .call_call_tool(accessor, name.into(), input.to_string(), context, cancel)
                .await
        })
        .await?
}

async fn invoke(
    store: &mut Store<Host>,
    app: &Drill,
    name: &str,
    input: serde_json::Value,
) -> wasmtime::Result<serde_json::Value> {
    let result = try_invoke(store, app, name, input).await?;
    Ok(serde_json::from_str(&result.unwrap()).unwrap())
}

/// The kept drafts as a window recovers them: the list, then one read for each, oldest first.
async fn kept_drafts(
    store: &mut Store<Host>,
    app: &Drill,
) -> wasmtime::Result<Vec<serde_json::Value>> {
    let listed = invoke(store, app, "list_drafts", serde_json::json!({})).await?;
    let mut drafts = Vec::new();
    for entry in listed["drafts"].as_array().unwrap() {
        let read = invoke(
            store,
            app,
            "read_draft",
            serde_json::json!({"id": entry["id"]}),
        )
        .await?;
        assert_eq!(read["revision"], entry["revision"]);
        drafts.push(read);
    }
    Ok(drafts)
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

async fn interrupted_export(stage: FileStage, cancel: bool) -> wasmtime::Result<()> {
    let root = scratch(if stage == FileStage::Replace {
        "replace-interrupted"
    } else {
        "export-interrupted"
    })?;
    let (mut store, app) = runtime(&root.0).await?;
    let reached = Arc::new(tokio::sync::Notify::new());
    store.data_mut().fault = Some((stage, cancel, reached.clone()));
    let scene =
        serde_json::json!({"type":"excalidraw","version":2,"elements":[],"appState":{},"files":{}});
    let input = serde_json::json!({"name":"copy.excalidraw","text":scene.to_string(),"offer":true})
        .to_string();
    let result = store
        .run_concurrent(async |accessor| {
            let cancel = accessor.with(|mut access| {
                FutureReader::new(&mut access, async move {
                    if cancel {
                        reached.notified().await;
                        Ok("cancel requested".to_string())
                    } else {
                        std::future::pending::<wasmtime::Result<String>>().await
                    }
                })
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
            app.chariox_app1_0_0_handlers()
                .call_call_tool(accessor, "write_file".into(), input, context, cancel)
                .await
        })
        .await??;
    assert_eq!(
        result.unwrap_err().code,
        if cancel {
            "CANCELLED"
        } else {
            "FILE_TEST_FAILURE"
        }
    );
    assert_eq!(
        fs::read_dir(&root.0)?.count(),
        0,
        "an interrupted export must remove its private staging directory and file"
    );
    assert!(store.data().offers.is_empty());
    Ok(())
}
#[tokio::test(flavor = "current_thread")]
async fn failed_replacement_releases_export_staging() -> wasmtime::Result<()> {
    interrupted_export(FileStage::Replace, false).await
}
#[tokio::test(flavor = "current_thread")]
async fn failed_offer_releases_export_staging() -> wasmtime::Result<()> {
    interrupted_export(FileStage::Export, false).await
}
#[tokio::test(flavor = "current_thread")]
async fn cancelled_replacement_releases_export_staging() -> wasmtime::Result<()> {
    interrupted_export(FileStage::Replace, true).await
}
#[tokio::test(flavor = "current_thread")]
async fn cancelled_offer_releases_export_staging() -> wasmtime::Result<()> {
    interrupted_export(FileStage::Export, true).await
}

#[tokio::test(flavor = "current_thread")]
async fn a_save_returns_its_own_revision_when_another_window_commits_right_after(
) -> wasmtime::Result<()> {
    let root = scratch("own-revision")?;
    let (mut store, app) = runtime(&root.0).await?;
    let mine =
        serde_json::json!({"type":"excalidraw","version":2,"elements":[],"appState":{},"files":{}});
    let theirs = r#"{"type":"excalidraw","version":2,"elements":[{"id":"theirs"}],"appState":{},"files":{}}"#;
    store.data_mut().rival = Some(theirs.into());
    let saved = invoke(
        &mut store,
        &app,
        "save_scene",
        serde_json::json!({"scene": mine, "version": null}),
    )
    .await?;
    // The version this window holds is the one its own transaction committed, not the rival's.
    assert_eq!(saved["version"], "1");
    let loaded = invoke(&mut store, &app, "load_scene", serde_json::json!({})).await?;
    assert_eq!(
        loaded["version"], "2",
        "the rival's commit is a later version"
    );
    assert_ne!(
        saved["version"], loaded["version"],
        "the window's next save checks against the version it holds, so the rival's commit is not overwritten"
    );
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn a_person_who_takes_longer_than_one_call_still_picks_with_the_same_request(
) -> wasmtime::Result<()> {
    let root = scratch("slow-pick")?;
    let (mut store, app) = runtime(&root.0).await?;
    // The call that asks finds the choice unmade twice (the request, then its wait), and so does the next one's wait.
    store.data_mut().unanswered = 3;
    let first = invoke(&mut store, &app, "pick_file", serde_json::json!({})).await?;
    assert_eq!(
        first,
        serde_json::json!({"pending": true, "pickId": "pick-1"})
    );
    let second = invoke(
        &mut store,
        &app,
        "pick_file",
        serde_json::json!({"pickId": "pick-1"}),
    )
    .await?;
    assert_eq!(
        second,
        serde_json::json!({"pending": true, "pickId": "pick-1"})
    );
    let third = invoke(
        &mut store,
        &app,
        "pick_file",
        serde_json::json!({"pickId": "pick-1"}),
    )
    .await?;
    assert_eq!(third, serde_json::json!({"grants": ["grant-1"]}));
    assert_eq!(
        store.data().requests,
        1,
        "asking again never opens a second request"
    );
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn each_window_keeps_its_own_draft_in_the_data_directory_and_discards_only_that_one(
) -> wasmtime::Result<()> {
    let root = scratch("drafts")?;
    let (mut store, app) = runtime(&root.0).await?;
    let scene = |id: &str| serde_json::json!({"type":"excalidraw","version":2,"elements":[{"id": id}],"appState":{},"files":{}});
    for (window, element) in [("window-1", "left"), ("window-2", "right")] {
        invoke(
            &mut store,
            &app,
            "save_draft",
            serde_json::json!({"id": window, "scene": scene(element)}),
        )
        .await?;
    }
    // A later write of a window replaces its own draft only.
    invoke(
        &mut store,
        &app,
        "save_draft",
        serde_json::json!({"id": "window-1", "scene": scene("left-again")}),
    )
    .await?;
    let read = kept_drafts(&mut store, &app).await?;
    let kept: Vec<(&str, &str)> = read
        .iter()
        .map(|draft| {
            (
                draft["id"].as_str().unwrap(),
                draft["scene"]["elements"][0]["id"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(kept.len(), 2);
    assert!(kept.contains(&("window-1", "left-again")));
    assert!(kept.contains(&("window-2", "right")));

    invoke(
        &mut store,
        &app,
        "discard_draft",
        serde_json::json!({"id": "window-1"}),
    )
    .await?;
    let read = kept_drafts(&mut store, &app).await?;
    assert_eq!(read.len(), 1, "the other window's draft stays");
    assert_eq!(read[0]["id"], "window-2");

    // An id names a file in the drafts folder and nothing else.
    for bad in ["../scene", "a/b", ""] {
        let refused = try_invoke(
            &mut store,
            &app,
            "save_draft",
            serde_json::json!({"id": bad, "scene": scene("x")}),
        )
        .await?;
        assert!(refused.is_err(), "{bad:?} must be refused");
    }
    assert_eq!(
        fs::read_dir(root.0.join("drafts"))?.count(),
        1,
        "only window-2's draft is on disk"
    );
    Ok(())
}

/// The review's second repro, at the App: a draft that a recovering window read and that its window then
/// advanced is still the newer one afterwards, and nothing a window sends removes another window's draft.
#[tokio::test(flavor = "current_thread")]
async fn a_window_recovering_drafts_cannot_remove_the_newer_draft_of_another_window(
) -> wasmtime::Result<()> {
    let root = scratch("drafts-copy")?;
    let (mut store, app) = runtime(&root.0).await?;
    let scene = |ids: &[&str]| {
        serde_json::json!({"type":"excalidraw","version":2,
            "elements": ids.iter().map(|id| serde_json::json!({"id": id})).collect::<Vec<_>>(),
            "appState":{},"files":{}})
    };
    invoke(
        &mut store,
        &app,
        "save_draft",
        serde_json::json!({"id": "open", "scene": scene(&["one"])}),
    )
    .await?;
    // The recovering window reads, then the open window keeps a second element.
    let read = kept_drafts(&mut store, &app).await?;
    assert_eq!(read[0]["scene"]["elements"].as_array().unwrap().len(), 1);
    invoke(
        &mut store,
        &app,
        "save_draft",
        serde_json::json!({"id": "open", "scene": scene(&["one", "two"])}),
    )
    .await?;
    // It writes its own draft and discards its own; every call it makes names its own id.
    invoke(
        &mut store,
        &app,
        "save_draft",
        serde_json::json!({"id": "recovering", "scene": scene(&["one"])}),
    )
    .await?;
    invoke(
        &mut store,
        &app,
        "discard_draft",
        serde_json::json!({"id": "recovering"}),
    )
    .await?;
    let kept = kept_drafts(&mut store, &app).await?;
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0]["id"], "open");
    assert_eq!(kept[0]["scene"]["elements"].as_array().unwrap().len(), 2);
    Ok(())
}

/// Review P2: there is no shared cap. However many other windows keep a draft, a window's write is
/// kept, replaces only its own file, and no other window's draft is removed to make room.
#[tokio::test(flavor = "current_thread")]
async fn any_number_of_windows_keep_a_draft_and_none_is_removed_to_make_room(
) -> wasmtime::Result<()> {
    let root = scratch("drafts-many")?;
    let (mut store, app) = runtime(&root.0).await?;
    let mut catalog = catalog_path::Catalog::new(include_bytes!("../schemas/tools.json"));
    let scene = |id: &str| serde_json::json!({"type":"excalidraw","version":2,"elements":[{"id": id}],"appState":{"viewBackgroundColor":"#ffffff"},"files":{}});
    for n in 0..9 {
        let input =
            serde_json::json!({"id": format!("window-{n}"), "scene": scene(&format!("e{n}"))});
        let result = invoke(&mut store, &app, "save_draft", input.clone()).await?;
        catalog.call("save_draft", &input, &result).unwrap();
    }
    let read = kept_drafts(&mut store, &app).await?;
    let ids: std::collections::BTreeSet<&str> = read
        .iter()
        .map(|draft| draft["id"].as_str().unwrap())
        .collect();
    let windows: Vec<String> = (0..9).map(|n| format!("window-{n}")).collect();
    assert_eq!(
        ids,
        windows.iter().map(String::as_str).collect(),
        "every window's draft is kept"
    );
    assert_eq!(fs::read_dir(root.0.join("drafts"))?.count(), 9);
    Ok(())
}

/// A scene of `kib` KiB: one element that carries that much data.
fn big_scene(id: &str, kib: usize) -> serde_json::Value {
    serde_json::json!({"type":"excalidraw","version":2,
        "elements":[{"id": id, "customData": {"pad": "x".repeat(kib * 1024)}}],
        "appState":{},"files":{}})
}

/// The review's first finding: three drafts of 180 KiB are 540 KiB, over the catalog's 512 KiB call
/// limit, so a reply with all of them was rejected before the window could merge any edit. Each reply
/// now carries one scene, and every call of the recovery passes the catalog's own validation.
#[tokio::test(flavor = "current_thread")]
async fn near_limit_drafts_are_recovered_one_at_a_time_through_the_catalog() -> wasmtime::Result<()>
{
    let root = scratch("drafts-budget")?;
    let (mut store, app) = runtime(&root.0).await?;
    let mut catalog = catalog_path::Catalog::new(include_bytes!("../schemas/tools.json"));
    for n in 0..3 {
        let input = serde_json::json!({"id": format!("window-{n}"), "scene": big_scene(&format!("e{n}"), 180)});
        let result = invoke(&mut store, &app, "save_draft", input.clone()).await?;
        catalog.call("save_draft", &input, &result).unwrap();
    }
    let listed = invoke(&mut store, &app, "list_drafts", serde_json::json!({})).await?;
    catalog
        .call("list_drafts", &serde_json::json!({}), &listed)
        .unwrap();
    let ids: Vec<&str> = listed["drafts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|draft| draft["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 3);
    let mut all = Vec::new();
    for id in ids {
        let input = serde_json::json!({"id": id});
        let read = invoke(&mut store, &app, "read_draft", input.clone()).await?;
        assert_eq!(read["scene"]["elements"].as_array().unwrap().len(), 1);
        catalog.call("read_draft", &input, &read).unwrap();
        all.push(read);
    }
    // The premise: the three together do not fit one reply.
    let together = serde_json::json!({"drafts": all});
    assert!(matches!(
        catalog.call(
            "read_draft",
            &serde_json::json!({"id": "window-0"}),
            &together
        ),
        Err(chariox_app_runtime::app_catalog::CatalogError::Limit)
    ));
    Ok(())
}

/// A draft that its window discarded between the list and the read has no scene to hand over.
#[tokio::test(flavor = "current_thread")]
async fn a_draft_discarded_after_the_list_reads_as_no_scene() -> wasmtime::Result<()> {
    let root = scratch("drafts-gone")?;
    let (mut store, app) = runtime(&root.0).await?;
    invoke(
        &mut store,
        &app,
        "save_draft",
        serde_json::json!({"id": "window-1", "scene": big_scene("e", 1)}),
    )
    .await?;
    invoke(
        &mut store,
        &app,
        "discard_draft",
        serde_json::json!({"id": "window-1"}),
    )
    .await?;
    let read = invoke(
        &mut store,
        &app,
        "read_draft",
        serde_json::json!({"id": "window-1"}),
    )
    .await?;
    assert_eq!(read["scene"], serde_json::Value::Null);
    Ok(())
}

/// Listing does not lease a draft: its age must still be checked by the individual read.
#[tokio::test(flavor = "current_thread")]
async fn a_draft_expired_after_listing_has_no_scene_and_is_removed() -> wasmtime::Result<()> {
    let root = scratch("drafts-expired")?;
    let (mut store, app) = runtime(&root.0).await?;
    let mut catalog = catalog_path::Catalog::new(include_bytes!("../schemas/tools.json"));
    let input = serde_json::json!({"id": "window-1", "scene": big_scene("e", 1)});
    let saved = invoke(&mut store, &app, "save_draft", input.clone()).await?;
    catalog.call("save_draft", &input, &saved).unwrap();
    let listed = invoke(&mut store, &app, "list_drafts", serde_json::json!({})).await?;
    catalog
        .call("list_drafts", &serde_json::json!({}), &listed)
        .unwrap();
    assert_eq!(listed["drafts"][0]["id"], "window-1");

    // Advance the retained file's age instead of waiting an hour. No intervening list or save sweeps it.
    let path = root.0.join("drafts/window-1.json");
    let mut draft: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
    let revision = draft["saved_at_ms"].as_u64().unwrap();
    draft["saved_at_ms"] = serde_json::json!(revision - 60 * 60 * 1000);
    fs::write(&path, draft.to_string())?;
    let input = serde_json::json!({"id": "window-1"});
    let read = invoke(&mut store, &app, "read_draft", input.clone()).await?;
    catalog.call("read_draft", &input, &read).unwrap();
    assert_eq!(
        read,
        serde_json::json!({"id": "window-1", "revision": null, "scene": null})
    );
    assert!(!path.exists(), "individual reads sweep expired drafts");
    let again = invoke(&mut store, &app, "read_draft", input.clone()).await?;
    catalog.call("read_draft", &input, &again).unwrap();
    assert_eq!(again, read);
    Ok(())
}

/// Retiring names the revision the caller read: a window that wrote since keeps its newer draft, and
/// the revisions of one window rise with every write.
#[tokio::test(flavor = "current_thread")]
async fn a_retirement_removes_only_the_revision_that_was_read() -> wasmtime::Result<()> {
    let root = scratch("drafts-retire")?;
    let (mut store, app) = runtime(&root.0).await?;
    let revision = |read: &serde_json::Value| read["revision"].as_u64().unwrap();
    let mut seen = Vec::new();
    for pad in 1..=3 {
        invoke(
            &mut store,
            &app,
            "save_draft",
            serde_json::json!({"id": "open", "scene": big_scene("e", pad)}),
        )
        .await?;
        let read = kept_drafts(&mut store, &app).await?;
        seen.push(revision(&read[0]));
    }
    assert!(seen.windows(2).all(|pair| pair[0] < pair[1]), "{seen:?}");
    let stale = invoke(
        &mut store,
        &app,
        "retire_draft",
        serde_json::json!({"id": "open", "revision": seen[1]}),
    )
    .await?;
    assert_eq!(stale["retired"], false);
    assert_eq!(kept_drafts(&mut store, &app).await?.len(), 1);
    let current = invoke(
        &mut store,
        &app,
        "retire_draft",
        serde_json::json!({"id": "open", "revision": seen[2]}),
    )
    .await?;
    assert_eq!(current["retired"], true);
    assert!(kept_drafts(&mut store, &app).await?.is_empty());
    // Retiring what is gone, or an id that names no file, changes nothing and says so.
    let again = invoke(
        &mut store,
        &app,
        "retire_draft",
        serde_json::json!({"id": "open", "revision": seen[2]}),
    )
    .await?;
    assert_eq!(again["retired"], false);
    let refused = try_invoke(
        &mut store,
        &app,
        "retire_draft",
        serde_json::json!({"id": "../scene", "revision": 1}),
    )
    .await?;
    assert!(refused.is_err());
    Ok(())
}

/// Windows that each recover the one before and retire it after saving leave at most the newest
/// window's draft behind, however many replacements there are.
#[tokio::test(flavor = "current_thread")]
async fn replacements_that_retire_what_they_recovered_leave_only_the_newest_draft(
) -> wasmtime::Result<()> {
    let root = scratch("drafts-chain")?;
    let (mut store, app) = runtime(&root.0).await?;
    for window in 0..12 {
        let before = kept_drafts(&mut store, &app).await?;
        assert!(
            before.len() <= 1,
            "window {window} found {} drafts",
            before.len()
        );
        invoke(
            &mut store,
            &app,
            "save_draft",
            serde_json::json!({"id": format!("window-{window}"), "scene": big_scene(&format!("e{window}"), 1)}),
        )
        .await?;
        for draft in &before {
            let retired = invoke(
                &mut store,
                &app,
                "retire_draft",
                serde_json::json!({"id": draft["id"], "revision": draft["revision"]}),
            )
            .await?;
            assert_eq!(retired["retired"], true);
        }
    }
    assert_eq!(kept_drafts(&mut store, &app).await?.len(), 1);
    Ok(())
}
