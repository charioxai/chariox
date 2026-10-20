use super::*;
use crate::durable_state::{
    app_installation_staging::AppVerifiedInstallationMutation,
    app_publishers::AppPublisherMutation,
    apps::{AppRegistryMutation, AppRegistryOutcome},
};
use chariox_app_package::{pack, verify, Limits, Manifest, TrustedPublisher, VerificationPolicy};
use chariox_app_runtime::{
    app_catalog::AppCatalog,
    installation::{
        CapabilityApproval, CapabilityDecision, InstallationRegistry, VerifiedInstallCandidate,
    },
    managed_state::StateWrite,
    publisher_trust::TrustDecision,
};
use ed25519_dalek::SigningKey;
use serde_json::json;
use std::{collections::BTreeMap, path::PathBuf, time::Duration};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "chariox-app-state-writer-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn open(&self) -> DurableKernelStateStore {
        DurableKernelStateStore::open_owned(self.0.join("kernel.sqlite")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn publisher() -> TrustedPublisher {
    TrustedPublisher {
        publisher_id: "com.example".into(),
        key_id: "state-key".into(),
        public_key: SigningKey::from_bytes(&[71; 32]).verifying_key(),
    }
}
fn decision(id: &str) -> TrustDecision {
    TrustDecision {
        decision_id: id.into(),
        authority_ref: "kernel-fixture-decision".into(),
    }
}
pub(super) fn catalog(store: &DurableKernelStateStore) -> Arc<EventCatalog> {
    catalog_with_package(store, package())
}
pub(super) fn tool_catalog(store: &DurableKernelStateStore) -> Arc<EventCatalog> {
    catalog_with_package(store, tool_package())
}
pub(super) fn http_catalog(store: &DurableKernelStateStore) -> Arc<EventCatalog> {
    catalog_with_package(store, http_package())
}
fn catalog_with_package(
    store: &DurableKernelStateStore,
    package_bytes: (Vec<u8>, TrustedPublisher),
) -> Arc<EventCatalog> {
    catalog_for_owner(store, "alice", package_bytes)
}
pub(super) fn catalog_for_owner(
    store: &DurableKernelStateStore,
    owner: &str,
    package_bytes: (Vec<u8>, TrustedPublisher),
) -> Arc<EventCatalog> {
    store
        .mutate_app_publisher(
            owner,
            AppPublisherMutation::Enroll {
                publisher: publisher(),
                expected_revision: 0,
                decision: decision("enroll"),
                now_ms: 1,
            },
        )
        .unwrap();
    install_package(store, owner, "installed", package_bytes)
}
/// Installs and activates `installation_id` from `package_bytes` for an
/// owner who already trusts the fixture publisher.
pub(super) fn install_package(
    store: &DurableKernelStateStore,
    owner: &str,
    installation_id: &str,
    package_bytes: (Vec<u8>, TrustedPublisher),
) -> Arc<EventCatalog> {
    let (bytes, publisher) = package_bytes;
    let trust = store
        .trusted_app_publisher(owner, "com.example", "state-key")
        .unwrap();
    let package = verify(
        &bytes,
        &VerificationPolicy::new(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, vec![publisher]),
    )
    .unwrap();
    let candidate = VerifiedInstallCandidate::from_verified(&package, &trust).unwrap();
    let AppRegistryOutcome::Update(record) = store
        .mutate_verified_app_installation(
            owner,
            AppVerifiedInstallationMutation::CreateAndStage {
                installation_id: installation_id.into(),
                candidate,
                now_ms: 1,
            },
        )
        .unwrap()
    else {
        panic!("expected stage")
    };
    let binding = {
        let mut connection = store.connection.lock().unwrap();
        InstallationRegistry::new(&mut connection)
            .staged_trust(owner, &record.token)
            .unwrap()
    };
    for operation in [
        AppRegistryMutation::Decide {
            token: record.token.clone(),
            decision: CapabilityDecision::Approved {
                approval: CapabilityApproval {
                    decision_id: "state-grant".into(),
                    authority_ref: "kernel-fixture".into(),
                },
            },
            now_ms: 2,
        },
        AppRegistryMutation::Quiesce {
            token: record.token.clone(),
            now_ms: 3,
        },
        AppRegistryMutation::MarkPrepared {
            token: record.token.clone(),
            now_ms: 4,
        },
    ] {
        store.mutate_app_installation(owner, operation).unwrap();
    }
    store
        .mutate_verified_app_installation(
            owner,
            AppVerifiedInstallationMutation::Commit {
                token: record.token,
                now_ms: 5,
            },
        )
        .unwrap();
    Arc::new(
        EventCatalog::compile(
            &package,
            Arc::new(AppCatalog::compile(&package, &binding, &trust).unwrap()),
        )
        .unwrap(),
    )
}
/// Updates the owner's installation to `package` as an approved update
/// leaves it.
pub(super) fn update_package(
    store: &DurableKernelStateStore,
    owner: &str,
    installation_id: &str,
    package_bytes: (Vec<u8>, TrustedPublisher),
) {
    let (bytes, publisher) = package_bytes;
    let trust = store
        .trusted_app_publisher(owner, "com.example", "state-key")
        .unwrap();
    let package = verify(
        &bytes,
        &VerificationPolicy::new(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, vec![publisher]),
    )
    .unwrap();
    let candidate = VerifiedInstallCandidate::from_verified(&package, &trust).unwrap();
    let expected_generation = store
        .get_app_installation(owner, installation_id)
        .unwrap()
        .generation;
    let AppRegistryOutcome::Update(record) = store
        .mutate_verified_app_installation(
            owner,
            AppVerifiedInstallationMutation::Stage {
                installation_id: installation_id.into(),
                expected_generation,
                candidate,
                now_ms: 10,
            },
        )
        .unwrap()
    else {
        panic!("expected stage")
    };
    for operation in [
        AppRegistryMutation::Decide {
            token: record.token.clone(),
            decision: CapabilityDecision::Approved {
                approval: CapabilityApproval {
                    decision_id: "state-update".into(),
                    authority_ref: "kernel-fixture".into(),
                },
            },
            now_ms: 11,
        },
        AppRegistryMutation::Quiesce {
            token: record.token.clone(),
            now_ms: 12,
        },
        AppRegistryMutation::MarkPrepared {
            token: record.token.clone(),
            now_ms: 13,
        },
    ] {
        store.mutate_app_installation(owner, operation).unwrap();
    }
    store
        .mutate_verified_app_installation(
            owner,
            AppVerifiedInstallationMutation::Commit {
                token: record.token,
                now_ms: 14,
            },
        )
        .unwrap();
}
pub(super) fn inbox_package_version(version: &str, schema: u32) -> (Vec<u8>, TrustedPublisher) {
    package_build(false, false, version, schema, true, false)
}
pub(super) fn package() -> (Vec<u8>, TrustedPublisher) {
    package_with_tools(false)
}
pub(super) fn stage_approved_update(
    store: &DurableKernelStateStore,
    owner: &str,
    installation_id: &str,
    (bytes, publisher): (Vec<u8>, TrustedPublisher),
) {
    let trust = store
        .trusted_app_publisher(owner, "com.example", "state-key")
        .unwrap();
    let package = verify(
        &bytes,
        &VerificationPolicy::new(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, vec![publisher]),
    )
    .unwrap();
    let AppRegistryOutcome::Update(record) = store
        .mutate_verified_app_installation(
            owner,
            AppVerifiedInstallationMutation::Stage {
                installation_id: installation_id.into(),
                expected_generation: 1,
                candidate: VerifiedInstallCandidate::from_verified(&package, &trust).unwrap(),
                now_ms: 10,
            },
        )
        .unwrap()
    else {
        panic!("expected stage")
    };
    store
        .mutate_app_installation(
            owner,
            AppRegistryMutation::Decide {
                token: record.token,
                decision: CapabilityDecision::Approved {
                    approval: CapabilityApproval {
                        decision_id: "update-grant".into(),
                        authority_ref: "kernel-fixture".into(),
                    },
                },
                now_ms: 11,
            },
        )
        .unwrap();
}
pub(super) fn tool_package() -> (Vec<u8>, TrustedPublisher) {
    package_with_tools(true)
}
fn package_with_tools(with_tools: bool) -> (Vec<u8>, TrustedPublisher) {
    package_with_options(with_tools, false)
}
pub(super) fn http_package() -> (Vec<u8>, TrustedPublisher) {
    package_with_options(false, true)
}
fn package_with_options(with_tools: bool, with_network: bool) -> (Vec<u8>, TrustedPublisher) {
    package_variant(with_tools, with_network, "1.0.0", 0)
}
/// Another release of the same App; `schema` > 0 declares data migrations.
pub(super) fn release_package(
    version: &str,
    schema: u32,
    with_network: bool,
) -> (Vec<u8>, TrustedPublisher) {
    package_variant(false, with_network, version, schema)
}
fn package_variant(
    with_tools: bool,
    with_network: bool,
    version: &str,
    schema: u32,
) -> (Vec<u8>, TrustedPublisher) {
    package_build(with_tools, with_network, version, schema, false, false)
}
/// An App that also declares the incoming event `received` (a generator
/// occurrence, or just `text`).
pub(super) fn inbox_package() -> (Vec<u8>, TrustedPublisher) {
    package_build(false, false, "1.0.0", 0, true, false)
}
pub(super) fn host_package() -> (Vec<u8>, TrustedPublisher) {
    package_build(false, false, "1.0.0", 0, false, true)
}
fn package_build(
    with_tools: bool,
    with_network: bool,
    version: &str,
    schema: u32,
    incoming: bool,
    clipboard_write: bool,
) -> (Vec<u8>, TrustedPublisher) {
    package_build_ui(
        with_tools,
        with_network,
        version,
        schema,
        incoming,
        clipboard_write,
        false,
    )
}
pub(super) fn browser_tool_package() -> (Vec<u8>, TrustedPublisher) {
    package_build_ui(true, false, "1.0.0", 0, false, false, true)
}
fn package_build_ui(
    with_tools: bool,
    with_network: bool,
    version: &str,
    schema: u32,
    incoming: bool,
    clipboard_write: bool,
    browser_ui: bool,
) -> (Vec<u8>, TrustedPublisher) {
    let mut manifest: Manifest=serde_json::from_value(json!({
        "schema":"chariox.app.v1","appId":"com.example.state","version":version,
        "publisher":{"id":"com.example","keyId":"state-key","name":"Developer"},
        "sdkVersion":chariox_app_package::SUPPORTED_SDK_VERSION,"appContractVersion":1,"minKernelProtocol":crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
        "resourcePolicy":"chariox.app.resources.v1","runtime":{"engine":"node","entry":"runtime/main.js"},
        "ui":{"entry":"ui/index.html"},"events":"schemas/events.json","capabilities":{}
    })).unwrap();
    if clipboard_write {
        manifest.capabilities.clipboard = vec![chariox_app_package::ClipboardAccess::Write];
    }
    if with_network {
        manifest.capabilities.network = vec![chariox_app_package::NetworkDestination {
            origin: "https://api.example.com".into(),
            methods: vec![
                chariox_app_package::HttpMethod::Get,
                chariox_app_package::HttpMethod::Post,
            ],
        }];
    }
    let mut files = BTreeMap::from([
        (
            "runtime/main.js".into(),
            b"export default function register() {}".to_vec(),
        ),
        (
            "ui/index.html".into(),
            b"<!doctype html><title>State fixture</title>".to_vec(),
        ),
        ("schemas/events.json".into(), {
            let mut events = vec![json!({
                "name":"changed","direction":"outgoing","schemaVersion":1,
                "payloadSchema":{"type":"object","additionalProperties":false,
                    "required":["text"],"properties":{"text":{"type":"string"}}}
            })];
            if incoming {
                events.push(json!({"name":"received","direction":"incoming",
                        "schemaVersion":1,"payloadSchema":{"type":"object","additionalProperties":false,
                        "properties":{"text":{"type":"string"},"source":{},"occurred_at":{"type":"string"},
                            "metadata":{},"artifacts":{"type":"array"},"reply_context":{}}}}));
            }
            serde_json::to_vec(&json!({ "events": events })).unwrap()
        }),
    ]);
    if schema > 0 {
        manifest.migrations = Some(chariox_app_package::Migrations {
            directory: "migrations".into(),
            target_version: schema,
            steps: (0..schema)
                .map(|from| chariox_app_package::Migration {
                    from,
                    to: from + 1,
                    entry: format!("migrations/{}.js", from + 1),
                })
                .collect(),
        });
        for step in 1..=schema {
            files.insert(
                format!("migrations/{step}.js"),
                b"export default function migrate() {}".to_vec(),
            );
        }
    }
    if incoming {
        // It may act through a granted dummy connection.
        manifest.capabilities.connections = vec![chariox_app_package::ConnectionAccess {
            generator: "dev.chariox.dummy".into(),
            actions: vec!["dummy.ping".into()],
        }];
    }
    if with_tools {
        manifest.tools = Some("schemas/tools.json".into());
        files.insert("schemas/tools.json".into(), serde_json::to_vec(&json!({"tools":[{
            "name":"echo", "description":"Fixed native tool fixture",
            "inputSchema":{"type":"object","additionalProperties":false,"required":["text"],"properties":{"text":{"type":"string"}}},
            "outputSchema":{"type":"object","additionalProperties":false,"required":["ok"],"properties":{"ok":{"type":"boolean"}}}
        }]})).unwrap());
    }
    if browser_ui {
        files.insert("ui/index.html".into(), b"<!doctype html><title>MD integration App</title><h1>Kernel-hosted App</h1><p id='result'>Loading</p><form id='action'><input id='message' aria-label='Message'><button>Call App</button></form><p id='keyboard'>No keyboard action</p><script src='app.js'></script>".to_vec());
        files.insert("ui/app.js".into(), b"window.chariox.call('echo',{text:'kernel browser page'}).then(result=>document.getElementById('result').textContent='App channel '+JSON.stringify(result)).catch(()=>document.getElementById('result').textContent='App channel refused');document.getElementById('action').addEventListener('submit',event=>{event.preventDefault();window.chariox.call('echo',{text:document.getElementById('message').value}).then(result=>document.getElementById('keyboard').textContent='Keyboard App channel '+JSON.stringify(result));});".to_vec());
    }
    let bytes = pack(
        &manifest,
        &files,
        &SigningKey::from_bytes(&[71; 32]),
        &Limits::default(),
    )
    .unwrap();
    (bytes, publisher())
}
fn budget() -> AppOperationBudget {
    AppOperationBudget::fixture(
        tokio::time::Instant::now() + Duration::from_secs(30),
        || false,
    )
}
fn put(value: i32) -> AppStateOperation {
    AppStateOperation::Transaction {
        changes: StateChanges::new(
            0,
            vec![],
            vec![StateWrite::Put {
                key: "count".into(),
                value: json!(value),
            }],
        )
        .unwrap(),
        occurrences: Vec::new(),
        wakes: Vec::new(),
        wakes_count_as_use: false,
    }
}
fn read() -> AppStateOperation {
    AppStateOperation::Get {
        key: "count".into(),
    }
}

#[test]
fn state_writer_uses_its_own_connection_and_persists_verified_values() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let catalog = catalog(&store);
    let held_reader = store.connection.lock().unwrap();
    let worker_store = store.clone();
    let worker_catalog = Arc::clone(&catalog);
    let (send, receive) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _ =
            send.send(worker_store.execute_app_state("alice", worker_catalog, put(7), budget()));
    });
    let result = receive.recv_timeout(Duration::from_secs(5));
    drop(held_reader);
    worker.join().unwrap();
    assert_eq!(
        result.unwrap().unwrap(),
        AppStateOutcome::Transaction {
            revision: 1,
            receipts: Vec::new()
        }
    );
    assert_eq!(
        store
            .execute_app_state("alice", Arc::clone(&catalog), read(), budget())
            .unwrap(),
        AppStateOutcome::Value(Some(StateRecord {
            value: json!(7),
            version: 1
        }))
    );
    assert!(store
        .execute_app_state("bob", Arc::clone(&catalog), put(8), budget())
        .is_err());
    drop(store);
    let reopened = fixture.open();
    assert_eq!(
        reopened
            .execute_app_state("alice", catalog, read(), budget())
            .unwrap(),
        AppStateOutcome::Value(Some(StateRecord {
            value: json!(7),
            version: 1
        }))
    );
}

