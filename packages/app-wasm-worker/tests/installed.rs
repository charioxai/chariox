//! Real portable source App, release/trust registry, cache and native supervisor.
//! The separate client drill also exercises the kernel's installation service.
#[path = "support/package.rs"]
mod package;
use chariox_app_package::{verify_app, VerificationPolicy, VerifiedApp};
use chariox_app_runtime::{
    installation::{
        CapabilityApproval, CapabilityDecision, InstallationRegistry, StageTrustBinding,
        VerifiedInstallCandidate,
    },
    publisher_trust::{PublisherTrustRegistry, TrustDecision, TrustedPublisherSnapshot},
    release_store::{
        aot_cache::{AotCache, AotError},
        ReleaseStore, StageBudget,
    },
    wire::{Channel, Message, Outcome, Success, WIRE_VERSION},
    worker_process::{wasm::WasmRuntime, PreparedWorker, WorkerLimits, WorkerProcess},
};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::Duration,
};

struct Installed {
    root: PathBuf,
    database: PathBuf,
    binding: StageTrustBinding,
    trust: TrustedPublisherSnapshot,
    release_path: PathBuf,
}
impl Installed {
    fn new() -> Self {
        let root =
            PathBuf::from(std::env::var("CHARIOX_WORKER_DRILL_DIR").expect("use install-drill.sh"))
                .join(format!(
                    "installed-{}-{}",
                    std::process::id(),
                    chariox_app_wasm_worker::ipc::wall_ms()
                ));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let database = root.join("kernel.db");
        let core =
            fs::read(std::env::var("CHARIOX_WORKER_DRILL_CORE").expect("build source App first"))
                .unwrap();
        let (bytes, publisher) = package::package(&core);
        let verified = verify_app(
            &bytes,
            &VerificationPolicy::new(619, vec![publisher.clone()]),
        )
        .unwrap();
        let mut connection = Connection::open(&database).unwrap();
        InstallationRegistry::new(&mut connection)
            .initialize()
            .unwrap();
        let mut publishers = PublisherTrustRegistry::new(&mut connection);
        publishers.initialize().unwrap();
        publishers
            .enroll(
                "owner",
                &publisher,
                0,
                &TrustDecision {
                    decision_id: "enroll".into(),
                    authority_ref: "drill-owner".into(),
                },
                1,
            )
            .unwrap();
        let trust = publishers
            .trusted_publisher("owner", &publisher.publisher_id, &publisher.key_id)
            .unwrap();
        let store = ReleaseStore::open_or_create(&database).unwrap();
        let reserved = store.required_reservation(&verified, &bytes).unwrap();
        let staged = store
            .stage(
                &verified,
                &bytes,
                StageBudget {
                    max_stage_bytes: 512 << 20,
                    reserved_bytes: reserved,
                    host_reserve_bytes: 4 << 30,
                },
            )
            .unwrap();
        let candidate = VerifiedInstallCandidate::from_verified(&verified, &trust).unwrap();
        let mut registry = InstallationRegistry::new(&mut connection);
        let token = registry
            .create_and_stage_verified("source-app", "owner", &candidate, 2)
            .unwrap()
            .token;
        registry
            .decide(
                &token,
                CapabilityDecision::Approved {
                    approval: CapabilityApproval {
                        decision_id: "approve".into(),
                        authority_ref: "drill-owner".into(),
                    },
                },
                3,
            )
            .unwrap();
        registry.quiesce(&token, 4).unwrap();
        registry.mark_prepared(&token, 5).unwrap();
        registry
            .commit_verified(&token, "owner", &trust, 6)
            .unwrap();
        let binding = registry.active_trust("owner", "source-app").unwrap();
        Self {
            root,
            database,
            binding,
            trust,
            release_path: staged.path,
        }
    }
    fn lease(&self) -> chariox_app_runtime::release_store::VerifiedReleaseLease {
        let store = ReleaseStore::open_or_create(&self.database).unwrap();
        let mut archive = store
            .open_stored_archive(self.binding.package_digest())
            .unwrap();
        let bytes = archive.read_bytes().unwrap();
        let verified = verify_app(
            &bytes,
            &VerificationPolicy::new(619, vec![self.trust.publisher().clone()]),
        )
        .unwrap();
        assert_eq!(verified.package_digest(), self.binding.package_digest());
        store.lease_verified(&verified, &bytes).unwrap()
    }
    fn prepare(
        &self,
        program: &Path,
    ) -> Result<PreparedWorker, chariox_app_runtime::worker_process::WorkerError> {
        PreparedWorker::prepare_wasm(
            WasmRuntime::open(program).unwrap(),
            self.lease(),
            &self.binding,
            &self.database,
        )
    }
}
impl Drop for Installed {
    fn drop(&mut self) {
        fn writable(path: &Path) {
            if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir()) {
                fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
                for entry in fs::read_dir(path).unwrap() {
                    writable(&entry.unwrap().path());
                }
            }
        }
        writable(&self.root);
        fs::remove_dir_all(&self.root).unwrap();
    }
}
async fn echo(prepared: PreparedWorker) {
    let mut process = tokio::task::spawn_blocking(move || {
        WorkerProcess::spawn_blocking(prepared, WorkerLimits::default())
    })
    .await
    .unwrap()
    .unwrap();
    let mut channel: Channel<tokio::net::UnixStream> = process.take_sdk_channel().unwrap();
    let Message::Request {
        id, method, params, ..
    } = channel.receive(Duration::from_secs(10)).await.unwrap()
    else {
        panic!("worker.ready required")
    };
    assert_eq!(method, "worker.ready");
    assert_eq!(params["tools"].as_array().unwrap().len(), 12);
    channel
        .send(
            &Message::Request {
                version: WIRE_VERSION,
                generation: "1".into(),
                id: "precommit-health".into(),
                method: "lifecycle.dispatch".into(),
                params: json!({"event":"health_check","data":null}),
                deadline_ms: chariox_app_wasm_worker::ipc::wall_ms() + 3000,
                context: None,
            },
            Duration::from_secs(2),
        )
        .await
        .unwrap();
    let response = channel.receive(Duration::from_secs(5)).await.unwrap();
    assert!(
        matches!(response, Message::Response { ref id, outcome: Outcome::Success(_), .. } if id == "precommit-health"),
        "precommit health failed: {response:?}"
    );
    channel
        .send(
            &Message::Response {
                version: WIRE_VERSION,
                generation: "1".into(),
                id,
                outcome: Outcome::Success(Success {
                    result: Value::Null,
                }),
            },
            Duration::from_secs(2),
        )
        .await
        .unwrap();
    channel
        .send(
            &Message::Request {
                version: WIRE_VERSION,
                generation: "1".into(),
                id: "echo".into(),
                method: "tools.invoke".into(),
                params: json!({"name":"echo","input":{"portable":"component"}}),
                deadline_ms: chariox_app_wasm_worker::ipc::wall_ms() + 5000,
                context: Some(json!({"actor":{"kind":"human","id":"drill"}})),
            },
            Duration::from_secs(2),
        )
        .await
        .unwrap();
    let Message::Response {
        id,
        outcome: Outcome::Success(result),
        ..
    } = channel.receive(Duration::from_secs(10)).await.unwrap()
    else {
        panic!("source App result required")
    };
    assert_eq!(id, "echo");
    assert_eq!(result.result, json!({"portable":"component"}));
    drop(channel);
    tokio::task::spawn_blocking(move || process.shutdown_blocking())
        .await
        .unwrap()
        .unwrap();
}
fn worker() -> &'static Path {
    static WORKER: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    WORKER
        .get_or_init(|| {
            let path = PathBuf::from(std::env::var("CHARIOX_WORKER_DRILL_DIR").unwrap())
                .join(format!("installed-worker-{}", std::process::id()));
            fs::copy(env!("CARGO_BIN_EXE_chariox-app-wasm-worker"), &path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o500)).unwrap();
            path
        })
        .as_path()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires real source App; use install-drill.sh"]
