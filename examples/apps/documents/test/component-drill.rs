//! The Documents App's real WASM component on a real WASI `data` directory. State is an in-memory
//! fixture with the kernel's version checks; the file broker (replace, import, export, snapshot) and
//! the picker are fixtures. The kernel path is covered by the hosted drills. Build the component with
//! the pinned SDK tools and pass it as `CHARIOX_DOCUMENTS_COMPONENT`.
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, VecDeque},
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

#[path = "../../../../packages/app-sdk-tools/tests/support/catalog_path.rs"]
mod catalog_path;

wasmtime::component::bindgen!({
    path: "../app-wit/wit",
    inline: r#"package chariox:documents-drill;
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
    records: BTreeMap<String, state::StateRecord>,
    tools: Vec<String>,
    /// What the person chooses next: (file name, bytes) per grant.
    chosen: Vec<(String, Vec<u8>)>,
    /// How many times the owner is still asked before they choose; `None` when they decline.
    waits: Option<u32>,
    /// How many requests for files reached the owner's prompt.
    asked: u32,
    offers: Vec<(String, Vec<u8>)>,
    snapshots: Vec<String>,
    /// Commits of the index the kernel refuses with a version conflict, as a concurrent writer would.
    conflicts: VecDeque<()>,
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
    fn tool(&mut self, name: String) {
        self.tools.push(name);
    }
    fn event(&mut self, _: String) {
        panic!("unexpected event")
    }
    fn lifecycle(&mut self, _: String) {
        panic!("unexpected lifecycle")
    }
}
fn error(code: &str, message: &str) -> AppError {
    AppError {
        code: code.into(),
        message: message.into(),
        retryable: None,
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
            if tx
                .writes
                .iter()
                .any(|w| matches!(w, state::Write::Put((key, _)) if key == "index"))
                && host.conflicts.pop_front().is_some()
            {
                return Err(error("CONFLICT", "App state version check failed"));
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
            Ok(state::Committed {
                revision: 1,
                receipts: vec![],
            })
        })
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
        panic!("a call that waits for the owner cannot outlast the kernel's 30 s")
    }
    async fn request_file(
        accessor: &Accessor<T, Self>,
        request: host::Pick,
    ) -> Result<host::Picked, AppError> {
        assert!(request.multiple && request.accept.contains(&".md".to_owned()));
        Ok(accessor.with(|mut access| {
            access.get().asked += 1;
            access.get().picked()
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
        Ok(accessor.with(|mut access| access.get().picked()))
    }
}
impl Host {
    /// Where the owner's choice stands after one more ask.
    fn picked(&mut self) -> host::Picked {
        match &mut self.waits {
            None => host::Picked::Declined,
            Some(0) => host::Picked::Granted(
                (0..self.chosen.len())
                    .map(|n| format!("grant-{n}"))
                    .collect(),
            ),
            Some(waits) => {
                *waits -= 1;
                host::Picked::Pending("pick-1".into())
            }
        }
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
        // The broker never creates parents.
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
        Ok("offer-1".into())
    }
    async fn import(
        accessor: &Accessor<T, Self>,
        grant: String,
        destination: String,
    ) -> Result<(u64, String), AppError> {
        accessor.with(|mut access| {
            let host = access.get();
            let n: usize = grant.trim_start_matches("grant-").parse().unwrap();
            let (name, bytes) = host.chosen[n].clone();
            fs::write(host.data.join(destination), &bytes).unwrap();
            Ok((bytes.len() as u64, name))
        })
    }
    async fn snapshot(
        accessor: &Accessor<T, Self>,
        name: String,
        consistency: files::Consistency,
    ) -> Result<files::SnapshotInfo, AppError> {
        assert!(matches!(consistency, files::Consistency::Quiescent));
        accessor.with(|mut access| access.get().snapshots.push(name));
        Ok(files::SnapshotInfo {
            snapshot_id: "snap-1".into(),
            consistency,
            files: 3,
            bytes: 99,
        })
    }
}

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

async fn runtime(label: &str) -> wasmtime::Result<(Scratch, Store<Host>, Drill)> {
    let root = Scratch(
        PathBuf::from(std::env::var("TMPDIR")?)
            .join(format!("documents-{label}-{}", std::process::id())),
    );
    fs::create_dir(&root.0)?;
    let mut config = Config::new();
    config.wasm_component_model_async(true);
    let engine = Engine::new(&config)?;
    let component = Component::from_file(&engine, std::env::var("CHARIOX_DOCUMENTS_COMPONENT")?)?;
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
            records: BTreeMap::new(),
            tools: vec![],
            chosen: vec![],
            waits: Some(0),
            asked: 0,
            offers: vec![],
            snapshots: vec![],
            conflicts: VecDeque::new(),
        },
    );
    let app = Drill::instantiate_async(&mut store, &component, &linker).await?;
    app.chariox_app1_0_0_handlers()
        .call_register(&mut store)
        .await?
        .unwrap();
    Ok((root, store, app))
}

async fn call(
    store: &mut Store<Host>,
    app: &Drill,
    name: &str,
    input: Value,
) -> Result<Value, AppError> {
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
            app.chariox_app1_0_0_handlers()
                .call_call_tool(accessor, name.into(), input.to_string(), context, cancel)
                .await
        })
        .await
        .unwrap()
        .unwrap();
    result.map(|text| serde_json::from_str(&text).unwrap())
}