#[test]
fn revoked_signer_blocks_state_reads_and_writes_without_changing_generation_or_data() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let catalog = catalog(&store);
    store
        .execute_app_state("alice", Arc::clone(&catalog), put(7), budget())
        .unwrap();
    store
        .mutate_app_publisher(
            "alice",
            AppPublisherMutation::Revoke {
                publisher_id: "com.example".into(),
                key_id: "state-key".into(),
                expected_revision: 1,
                decision: decision("revoke"),
                now_ms: 10,
            },
        )
        .unwrap();
    assert!(store
        .execute_app_state("alice", Arc::clone(&catalog), read(), budget())
        .is_err());
    assert!(store
        .execute_app_state("alice", Arc::clone(&catalog), put(8), budget())
        .is_err());
    assert_eq!(
        store
            .get_app_installation("alice", "installed")
            .unwrap()
            .generation,
        1
    );
    store
        .mutate_app_publisher(
            "alice",
            AppPublisherMutation::Enroll {
                publisher: publisher(),
                expected_revision: 2,
                decision: decision("reenroll"),
                now_ms: 11,
            },
        )
        .unwrap();
    assert!(store
        .execute_app_state("alice", Arc::clone(&catalog), read(), budget())
        .is_err());
    let connection = store.connection.lock().unwrap();
    let saved: (String,i64)=connection.query_row("SELECT value_json,version FROM app_state_values WHERE installation_id='installed' AND key='count'",[],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
    assert_eq!(saved, ("7".into(), 1));
}

