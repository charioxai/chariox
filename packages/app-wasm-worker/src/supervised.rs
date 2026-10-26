//! Only the supervisor's FD5 configuration, FD6 portable component and FD7
//! locally derived cache entry reach this path. All close before guest entry.
use crate::{
    cli,
    worker::{AppWorker, RetryPolicy, WorkerEngine},
};
use chariox_app_runtime::{
    release_store::aot_cache::VerifiedAot, worker_process::wasm_config::WasmLaunch,
};
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, os::fd::FromRawFd};

fn descriptor(number: i32) -> wasmtime::Result<File> {
    let copy = unsafe { libc::fcntl(number, libc::F_DUPFD_CLOEXEC, 8) };
    wasmtime::ensure!(copy >= 8, "supervisor descriptor {number} is missing");
    unsafe {
        libc::close(number);
    }
    Ok(unsafe { File::from_raw_fd(copy) })
}
fn read(number: i32, maximum: usize) -> wasmtime::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    descriptor(number)?
        .take(maximum as u64 + 1)
        .read_to_end(&mut bytes)?;
    wasmtime::ensure!(bytes.len() <= maximum, "supervisor input exceeds limit");
    Ok(bytes)
}
pub(crate) fn serve() -> wasmtime::Result<()> {
    let launch: WasmLaunch = serde_json::from_slice(&read(5, 256 * 1024)?)?;
    launch
        .manifest
        .validate(&chariox_app_package::Limits::default())?;
    wasmtime::ensure!(
        launch.manifest.world == chariox_app_wit::APP_WORLD,
        "backend requires App world"
    );
    let bytes = read(6, chariox_app_wit::MAX_COMPONENT_BYTES)?;
    wasmtime::ensure!(
        format!("sha256:{:x}", Sha256::digest(&bytes)) == launch.cache_key.component_digest,
        "trusted component digest mismatch"
    );
    chariox_app_wit::check_component(&bytes, &launch.manifest.world)?;
    let engine = WorkerEngine::new()?;
    let identity = crate::engine::runtime_identity(engine.engine())?;
    wasmtime::ensure!(
        identity.key(&launch.cache_key.component_digest) == launch.cache_key,
        "stale worker/engine AOT key"
    );
    let artifact = VerifiedAot::from_supervisor_descriptor(descriptor(7)?, &launch.cache_key)?;
    cli::ready(
        &launch.generation,
        &launch.installation_id,
        &launch.release_digest,
    )?;
    // SAFETY: the supervisor creates native code in its separate private cache
    // from the reverified component. The inherited descriptor is checked again
    // for the exact build/engine/CPU key and byte integrity above. No uploaded
    // native code or caller path is accepted here.
    let component =
        unsafe { wasmtime::component::Component::deserialize(engine.engine(), artifact.bytes())? };
    let worker = AppWorker::new(
        engine,
        component,
        crate::limits::Limits::default(),
        RetryPolicy::from_manifest(&launch.manifest),
    )?;
    let stream = cli::inherited_stream(3)?;
    stream.set_nonblocking(true)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let mut transport =
            crate::ipc::Ipc::start(tokio::net::UnixStream::from_std(stream)?, launch.generation)?;
        worker
            .run(transport.ipc.clone(), &mut transport.incoming)
            .await
    })
}