async fn ok(store: &mut Store<Host>, app: &Drill, name: &str, input: Value) -> Value {
    call(store, app, name, input)
        .await
        .unwrap_or_else(|e| panic!("{name}: {} {}", e.code, e.message))
}

fn folder(root: &Scratch, id: &str) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(root.0.join("documents").join(id))
        .map(|dir| {
            dir.map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[tokio::test(flavor = "current_thread")]
async fn the_view_and_the_tools_are_what_the_signed_manifest_lists() -> wasmtime::Result<()> {
    let (_root, store, _) = runtime("manifest").await?;
    let listed: Value = serde_json::from_str(include_str!("../schemas/tools.json"))?;
    let mut listed: Vec<String> = listed["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().into())
        .collect();
    let mut registered = store.data().tools.clone();
    listed.sort();
    registered.sort();
    assert_eq!(registered, listed);
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn a_document_lives_through_edits_history_restore_and_delete() -> wasmtime::Result<()> {
    let (root, mut store, app) = runtime("lifecycle").await?;
    let (store, app) = (&mut store, &app);
    let made = ok(
        store,
        app,
        "create_document",
        json!({"title": "Plan", "folder": "work", "content": "v1"}),
    )
    .await;
    let id = made["id"].as_str().unwrap().to_owned();
    assert_eq!(made["revision"], 1);
    assert_eq!(folder(&root, &id).len(), 1);
    assert!(folder(&root, &id)[0].starts_with("1-") && folder(&root, &id)[0].ends_with(".md"));

    let saved = ok(
        store,
        app,
        "update_document",
        json!({"id": id, "expected_revision": 1, "content": "v2"}),
    )
    .await;
    assert_eq!(saved["revision"], 2);
    // A human and an agent that both started from revision 1: the second is refused, not merged.
    let stale = call(
        store,
        app,
        "update_document",
        json!({"id": id, "expected_revision": 1, "content": "stale"}),
    )
    .await;
    assert_eq!(stale.unwrap_err().code, "CONFLICT");
    assert_eq!(
        ok(store, app, "read_document", json!({"id": id})).await["content"],
        "v2"
    );
    assert_eq!(
        ok(
            store,
            app,
            "read_document",
            json!({"id": id, "revision": 1})
        )
        .await["content"],
        "v1"
    );

    // A rename is a revision of its own and restores nothing visible.
    let renamed = ok(
        store,
        app,
        "update_document",
        json!({"id": id, "expected_revision": 2, "title": "Plan B"}),
    )
    .await;
    assert_eq!(
        (renamed["revision"].as_u64(), renamed["title"].as_str()),
        (Some(3), Some("Plan B"))
    );
    let restored = ok(
        store,
        app,
        "restore_version",
        json!({"id": id, "revision": 1, "expected_revision": 3}),
    )
    .await;
    assert_eq!(restored["revision"], 4);
    assert_eq!(
        ok(store, app, "read_document", json!({"id": id})).await["content"],
        "v1"
    );
    assert_eq!(
        ok(store, app, "list_documents", json!({"folder": "work"})).await["documents"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        ok(store, app, "list_documents", json!({"folder": "none"})).await["documents"]
            .as_array()
            .unwrap()
            .len(),
        0
    );

    // Ten revisions are kept and older files are pruned.
    for expected in 4..16u64 {
        ok(
            store,
            app,
            "update_document",
            json!({"id": id, "expected_revision": expected, "content": format!("r{expected}")}),
        )
        .await;
    }
    let read = ok(store, app, "read_document", json!({"id": id})).await;
    assert_eq!(read["revision"], 16);
    assert_eq!(read["versions"].as_array().unwrap().len(), 10);
    assert_eq!(read["versions"][0], 7);
    assert_eq!(
        call(
            store,
            app,
            "read_document",
            json!({"id": id, "revision": 1})
        )
        .await
        .unwrap_err()
        .code,
        "NOT_FOUND"
    );
    let kept = folder(&root, &id);
    assert!(
        kept.iter()
            .all(|name| name.split('-').next().unwrap().parse::<u64>().unwrap() >= 7),
        "{kept:?}"
    );

    ok(store, app, "delete_document", json!({"id": id})).await;
    assert!(folder(&root, &id).is_empty() && !root.0.join("documents").join(&id).exists());
    assert_eq!(
        call(store, app, "read_document", json!({"id": id}))
            .await
            .unwrap_err()
            .code,
        "NOT_FOUND"
    );
    Ok(())
}

fn kept_drafts(root: &Scratch) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(root.0.join("drafts"))
        .map(|dir| {
            dir.map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// Draft names as `kept_drafts` lists them; ids are random, so the expected names are sorted too.
fn sorted(mut names: Vec<String>) -> Vec<String> {
    names.sort();
    names
}

async fn make(store: &mut Store<Host>, app: &Drill, title: &str) -> String {
    ok(
        store,
        app,
        "create_document",
        json!({"title": title, "content": "v1"}),
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// Drafts are ordered by the millisecond they were kept, so each write waits for the next one.
async fn keep(store: &mut Store<Host>, app: &Drill, editor: &str, id: &str, content: &str) {
    std::thread::sleep(std::time::Duration::from_millis(3));
    ok(
        store,
        app,
        "save_draft",
        json!({"editor": editor, "id": id, "base_revision": 1, "content": content}),
    )
    .await;
}

async fn claim(store: &mut Store<Host>, app: &Drill, editor: &str) -> Value {
    ok(store, app, "claim_draft", json!({"editor": editor})).await["draft"].clone()
}

// Every path that touches a draft (save, claim, discard, rename, delete) has one test below, and the
// last one is the scenario the review found: two windows on one document.

#[tokio::test(flavor = "current_thread")]
async fn save_keeps_the_revision_the_edit_started_from_for_its_own_editor() -> wasmtime::Result<()>
{
    let (root, mut store, app) = runtime("draft_save").await?;
    let (store, app) = (&mut store, &app);
    let id = make(store, app, "Plan").await;
    assert_eq!(claim(store, app, "w1").await, Value::Null);

    ok(
        store,
        app,
        "save_draft",
        json!({"editor": "w1", "id": id, "base_revision": 1, "title": "Plan 2", "folder": "work", "content": "unsaved"}),
    )
    .await;
    assert_eq!(kept_drafts(&root), vec![format!("{id}.w1.json")]);
    // Someone else saves revision 2; the draft still names the revision it started from.
    ok(
        store,
        app,
        "update_document",
        json!({"id": id, "expected_revision": 1, "content": "v2"}),
    )
    .await;
    let kept = claim(store, app, "w1").await;
    assert_eq!(kept["id"], id.as_str());
    assert_eq!(kept["base_revision"], 1);
    assert_eq!(kept["title"], "Plan 2");
    assert_eq!(kept["folder"], "work");
    assert_eq!(kept["content"], "unsaved");
    assert_eq!(
        ok(store, app, "read_document", json!({"id": id})).await["content"],
        "v2",
        "a draft is not a revision"
    );

    // A later save of the same editor replaces its draft; it does not add a second.
    keep(store, app, "w1", &id, "typed more").await;
    assert_eq!(kept_drafts(&root).len(), 1);
    assert_eq!(claim(store, app, "w1").await["content"], "typed more");

    let missing = call(
        store,
        app,
        "save_draft",
        json!({"editor": "w1", "id": "nope", "base_revision": 1, "content": "x"}),
    )
    .await;
    assert_eq!(missing.unwrap_err().code, "NOT_FOUND");
    for editor in ["", "../w", "w.1", &"e".repeat(65)] {
        let bad = call(
            store,
            app,
            "save_draft",
            json!({"editor": editor, "id": id, "base_revision": 1, "content": "x"}),
        )
        .await;
        assert_eq!(
            bad.unwrap_err().code,
            "INVALID_ARGUMENT",
            "editor {editor:?}"
        );
    }
    let no_editor = call(
        store,
        app,
        "save_draft",
        json!({"id": id, "base_revision": 1, "content": "x"}),
    )
    .await;
    assert!(no_editor.is_err(), "a draft names its editor");
    assert_eq!(kept_drafts(&root).len(), 1);
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn claim_gives_an_editor_its_own_draft_else_a_copy_of_the_latest_of_another(
) -> wasmtime::Result<()> {
    let (root, mut store, app) = runtime("draft_claim").await?;
    let (store, app) = (&mut store, &app);
    let (plan, notes) = (
        make(store, app, "Plan").await,
        make(store, app, "Notes").await,
    );
    keep(store, app, "old", &plan, "older").await;
    keep(store, app, "old", &notes, "newer").await;

    // The replacement is a new editor: it is handed a copy of the latest draft, and the source stays.
    let copied = claim(store, app, "new").await;
    assert_eq!(
        (copied["id"].as_str(), copied["content"].as_str()),
        (Some(notes.as_str()), Some("newer"))
    );
    assert_eq!(
        kept_drafts(&root),
        sorted(vec![
            format!("{notes}.new.json"),
            format!("{notes}.old.json"),
            format!("{plan}.old.json")
        ]),
        "the copy is the new editor's own; nothing of the old editor is moved or removed"
    );
    // Asked again, it keeps what it has.
    assert_eq!(claim(store, app, "new").await["content"], "newer");
    keep(store, app, "new", &plan, "own plan").await;
    assert_eq!(
        claim(store, app, "new").await["content"],
        "own plan",
        "an editor with a draft of its own takes none of another's"
    );
    assert_eq!(kept_drafts(&root).len(), 4);
    assert_eq!(claim(store, app, "idle").await["content"], "own plan");
    for editor in ["../w", ""] {
        let bad = call(store, app, "claim_draft", json!({"editor": editor})).await;
        assert_eq!(bad.unwrap_err().code, "INVALID_ARGUMENT");
    }
    Ok(())
}

/// The review's first repro: the only draft of a window that is still open must outlive another
/// window opening, restoring it and discarding it.
#[tokio::test(flavor = "current_thread")]
async fn a_live_editors_only_draft_survives_another_editor_claiming_and_discarding(
) -> wasmtime::Result<()> {
    let (root, mut store, app) = runtime("draft_live").await?;
    let (store, app) = (&mut store, &app);
    let id = make(store, app, "Plan").await;
    keep(store, app, "a", &id, "typed in the open window").await;

    // Another window starts, is handed the edit, and its person gives it up.
    assert_eq!(
        claim(store, app, "b").await["content"],
        "typed in the open window"
    );
    ok(
        store,
        app,
        "discard_draft",
        json!({"editor": "b", "id": id}),
    )
    .await;
    assert_eq!(kept_drafts(&root), vec![format!("{id}.a.json")]);

    // The first window is replaced; its successor still finds the edit.
    assert_eq!(
        claim(store, app, "a-next").await["content"],
        "typed in the open window"
    );
    // A window that is still open keeps its own draft through all of it.
    assert_eq!(
        claim(store, app, "a").await["content"],
        "typed in the open window"
    );
    Ok(())
}

/// A draft older than a day is swept by the next claim; one that is not stays, whoever kept it.
#[tokio::test(flavor = "current_thread")]
async fn a_claim_sweeps_drafts_older_than_a_day_and_no_others() -> wasmtime::Result<()> {
    let (root, mut store, app) = runtime("draft_age").await?;
    let (store, app) = (&mut store, &app);
    let id = make(store, app, "Plan").await;
    keep(store, app, "fresh", &id, "recent").await;
    let stale = json!({
        "editor": "stale", "id": id, "base_revision": 1, "saved_at_ms": 1,
        "title": "", "folder": "", "content": "from long ago",
    });
    fs::write(
        root.0.join("drafts").join(format!("{id}.stale.json")),
        stale.to_string(),
    )?;
    assert_eq!(kept_drafts(&root).len(), 2);
    assert_eq!(claim(store, app, "next").await["content"], "recent");
    assert_eq!(
        kept_drafts(&root),
        sorted(vec![format!("{id}.fresh.json"), format!("{id}.next.json")]),
        "the day-old draft is gone, the recent one and its copy stay"
    );
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn discard_removes_only_the_draft_of_that_editor_and_document() -> wasmtime::Result<()> {
    let (root, mut store, app) = runtime("draft_discard").await?;
    let (store, app) = (&mut store, &app);
    let (plan, notes) = (
        make(store, app, "Plan").await,
        make(store, app, "Notes").await,
    );
    for (editor, id) in [("w1", &plan), ("w2", &plan), ("w1", &notes)] {
        keep(store, app, editor, id, "edit").await;
    }
    ok(
        store,
        app,
        "discard_draft",
        json!({"editor": "w1", "id": plan}),
    )
    .await;
    assert_eq!(
        kept_drafts(&root),
        sorted(vec![format!("{notes}.w1.json"), format!("{plan}.w2.json")])
    );
    // Discarding what is not there is not an error, and removes nothing else.
    ok(
        store,
        app,
        "discard_draft",
        json!({"editor": "w3", "id": plan}),
    )
    .await;
    ok(
        store,
        app,
        "discard_draft",
        json!({"editor": "w1", "id": plan}),
    )
    .await;
    assert_eq!(kept_drafts(&root).len(), 2);
    for input in [
        json!({"editor": "w1", "id": "../documents"}),
        json!({"editor": "../w", "id": plan}),
        json!({"editor": "w1.x", "id": plan}),
        json!({"editor": "w1"}),
        json!({"id": plan}),
    ] {
        assert!(call(store, app, "discard_draft", input).await.is_err());
    }
    assert_eq!(kept_drafts(&root).len(), 2);
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn renaming_or_moving_a_document_leaves_every_editors_draft_of_it() -> wasmtime::Result<()> {
    let (root, mut store, app) = runtime("draft_rename").await?;
    let (store, app) = (&mut store, &app);
    let id = make(store, app, "Plan").await;
    keep(store, app, "w1", &id, "first").await;
    keep(store, app, "w2", &id, "second").await;
    // Another editor renames and files the document; its id, and with it every draft's key, stays.
    ok(
        store,
        app,
        "update_document",
        json!({"id": id, "expected_revision": 1, "title": "Renamed", "folder": "work"}),
    )
    .await;
    assert_eq!(
        kept_drafts(&root),
        sorted(vec![format!("{id}.w1.json"), format!("{id}.w2.json")])
    );
    let first = claim(store, app, "w1").await;
    assert_eq!(
        (first["content"].as_str(), first["base_revision"].as_u64()),
        (Some("first"), Some(1))
    );
    assert_eq!(claim(store, app, "w2").await["content"], "second");
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn deleting_a_document_offers_none_of_its_drafts_removes_none_and_refuses_new_ones(
) -> wasmtime::Result<()> {
    let (root, mut store, app) = runtime("draft_delete").await?;
    let (store, app) = (&mut store, &app);
    let (gone, kept) = (
        make(store, app, "Gone").await,
        make(store, app, "Kept").await,
    );
    keep(store, app, "w1", &gone, "a").await;
    keep(store, app, "w2", &gone, "b").await;
    keep(store, app, "w2", &kept, "c").await;
    ok(store, app, "delete_document", json!({"id": gone})).await;

    // A window that starts now is offered the draft of the document that is left; the drafts of the
    // deleted one are not offered, and no one but their editors (or a day's age) removes them.
    assert_eq!(claim(store, app, "w3").await["id"], kept.as_str());
    assert_eq!(
        kept_drafts(&root),
        sorted(vec![
            format!("{gone}.w1.json"),
            format!("{gone}.w2.json"),
            format!("{kept}.w2.json"),
            format!("{kept}.w3.json")
        ])
    );
    let late = call(
        store,
        app,
        "save_draft",
        json!({"editor": "w1", "id": gone, "base_revision": 1, "content": "late"}),
    )
    .await;
    assert_eq!(late.unwrap_err().code, "NOT_FOUND");
    ok(store, app, "delete_document", json!({"id": kept})).await;
    assert_eq!(claim(store, app, "w4").await, Value::Null);
    assert_eq!(kept_drafts(&root).len(), 4);
    // The editor of a deleted document's draft may still discard it.
    ok(
        store,
        app,
        "discard_draft",
        json!({"editor": "w1", "id": gone}),
    )
    .await;
    assert_eq!(kept_drafts(&root).len(), 3);
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn the_drafts_kept_at_once_are_capped_per_editor_and_document_pair() -> wasmtime::Result<()> {
    let (root, mut store, app) = runtime("draft_cap").await?;
    let (store, app) = (&mut store, &app);
    let id = make(store, app, "Plan").await;
    for n in 0..32 {
        keep(store, app, &format!("w{n}"), &id, "edit").await;
    }
    let over = call(
        store,
        app,
        "save_draft",
        json!({"editor": "extra", "id": id, "base_revision": 1, "content": "x"}),
    )
    .await;
    assert_eq!(over.unwrap_err().code, "INVALID_STATE");
    keep(store, app, "w7", &id, "replaced").await;
    assert_eq!(kept_drafts(&root).len(), 32, "a kept pair is still written");
    // A new window is still handed the edit; no copy fits, so the source stays and nothing else is
    // removed to make room.
    assert_eq!(claim(store, app, "late").await["content"], "replaced");
    assert_eq!(kept_drafts(&root).len(), 32);
    assert!(!kept_drafts(&root).contains(&format!("{id}.late.json")));
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn two_windows_on_one_document_keep_their_own_edit_when_the_other_saves_or_is_replaced(
) -> wasmtime::Result<()> {
    let (root, mut store, app) = runtime("draft_two_windows").await?;
    let (store, app) = (&mut store, &app);
    let id = make(store, app, "Plan").await;
    keep(store, app, "w1", &id, "first window").await;
    keep(store, app, "w2", &id, "second window").await;

    // The first window saves its edit and gives up its draft: the second window's stays.
    ok(
        store,
        app,
        "update_document",
        json!({"id": id, "expected_revision": 1, "content": "first window"}),
    )
    .await;
    ok(
        store,
        app,
        "discard_draft",
        json!({"editor": "w1", "id": id}),
    )
    .await;
    assert_eq!(kept_drafts(&root), vec![format!("{id}.w2.json")]);

    // The first window is replaced: its successor is handed a copy of the second window's edit, and
    // the second window, still open, keeps its own.
    let handed = claim(store, app, "w1-next").await;
    assert_eq!(handed["content"], "second window");
    assert_eq!(
        handed["base_revision"], 1,
        "it names the revision it started from"
    );
    assert_eq!(
        kept_drafts(&root),
        sorted(vec![format!("{id}.w1-next.json"), format!("{id}.w2.json")])
    );
    // The second window keeps typing and saves again; it writes its own draft and nothing of the
    // successor's is overwritten.
    keep(store, app, "w2", &id, "second again").await;
    assert_eq!(
        claim(store, app, "w1-next").await["content"],
        "second window"
    );
    assert_eq!(claim(store, app, "w2").await["content"], "second again");
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn a_lost_race_retries_and_leaves_no_stray_file() -> wasmtime::Result<()> {
    let (root, mut store, app) = runtime("race").await?;
    store.data_mut().conflicts.push_back(());
    let made = ok(
        &mut store,
        &app,
        "create_document",
        json!({"title": "Raced", "content": "x"}),
    )
    .await;
    let id = made["id"].as_str().unwrap();
    assert_eq!(
        folder(&root, id).len(),
        1,
        "the file the lost attempt wrote is removed"
    );
    store.data_mut().conflicts.extend([(); 5]);
    let refused = call(
        &mut store,
        &app,
        "create_document",
        json!({"title": "Never", "content": "x"}),
    )
    .await;
    assert_eq!(refused.unwrap_err().code, "CONFLICT");
    assert_eq!(
        ok(&mut store, &app, "list_documents", json!({})).await["documents"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let entries: Vec<_> = fs::read_dir(root.0.join("documents"))?.collect();
    assert_eq!(
        entries.len(),
        1,
        "a create that never committed leaves no folder"
    );
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn the_view_serves_its_state_and_the_feed_of_every_change() -> wasmtime::Result<()> {
    let (_root, mut store, app) = runtime("feed").await?;
    let (store, app) = (&mut store, &app);
    let declared: Value = serde_json::from_str(include_str!("../schemas/ui-actions.json"))?;
    assert_eq!(
        ok(store, app, "view_actions", json!({})).await,
        declared["uiActions"]
    );
    let made = ok(store, app, "create_document", json!({"title": "A"})).await;
    let id = made["id"].as_str().unwrap().to_owned();
    ok(
        store,
        app,
        "update_document",
        json!({"id": id, "expected_revision": 1, "content": "x"}),
    )
    .await;
    ok(store, app, "delete_document", json!({"id": id})).await;
    let feed = ok(store, app, "view_changes", json!({"since": 0})).await;
    assert_eq!(feed["latest"], 3);
    let deltas: Vec<Value> = feed["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["delta"].clone())
        .collect();
    assert_eq!(
        deltas,
        [
            json!({"id": id, "revision": 1}),
            json!({"id": id, "revision": 2}),
            json!({"id": id, "deleted": true})
        ]
    );
    assert_eq!(
        ok(store, app, "view_changes", json!({"since": 2})).await["changes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        ok(store, app, "view_state", json!({})).await,
        json!({"documents": []})
    );
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn an_owner_who_takes_longer_than_one_call_still_imports_with_the_same_request(
) -> wasmtime::Result<()> {
    let (_root, mut store, app) = runtime("slow-choice").await?;
    let (store, app) = (&mut store, &app);
    store.data_mut().chosen = vec![("Later.md".into(), b"# Later\n".to_vec())];
    // The owner has not chosen when the first call ends, nor the second.
    store.data_mut().waits = Some(3);
    let first = ok(store, app, "import_documents", json!({})).await;
    assert_eq!(first, json!({"pending": true, "importId": "pick-1"}));
    let second = ok(
        store,
        app,
        "import_documents",
        json!({"importId": "pick-1"}),
    )
    .await;
    assert_eq!(second, json!({"pending": true, "importId": "pick-1"}));
    assert_eq!(
        ok(store, app, "list_documents", json!({})).await,
        json!({"documents": []}),
        "nothing is imported before the choice"
    );
    let third = ok(
        store,
        app,
        "import_documents",
        json!({"importId": "pick-1", "folder": "inbox"}),
    )
    .await;
    assert_eq!(third["imported"][0]["title"], "Later");
    assert_eq!(third["imported"][0]["folder"], "inbox");
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn files_the_window_read_become_documents_without_asking_the_owner() -> wasmtime::Result<()> {
    let (_root, mut store, app) = runtime("uploaded").await?;
    let (store, app) = (&mut store, &app);
    let imported = ok(
        store,
        app,
        "import_documents",
        json!({"folder": "inbox", "files": [
            {"name": "Meeting notes.MD", "content": "# Notes\n"},
            {"name": "page.html", "content": "<p>hi</p>"},
        ]}),
    )
    .await;
    assert_eq!(imported["imported"][0]["title"], "Meeting notes");
    assert_eq!(imported["imported"][1]["kind"], "html");
    assert_eq!(imported["imported"][1]["folder"], "inbox");
    assert_eq!(imported["skipped"], json!([]));
    assert_eq!(store.data().asked, 0, "the window already has the files");
    assert_eq!(
        ok(store, app, "list_documents", json!({})).await["documents"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn an_owner_who_declines_ends_the_import_with_a_reason() -> wasmtime::Result<()> {
    let (_root, mut store, app) = runtime("declined").await?;
    let (store, app) = (&mut store, &app);
    store.data_mut().waits = None;
    let refused = call(store, app, "import_documents", json!({}))
        .await
        .unwrap_err();
    assert_eq!(refused.code, "CANCELLED");
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn the_owner_chooses_files_to_import_and_is_offered_a_copy_to_export() -> wasmtime::Result<()>
{
    let (root, mut store, app) = runtime("files").await?;
    let (store, app) = (&mut store, &app);
    store.data_mut().chosen = vec![
        ("Meeting notes.MD".into(), b"# Notes\n".to_vec()),
        ("page.html".into(), b"<p>hi</p>".to_vec()),
        ("photo.txt".into(), vec![0xff, 0xfe, 0x00]),
    ];
    let imported = ok(store, app, "import_documents", json!({"folder": "inbox"})).await;
    let titles: Vec<&str> = imported["imported"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["Meeting notes", "page"]);
    assert_eq!(imported["imported"][1]["kind"], "html");
    assert_eq!(imported["imported"][0]["folder"], "inbox");
    assert_eq!(
        imported["skipped"],
        json!([{"name": "photo.txt", "reason": "photo.txt is not UTF-8 text"}])
    );
    assert!(
        !root.0.join("imports").read_dir()?.any(|_| true),
        "the staged copies are removed"
    );

    let id = imported["imported"][0]["id"].as_str().unwrap();
    assert_eq!(
        ok(store, app, "export_document", json!({"id": id})).await,
        json!({"offered": "Meeting notes.md"})
    );
    let (path, bytes) = store.data().offers[0].clone();
    assert!(
        path.starts_with("exports/") && path.ends_with("/Meeting notes.md"),
        "{path}"
    );
    assert_eq!(bytes, b"# Notes\n");
    assert!(
        !root.0.join("exports").read_dir()?.any(|_| true),
        "the staged export is removed"
    );

    let snapshot = ok(
        store,
        app,
        "snapshot_documents",
        json!({"name": "before-cleanup"}),
    )
    .await;
    assert_eq!(snapshot["snapshotId"], "snap-1");
    assert_eq!(store.data().snapshots, ["before-cleanup"]);
    Ok(())
}

/// The kernel refuses a call or a result over 512 KiB. A document that the App accepts must therefore
/// pass that check in every tool that carries it, with the draft tools' envelope too, and a document
/// whose JSON text is too big for it (a small file of quotes doubles in size) is refused by the App
/// with its own reason, not accepted and then stranded.
#[tokio::test(flavor = "current_thread")]
async fn a_near_limit_document_and_its_draft_pass_the_catalog_and_a_bigger_one_is_refused(
) -> wasmtime::Result<()> {
    let (_root, mut store, app) = runtime("catalog_budget").await?;
    let mut catalog = catalog_path::Catalog::new(include_bytes!("../schemas/tools.json"));
    // Through the catalog as the kernel does it: the input before the call, the result after it.
    async fn through(
        catalog: &mut catalog_path::Catalog,
        store: &mut Store<Host>,
        app: &Drill,
        tool: &str,
        input: Value,
    ) -> Result<Value, AppError> {
        let result = call(store, app, tool, input.clone()).await?;
        Ok(catalog
            .call(tool, &input, &result)
            .unwrap_or_else(|e| panic!("{tool}: {e:?}")))
    }
    let near = "x".repeat(504 * 1024 - 2);
    let created = through(
        &mut catalog,
        &mut store,
        &app,
        "create_document",
        json!({"title": "Near the limit", "folder": "f".repeat(200), "content": near}),
    )
    .await
    .unwrap();
    let id = created["id"].as_str().unwrap().to_owned();
    let editor = "e".repeat(32);
    through(
        &mut catalog,
        &mut store,
        &app,
        "save_draft",
        json!({"editor": editor, "id": id, "base_revision": 1, "title": "t".repeat(200),
            "folder": "f".repeat(200), "content": near}),
    )
    .await
    .unwrap();
    let claimed = through(
        &mut catalog,
        &mut store,
        &app,
        "claim_draft",
        json!({"editor": "w".repeat(32)}),
    )
    .await
    .unwrap();
    assert_eq!(
        claimed["draft"]["content"].as_str().unwrap().len(),
        near.len()
    );
    // 300 KiB of quotes is 600 KiB of JSON: the App refuses it, for the reason, in every tool.
    let quotes = "\"".repeat(300 * 1024);
    for (tool, input) in [
        (
            "create_document",
            json!({"title": "Quotes", "content": quotes}),
        ),
        (
            "update_document",
            json!({"id": id, "expected_revision": 1, "content": quotes}),
        ),
        (
            "save_draft",
            json!({"editor": editor, "id": id, "base_revision": 1, "content": quotes}),
        ),
    ] {
        let refused = call(&mut store, &app, tool, input).await.unwrap_err();
        assert_eq!(refused.code, "LIMIT_EXCEEDED", "{tool}");
        assert!(
            refused.message.contains("504 KiB"),
            "{tool}: {}",
            refused.message
        );
    }
    Ok(())
}
