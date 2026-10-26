use crate::{
    ipc::Ipc,
    limits::Limits,
    worker::{AppWorker, RetryPolicy, WorkerEngine},
};
use serde::Deserialize;
use std::{
    fs::File,
    io::{Read, Write},
    os::{fd::FromRawFd, unix::fs::OpenOptionsExt},
    path::Path,
    time::Duration,
};

#[cfg(all(not(test), not(debug_assertions)))]
const _: () = chariox_app_runtime::assert_production_build();

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Launch {
    component: String,
    generation: String,
    installation_id: String,
    release_digest: String,
    manifest: chariox_app_package::v2::ManifestV2,
    #[serde(default)]
    limits: Limits,
}
fn bounded_read(path: impl AsRef<Path>, limit: usize) -> wasmtime::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    wasmtime::ensure!(bytes.len() <= limit, "input exceeds the {limit} byte limit");
    Ok(bytes)
}
fn inherited_stream(descriptor: i32) -> wasmtime::Result<std::os::unix::net::UnixStream> {
    // Duplicate through libc first: FromRawFd must never receive a missing fd.
    let duplicate = unsafe { libc::fcntl(descriptor, libc::F_DUPFD_CLOEXEC, 5) };
    if duplicate < 0 {
        return Err(wasmtime::Error::msg(format!(
            "worker requires inherited supervisor descriptor {descriptor}: {}",
            std::io::Error::last_os_error()
        )));
    }
    let stream = unsafe { std::os::unix::net::UnixStream::from_raw_fd(duplicate) };
    unsafe {
        libc::close(descriptor);
    }
    Ok(stream)
}
fn ready(launch: &Launch) -> wasmtime::Result<()> {
    let generation = launch.generation.parse::<u64>()?;
    wasmtime::ensure!(
        generation > 0
            && generation <= i64::MAX as u64
            && generation.to_string() == launch.generation,
        "invalid generation"
    );
    wasmtime::ensure!(
        !launch.installation_id.is_empty()
            && launch.installation_id.len() <= 128
            && launch
                .installation_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte)),
        "invalid installation id"
    );
    wasmtime::ensure!(
        launch.release_digest.len() == 71
            && launch.release_digest.starts_with("sha256:")
            && launch.release_digest[7..]
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "invalid release digest"
    );
    // FD4 belongs to the supervisor; it releases the worker only after its
    // platform containment and generation checks have completed.
    let mut stream = inherited_stream(4)?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    stream.write_all(b"CXAWR001")?;
    for value in [
        &launch.generation,
        &launch.installation_id,
        &launch.release_digest[7..],
    ] {
        stream.write_all(&(value.len() as u32).to_be_bytes())?;
        stream.write_all(value.as_bytes())?;
    }
    let mut continuation = [0u8; 8];
    stream.read_exact(&mut continuation)?;
    wasmtime::ensure!(&continuation == b"CXAWGO01", "worker continuation refused");
    Ok(())
}
pub fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("App worker: {error:#}");
            std::process::ExitCode::FAILURE
        }
    }
}
fn run() -> wasmtime::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command,world,input] if command=="check"=> {
            let engine=WorkerEngine::new()?;
            engine.compile(&bounded_read(input,chariox_app_wit::MAX_COMPONENT_BYTES)?,world)?;
            println!("compatible");
        },
        [command,world,input,output] if command=="precompile"=> {
            let engine=WorkerEngine::new()?;
            let compiled=engine.precompile(&bounded_read(input,chariox_app_wit::MAX_COMPONENT_BYTES)?,world)?;
            // R4 owns atomic cache publication and provenance. Do not overwrite
            // an existing artifact or deserialize caller-supplied native bytes.
            let mut file=File::options().write(true).create_new(true).mode(0o600).open(output)?;
            let saved=file.write_all(&compiled).and_then(|()|file.sync_all());
            if let Err(error)=saved { drop(file);std::fs::remove_file(output)?;return Err(error.into()); }
            println!("precompiled {} bytes",compiled.len());
        },
        [command,config] if command=="serve"=> {
            let launch:Launch=serde_json::from_slice(&bounded_read(config,256*1024)?)?;
            launch.limits.validate()?;
            launch.manifest.validate(&chariox_app_package::Limits::default())?;
            wasmtime::ensure!(launch.manifest.world==chariox_app_wit::APP_WORLD,"backend requires the App world");
            let bytes=bounded_read(&launch.component,chariox_app_wit::MAX_COMPONENT_BYTES)?;
            // All native inputs are read before filesystem authority is removed.
            // Neither compilation, registration nor the ready reply can precede
            // containment. R4 uses this same entry for its verified AOT bytes.
            crate::containment::enter().map_err(|error| wasmtime::Error::msg(format!("APP_SANDBOX_UNAVAILABLE: {error}")))?;
            ready(&launch)?;
            let engine=WorkerEngine::new()?;
            let component=engine.compile(&bytes,&launch.manifest.world)?;
            let worker=AppWorker::new(engine,component,launch.limits,RetryPolicy::from_manifest(&launch.manifest))?;
            let stream=inherited_stream(3)?;
            stream.set_nonblocking(true)?;
            let runtime=tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;
            runtime.block_on(async move {
                let mut transport=Ipc::start(tokio::net::UnixStream::from_std(stream)?,launch.generation)?;
                worker.run(transport.ipc.clone(),&mut transport.incoming).await
            })?;
        },
        _=>wasmtime::bail!("usage: chariox-app-wasm-worker check WORLD COMPONENT | precompile WORLD COMPONENT OUTPUT | serve SUPERVISOR_CONFIG"),
    }
    Ok(())
}