#[test]
fn failed_state_head_publication_rolls_back_values_and_writer_continues() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let catalog = catalog(&store);
    let connection = Connection::open(store.path()).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_app_state_head BEFORE INSERT ON app_state_heads BEGIN SELECT RAISE(ABORT,'fixture write failure'); END;").unwrap();
    assert!(matches!(
        store.execute_app_state("alice", Arc::clone(&catalog), put(1), budget()),
        Err(AppStateError::State(StateError::Database(_)))
    ));
    assert_eq!(
        store
            .execute_app_state("alice", Arc::clone(&catalog), read(), budget())
            .unwrap(),
        AppStateOutcome::Value(None)
    );
    connection
        .execute_batch("DROP TRIGGER fail_app_state_head;")
        .unwrap();
    assert_eq!(
        store
            .execute_app_state("alice", catalog, put(2), budget())
            .unwrap(),
        AppStateOutcome::Transaction {
            revision: 1,
            receipts: Vec::new()
        }
    );
}

#[test]
fn cancelled_work_waiting_in_the_writer_queue_never_starts_a_state_change() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let fixture = Fixture::new();
    let store = fixture.open();
    let catalog = catalog(&store);
    let mut blocker = Connection::open(store.path()).unwrap();
    let held = blocker
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let (started, receive_started) = mpsc::channel();
    let checks = AtomicUsize::new(0);
    let first_budget = AppOperationBudget::fixture(
        tokio::time::Instant::now() + Duration::from_secs(30),
        move || {
            if checks.fetch_add(1, Ordering::SeqCst) == 1 {
                let _ = started.send(());
            }
            false
        },
    );
    let first_store = store.clone();
    let first_catalog = Arc::clone(&catalog);
    let first = std::thread::spawn(move || {
        first_store.execute_app_state("alice", first_catalog, put(7), first_budget)
    });
    // The writer has dequeued its first request and is about to wait for the
    // actual SQLite lock. Hold that lock while adding and cancelling the next.
    let started = receive_started.recv_timeout(Duration::from_secs(5));
    if started.is_err() {
        drop(held);
        first.join().unwrap().unwrap();
        panic!("writer did not start within fixture deadline");
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cancelled);
    let (response, receive) = mpsc::channel();
    store
        .writer
        .enqueue(DurableWriterRequest::AppState(Box::new(AppStateRequest {
            wake_changed: store.app_wake_changed.clone(),
            owner: "alice".into(),
            catalog: Arc::clone(&catalog),
            operation: put(9),
            budget: AppOperationBudget::fixture(
                tokio::time::Instant::now() + Duration::from_secs(30),
                move || flag.load(Ordering::Acquire),
            ),
            response,
        })))
        .unwrap();
    cancelled.store(true, Ordering::Release);
    drop(held);
    assert_eq!(
        first.join().unwrap().unwrap(),
        AppStateOutcome::Transaction {
            revision: 1,
            receipts: Vec::new()
        }
    );
    assert!(matches!(
        receive.recv_timeout(Duration::from_secs(5)).unwrap(),
        Err(AppStateError::Stopped(AppOperationStopped::Cancelled))
    ));
    assert_eq!(
        store
            .execute_app_state("alice", catalog, read(), budget())
            .unwrap(),
        AppStateOutcome::Value(Some(StateRecord {
            value: json!(7),
            version: 1
        }))
    );
}

