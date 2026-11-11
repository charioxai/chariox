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

mod migrations;

fn descriptor(number: i32) -> wasmtime::Result<File> {
    let copy = unsafe { libc::fcntl(number, libc::F_DUPFD_CLOEXEC, 8) };
    wasmtime::ensure!(copy >= 8, "supervisor descriptor {number} is missing");
    unsafe {
        libc::close(number);
    }
    Ok(unsafe { File::from_raw_fd(copy) })
}
pub(crate) fn read(number: i32, maximum: usize) -> wasmtime::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    descriptor(number)?
        .take(maximum as u64 + 1)
        .read_to_end(&mut bytes)?;
    wasmtime::ensure!(bytes.len() <= maximum, "supervisor input exceeds limit");
    Ok(bytes)
}
pub(crate) fn serve() -> wasmtime::Result<()> {
    // The executable descriptor pins the kernel-selected inode through exec.
    unsafe {
        libc::close(8);
    }
    let launch: WasmLaunch = serde_json::from_slice(&read(5, 256 * 1024)?)?;
    launch
        .manifest
        .validate(&chariox_app_package::Limits::default())?;
    wasmtime::ensure!(
        launch.manifest.world == chariox_app_wit::APP_WORLD,
        "backend requires App world"
    );
    let bytes = read(6, chariox_app_wit::MAX_COMPONENT_BYTES)?;
    // Migration components are read in full: containment closes FD9.
    wasmtime::ensure!(
        launch
            .migrations
            .iter()
            .all(|step| step.bytes <= chariox_app_wit::MAX_COMPONENT_BYTES),
        "migration component exceeds limit"
    );
    let migration_source = read(9, launch.migrations.iter().map(|step| step.bytes).sum())?;
    let artifact = VerifiedAot::from_supervisor_descriptor(
        descriptor(7)?,
        &launch.cache_key,
        &launch.cache_public_key,
    )?;
    let build = crate::engine::worker_build()?;
    let directories = launch
        .installation_root
        .as_deref()
        .map(|root| crate::wasi::InstallationDirectories::open(std::path::Path::new(root)))
        .transpose()?;
    // Every supervisor input is read and the installation's directories are
    // resolved; nothing runs as the App before R5's boundary, including the
    // engine's threads, registration and the ready reply.
    cli::contain(
        &directories
            .as_ref()
            .map(crate::wasi::InstallationDirectories::files)
            .unwrap_or_default(),
    )?;
    wasmtime::ensure!(
        format!("sha256:{:x}", Sha256::digest(&bytes)) == launch.cache_key.component_digest,
        "trusted component digest mismatch"
    );
    chariox_app_wit::check_component(&bytes, &launch.manifest.world)?;
    let engine = WorkerEngine::new()?;
    let identity = crate::engine::runtime_identity(engine.engine(), build)?;
    wasmtime::ensure!(
        identity.key(&launch.cache_key.component_digest) == launch.cache_key,
        "stale worker/engine AOT key"
    );
    cli::ready(
        &launch.generation,
        &launch.installation_id,
        &launch.release_digest,
    )?;
    // SAFETY: the supervisor creates native code in its separate private cache
    // from the reverified component. The inherited descriptor is checked again
    // for the exact build/engine/CPU key and byte integrity above. No uploaded
    // native code or caller path is accepted here.
    let component = unsafe { engine.deserialize_component(artifact.bytes())? };
    let mut worker = AppWorker::new(
        engine,
        component,
        crate::limits::Limits::default(),
        RetryPolicy::from_manifest(&launch.manifest),
    )?;
    if let Some(directories) = directories {
        worker = worker.with_directories(directories);
    }
    let stream = cli::inherited_stream(3)?;
    stream.set_nonblocking(true)?;
    let runtime = cli::runtime()?;
    runtime.block_on(async move {
        let mut transport =
            crate::ipc::Ipc::start(tokio::net::UnixStream::from_std(stream)?, launch.generation)?;
        migrations::run(
            &transport.ipc,
            std::io::Cursor::new(migration_source),
            &launch.installation_id,
            launch.migrations,
        )
        .await?;
        worker
            .run(transport.ipc.clone(), &mut transport.incoming)
            .await
    })
}