async fn installed_component_launch_restart_cache_hit_and_eviction() {
    let installed = Installed::new();
    echo(installed.prepare(worker()).unwrap()).await;
    let runtime = WasmRuntime::open(worker()).unwrap();
    let source = installed.lease().verified_component().unwrap();
    let cache = AotCache::open_or_create(&installed.database).unwrap();
    let prepared = cache.prepare(&source, &runtime).unwrap();
    assert!(prepared.cache_hit);
    println!(
        "DRILL real signed .cxapp installation, native launch, echo and restart cache hit: PASS"
    );
    echo(installed.prepare(worker()).unwrap()).await;
    cache.evict(&prepared.key).unwrap();
    assert!(installed.release_path.join("envelope.cxapp").is_file());
    echo(installed.prepare(worker()).unwrap()).await;
    println!("DRILL cache eviction retains installation and recompiles source: PASS");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires real source App; use install-drill.sh"]
async fn retained_component_recompiles_after_exact_worker_build_update_offline() {
    let installed = Installed::new();
    echo(installed.prepare(worker()).unwrap()).await;
    let updated = installed.root.join("updated-worker");
    fs::copy(worker(), &updated).unwrap();
    // ELF/Mach-O ignores a non-loadable trailer; the running executable's exact
    // build digest changes, simulating a runtime replacement without a network.
    use std::io::Write;
    fs::OpenOptions::new()
        .append(true)
        .open(&updated)
        .unwrap()
        .write_all(b"R4 simulated security update\n")
        .unwrap();
    fs::set_permissions(&updated, fs::Permissions::from_mode(0o500)).unwrap();
    let old = WasmRuntime::open(worker()).unwrap();
    let new = WasmRuntime::open(&updated).unwrap();
    assert_ne!(old.identity().worker_build, new.identity().worker_build);
    let source = installed.lease().verified_component().unwrap();
    let cache = AotCache::open_or_create(&installed.database).unwrap();
    let derived = cache.prepare(&source, &new).unwrap();
    assert!(!derived.cache_hit);
    echo(installed.prepare(&updated).unwrap()).await;
    println!("DRILL offline retained-component recompile after exact worker build update: PASS");
}

#[test]
#[ignore = "requires real source App; use install-drill.sh"]
fn tampered_component_cache_and_stale_build_are_refused_before_native_load() {
    let installed = Installed::new();
    let lease = installed.lease();
    let source = lease.verified_component().unwrap();
    let runtime = WasmRuntime::open(worker()).unwrap();
    let cache = AotCache::open_or_create(&installed.database).unwrap();
    let prepared = cache.prepare(&source, &runtime).unwrap();
    let cache_root = fs::read_dir(&installed.root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("app-aot-")
        })
        .unwrap();
    let path = cache_root.join(&prepared.key.id()[7..]);
    let original = fs::read(&path).unwrap();
    let mut bad = original.clone();
    *bad.last_mut().unwrap() ^= 1;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&path, bad).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
    assert!(matches!(
        cache.prepare(&source, &runtime),
        Err(AotError::Integrity)
    ));
    let _ = original;
    cache.prepare(&source, &runtime).unwrap();
    let updated = installed.root.join("stale-worker");
    fs::copy(worker(), &updated).unwrap();
    use std::io::Write;
    fs::OpenOptions::new()
        .append(true)
        .open(&updated)
        .unwrap()
        .write_all(b"R4 stale-key probe\n")
        .unwrap();
    fs::set_permissions(&updated, fs::Permissions::from_mode(0o500)).unwrap();
    let updated = WasmRuntime::open(&updated).unwrap();
    let stale_path = cache_root.join(&updated.identity().key(source.digest()).id()[7..]);
    fs::copy(&path, &stale_path).unwrap();
    assert!(matches!(
        cache.prepare(&source, &updated),
        Err(AotError::Incompatible)
    ));
    let component = installed.release_path.join("payload/runtime/app.wasm");
    fs::set_permissions(&component, fs::Permissions::from_mode(0o600)).unwrap();
    let mut bad = fs::read(&component).unwrap();
    *bad.last_mut().unwrap() ^= 1;
    fs::write(&component, bad).unwrap();
    fs::set_permissions(&component, fs::Permissions::from_mode(0o400)).unwrap();
    assert!(lease.verified_component().is_err());
    println!("DRILL tampered component/cache and stale exact-build cache are refused before native load: PASS");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires real source App; use install-drill.sh"]