#[test]
fn expired_after_sqlite_admission_does_not_change_state() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let fixture = Fixture::new();
    let store = fixture.open();
    let catalog = catalog(&store);
    let mut blocker = Connection::open(store.path()).unwrap();
    let held = blocker
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let (started, receive_started) = mpsc::channel();
    let checks = AtomicUsize::new(0);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    let request_budget = AppOperationBudget::fixture(deadline, move || {
        if checks.fetch_add(1, Ordering::SeqCst) == 1 {
            let _ = started.send(());
        }
        false
    });
    let writer_store = store.clone();
    let writer_catalog = Arc::clone(&catalog);
    let writer = std::thread::spawn(move || {
        writer_store.execute_app_state("alice", writer_catalog, put(1), request_budget)
    });
    let started = receive_started.recv_timeout(Duration::from_secs(5));
    if started.is_err() {
        drop(held);
        let _ = writer.join();
        panic!("writer did not reach SQLite admission within fixture deadline");
    }
    // This is a real SQLite wait. The kernel's second expiry check must reject
    // after BEGIN succeeds, even though the writer had admitted a live request.
    std::thread::sleep(
        deadline.saturating_duration_since(tokio::time::Instant::now()) + Duration::from_millis(5),
    );
    drop(held);
    assert!(matches!(
        writer.join().unwrap(),
        Err(AppStateError::Stopped(AppOperationStopped::Deadline))
    ));
    assert_eq!(
        store
            .execute_app_state("alice", catalog, read(), budget())
            .unwrap(),
        AppStateOutcome::Value(None)
    );
}

