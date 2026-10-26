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
    assert_eq!(params["tools"].as_array().unwrap().len(), 11);
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
    Path::new(env!("CARGO_BIN_EXE_chariox-app-wasm-worker"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires real source App; use install-drill.sh"]
async fn installed_component_launch_restart_cache_hit_and_eviction() {
    let installed = Installed::new();
    echo(installed.prepare(worker()).unwrap()).await;
    let runtime = WasmRuntime::open(worker()).unwrap();
    let source = installed.lease().verified_component().unwrap();
    let cache = AotCache::open_or_create(&installed.database).unwrap();
    let prepared = cache
        .prepare(&source, runtime.identity(), |_| {
            panic!("restart must hit the cache")
        })
        .unwrap();
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
    let derived = cache
        .prepare(&source, new.identity(), |component| new.compile(component))
        .unwrap();
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
    let prepared = cache
        .prepare(&source, runtime.identity(), |component| {
            runtime.compile(component)
        })
        .unwrap();
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
    fs::write(&path, bad).unwrap();
    assert!(matches!(
        cache.prepare(&source, runtime.identity(), |_| panic!(
            "bad native code must be refused"
        )),
        Err(AotError::Integrity)
    ));
    fs::write(&path, original).unwrap();
    let mut stale = runtime.identity().clone();
    stale.worker_build.push_str("-stale");
    let stale_path = cache_root.join(&stale.key(source.digest()).id()[7..]);
    fs::copy(&path, &stale_path).unwrap();
    assert!(matches!(
        cache.prepare(&source, &stale, |_| panic!(
            "stale native code must be refused"
        )),
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