async fn worker_inode_stays_pinned_across_executable_path_replacement() {
    let installed = Installed::new();
    let program = installed.root.join("pinned-worker");
    fs::copy(worker(), &program).unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o500)).unwrap();
    let runtime = WasmRuntime::open(&program).unwrap();
    fs::rename(&program, installed.root.join("original-worker")).unwrap();
    fs::copy("/usr/bin/false", &program).unwrap();
    let source = installed.lease().verified_component().unwrap();
    assert!(!runtime.compile(&source).unwrap().is_empty());
    let prepared = PreparedWorker::prepare_wasm(
        runtime,
        installed.lease(),
        &installed.binding,
        &installed.database,
    )
    .unwrap();
    echo(prepared).await;
    let root = fs::read_dir(&installed.root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("app-wasm-state-")
        })
        .unwrap();
    let installation_root = fs::read_dir(root).unwrap().next().unwrap().unwrap().path();
    assert!(installation_root.join("data").is_dir());
    assert!(installation_root.join("tmp").is_dir());
    println!("DRILL executable path replacement cannot change identity/compile/launch inode; installation data/tmp exist: PASS");
}

#[test]
#[ignore = "requires native worker; use install-drill.sh"]
fn writable_runtime_is_refused_before_compilation() {
    let path = worker().with_file_name(format!("writable-worker-{}", std::process::id()));
    fs::copy(worker(), &path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        WasmRuntime::open(&path).is_err(),
        "owner-writable runtime must be refused"
    );
}
