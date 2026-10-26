use chariox_app_wasm_worker::engine::check;
use chariox_app_wit::APP_WORLD;
mod support;

#[test]
fn r1_forward_vector_reports_engine_incompatibility() {
    let bytes = support::vector(include_str!("../../app-wit/tests/fixtures/v1.wit"), false);
    chariox_app_wit::check_component(&bytes, APP_WORLD).unwrap();
    let error = check(&bytes, APP_WORLD).unwrap_err();
    assert!(format!("{error:#}").contains("component model forward feature"));
}

#[test]
fn r1_async_vector_without_unused_forward_compiles_in_the_real_engine() {
    let bytes = support::vector(include_str!("../../app-wit/tests/fixtures/v1.wit"), true);
    check(&bytes, APP_WORLD).unwrap();
}

#[tokio::test]
async fn r1_vectors_link_and_execute_their_dummy_export_in_the_real_worker() {
    use chariox_app_wasm_worker::{
        ipc::Ipc,
        limits::Limits,
        worker::{AppWorker, RetryPolicy, WorkerEngine},
    };
    // Dummy fixture exports intentionally trap. Missing WASI implementations
    // are explicit trapping test bindings; production never uses this linker.
    let bytes = support::vector(include_str!("../../app-wit/tests/fixtures/v1.wit"), true);
    let engine = WorkerEngine::new().unwrap();
    let component = engine.compile(&bytes, APP_WORLD).unwrap();
    let mut linker = AppWorker::linker(engine.engine()).unwrap();
    linker.define_unknown_imports_as_traps(&component).unwrap();
    let worker = AppWorker::with_linker(
        engine,
        component,
        linker,
        Limits::default(),
        RetryPolicy::default(),
    )
    .unwrap();
    let (stream, _peer) = tokio::net::UnixStream::pair().unwrap();
    let mut transport = Ipc::start(stream, "1".into()).unwrap();
    let failure = worker
        .run(transport.ipc.clone(), &mut transport.incoming)
        .await
        .unwrap_err();
    assert!(format!("{failure:#}").contains("unreachable"));
}

#[test]
fn precompile_uses_admission_and_the_same_engine() {
    use chariox_app_wasm_worker::worker::WorkerEngine;
    let engine = WorkerEngine::new().unwrap();
    let bytes = support::vector(include_str!("../../app-wit/tests/fixtures/v1.wit"), true);
    assert!(!engine.precompile(&bytes, APP_WORLD).unwrap().is_empty());
    assert!(engine
        .precompile(&bytes, chariox_app_wit::MIGRATION_WORLD)
        .is_err());
    assert!(engine.precompile(b"not a component", APP_WORLD).is_err());
}

#[test]
fn check_and_precompile_cli_never_replace_existing_native_artifacts() {
    use std::{fs, process::Command};
    let directory =
        std::env::temp_dir().join(format!("chariox-worker-precompile-{}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
    let _cleanup = Cleanup(directory.clone());
    let component = directory.join("app.wasm");
    let output = directory.join("app.cwasm");
    fs::write(
        &component,
        support::vector(include_str!("../../app-wit/tests/fixtures/v1.wit"), true),
    )
    .unwrap();
    let launch = directory.join("launch.json");
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../app-package/tests/vectors/v2/package.json"
    ))
    .unwrap();
    fs::write(&launch,serde_json::json!({"component":component,"generation":"1","installation_id":"missing-descriptors","release_digest":format!("sha256:{}","0".repeat(64)),"manifest":fixture["shared"]["manifest"]}).to_string()).unwrap();
    let binary = env!("CARGO_BIN_EXE_chariox-app-wasm-worker");
    let missing = Command::new(binary)
        .args(["serve", launch.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("supervisor descriptor 4"));
    let checked = Command::new(binary)
        .args(["check", APP_WORLD, component.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        checked.status.success(),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let precompiled = Command::new(binary)
        .args([
            "precompile",
            APP_WORLD,
            component.to_str().unwrap(),
            output.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        precompiled.status.success(),
        "{}",
        String::from_utf8_lossy(&precompiled.stderr)
    );
    let artifact = fs::read(&output).unwrap();
    assert!(!artifact.is_empty());
    let duplicate = Command::new(binary)
        .args([
            "precompile",
            APP_WORLD,
            component.to_str().unwrap(),
            output.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!duplicate.status.success());
    assert_eq!(fs::read(&output).unwrap(), artifact);
    fs::write(&component, b"not wasm").unwrap();
    let refused = directory.join("invalid.cwasm");
    let invalid = Command::new(binary)
        .args([
            "precompile",
            APP_WORLD,
            component.to_str().unwrap(),
            refused.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    assert!(!refused.exists());
}

#[tokio::test]
async fn startup_is_bounded_before_the_app_can_announce_ready() {
    use chariox_app_wasm_worker::{
        ipc::Ipc,
        limits::Limits,
        worker::{AppWorker, RetryPolicy, WorkerEngine},
    };
    let engine = WorkerEngine::new().unwrap();
    let component = engine.compile(&support::startup_loop(), APP_WORLD).unwrap();
    let worker = AppWorker::new(
        engine,
        component,
        Limits {
            call_ms: 30,
            slice_ms: 1000,
            ..Limits::default()
        },
        RetryPolicy::default(),
    )
    .unwrap();
    let (stream, peer) = tokio::net::UnixStream::pair().unwrap();
    let mut transport = Ipc::start(stream, "1".into()).unwrap();
    let error = worker
        .run(transport.ipc.clone(), &mut transport.incoming)
        .await
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("interrupt")
            || format!("{error:#}").contains("APP_STARTUP_TIMEOUT")
    );
    let mut peer = chariox_app_runtime::wire::Channel::new(
        peer,
        "1".into(),
        chariox_app_runtime::wire::Sender::Worker,
    )
    .unwrap();
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(20),
            peer.receive(std::time::Duration::from_secs(1))
        )
        .await
        .is_err(),
        "startup must not publish worker.ready"
    );
}

#[test]
fn r1_full_allowlist_and_migration_vectors_compile_in_the_pinned_engine() {
    let app = support::vector(
        "package fixture:full; world probe { include chariox:app/app-admission@1.0.0; }",
        true,
    );
    check(&app, APP_WORLD).unwrap();
    let migration=support::vector("package fixture:migration; world probe { include chariox:app/app-migrations-admission@1.0.0; }",true);
    check(&migration, chariox_app_wit::MIGRATION_WORLD).unwrap();
}