#[test]
fn schedule_operations_commit_wakes_that_the_writer_reports_due_and_completes() {
    use crate::durable_state::app_wakes::{AppWakeOperation, AppWakeOutcome};
    use chariox_app_runtime::managed_state::{Wake, WakeChange};
    let fixture = Fixture::new();
    let store = fixture.open();
    let catalog = catalog(&store);
    let set = |id: &str, due_at_ms: u64| {
        WakeChange::Set(Wake {
            id: id.into(),
            due_at_ms,
            revision: "r1".into(),
        })
    };
    let outcome = store
        .execute_app_state(
            "alice",
            Arc::clone(&catalog),
            // Armed during a tool call: delivering it counts as use.
            AppStateOperation::Schedule {
                wakes: vec![set("later", 5_000), set("soon", 1_000)],
                wakes_count_as_use: false,
            }
            .armed_during_use(true),
            budget(),
        )
        .unwrap();
    let AppStateOutcome::Wakes(wakes) = outcome else {
        panic!("schedule returns the installation's wakes");
    };
    assert_eq!(
        wakes.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(),
        ["soon", "later"]
    );
    assert!(!store.has_due_app_wakes("alice", "installed", 999));
    assert!(store.has_due_app_wakes("alice", "installed", 1_000));
    assert!(!store.has_due_app_wakes("bob", "installed", 1_000));
    // Another owner cannot schedule into this installation.
    assert!(store
        .execute_app_state(
            "bob",
            Arc::clone(&catalog),
            AppStateOperation::Schedule {
                wakes: vec![set("forged", 1)],
                wakes_count_as_use: false,
            },
            budget(),
        )
        .is_err());
    // A state transaction commits its wake changes atomically with its writes.
    let mut transaction = put(1);
    if let AppStateOperation::Transaction { wakes, .. } = &mut transaction {
        wakes.push(WakeChange::Cancel { id: "later".into() });
    }
    store
        .execute_app_state("alice", Arc::clone(&catalog), transaction, budget())
        .unwrap();
    let AppWakeOutcome::Due(due) = store
        .app_wakes(AppWakeOperation::Due {
            now_ms: 10_000,
            limit: 8,
        })
        .unwrap()
    else {
        panic!("due wakes");
    };
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].wake.id, "soon");
    assert_eq!(due[0].owner_id, "alice");
    assert!(due[0].counts_as_use);
    // A failed delivery is retried later, and the App's log says why.
    store
        .app_wakes(AppWakeOperation::Failed {
            wake: due[0].clone(),
            now_ms: 10_000,
            reason: "app_handler_failed: INVALID_ARGUMENT: bad occurrence".into(),
        })
        .unwrap();
    let logs = store
        .app_logs("alice", &due[0].installation_id, 0, 8)
        .unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].message, "A due wake failed; it will be retried");
    assert_eq!(logs[0].fields["wake_id"], "soon");
    assert_eq!(logs[0].fields["attempt"], 1);
    assert_eq!(logs[0].fields["kernel"], true);
    assert!(logs[0].fields["reason"]
        .as_str()
        .unwrap()
        .contains("bad occurrence"));
    assert_eq!(
        store
            .app_wakes(AppWakeOperation::Due {
                now_ms: 10_000,
                limit: 8
            })
            .unwrap(),
        AppWakeOutcome::Due(Vec::new())
    );
    store
        .app_wakes(AppWakeOperation::Delivered(due[0].clone()))
        .unwrap();
    assert_eq!(
        store
            .app_wakes(AppWakeOperation::Due {
                now_ms: u64::MAX >> 12,
                limit: 8
            })
            .unwrap(),
        AppWakeOutcome::Due(Vec::new())
    );
}

fn failed_wake_settlement_after_change(replaced: bool) {
    use crate::durable_state::app_wakes::{AppWakeOperation, AppWakeOutcome};
    use chariox_app_runtime::managed_state::{Wake, WakeChange};

    // Both retry and final-attempt deletion must identify an obsolete delivery.
    for attempts in [0, 7] {
        let fixture = Fixture::new();
        let store = fixture.open();
        let catalog = catalog(&store);
        store
            .execute_app_state(
                "alice",
                Arc::clone(&catalog),
                AppStateOperation::Schedule {
                    wakes: vec![WakeChange::Set(Wake {
                        id: "scheduled".into(),
                        due_at_ms: 1_000,
                        revision: "old".into(),
                    })],
                    wakes_count_as_use: false,
                },
                budget(),
            )
            .unwrap();
        let AppWakeOutcome::Due(mut due) = store
            .app_wakes(AppWakeOperation::Due {
                now_ms: 1_000,
                limit: 1,
            })
            .unwrap()
        else {
            panic!("due wake");
        };
        let mut delivered = due.pop().unwrap();
        let installation = delivered.installation_id.clone();
        delivered.attempts = attempts;
        let change = if replaced {
            WakeChange::Set(Wake {
                id: "scheduled".into(),
                due_at_ms: 2_000,
                revision: "new".into(),
            })
        } else {
            WakeChange::Cancel {
                id: "scheduled".into(),
            }
        };
        store
            .execute_app_state(
                "alice",
                Arc::clone(&catalog),
                AppStateOperation::Schedule {
                    wakes: vec![change],
                    wakes_count_as_use: false,
                },
                budget(),
            )
            .unwrap();
        store
            .app_wakes(AppWakeOperation::Failed {
                wake: delivered,
                now_ms: 1_001,
                reason: "handler failed".into(),
            })
            .unwrap();
        let logs = store.app_logs("alice", &installation, 0, 8).unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(
            logs[0].message,
            "A failed wake was cancelled or replaced; no retry was scheduled"
        );
        assert_eq!(logs[0].fields["revision"], "old");
        assert_eq!(logs[0].fields["due_at_ms"], 1_000);
        let AppWakeOutcome::Due(current) = store
            .app_wakes(AppWakeOperation::Due {
                now_ms: 2_000,
                limit: 8,
            })
            .unwrap()
        else {
            panic!("due wake");
        };
        if replaced {
            assert_eq!(current.len(), 1);
            assert_eq!(current[0].wake.revision, "new");
            assert_eq!(current[0].wake.due_at_ms, 2_000);
            assert_eq!(current[0].attempts, 0);
        } else {
            assert!(current.is_empty());
        }
    }
}

#[test]
fn failed_wake_settlement_after_cancellation_is_obsolete() {
    failed_wake_settlement_after_change(false);
}

#[test]
fn failed_wake_settlement_after_replacement_is_obsolete() {
    failed_wake_settlement_after_change(true);
}
