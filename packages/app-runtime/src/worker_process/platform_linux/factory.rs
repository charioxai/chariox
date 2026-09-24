//! Physical preparation from sealed kernel/installer proofs. This creates no
//! permission grant: the kernel must fence the binding before and after startup.
use super::{cgroup, domain, inspection, plan, Result, WorkerError};
use crate::{
    installation::StageTrustBinding,
    release_store::VerifiedReleaseLease,
    runtime_enrollment::EnrolledRuntime,
    worker_process::{record::LaunchRecord, storage_linux, PreparedWorker},
};
use chariox_app_package::EventDirection;
use std::{
    ffi::CString,
    fs::File,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};

pub(in crate::worker_process) fn prepare(
    runtime: EnrolledRuntime,
    release: VerifiedReleaseLease,
    binding: &StageTrustBinding,
) -> Result<PreparedWorker> {
    if release.package_digest() != binding.package_digest() {
        return Err(WorkerError::Identity);
    }
    let generation = binding.token().generation;
    let installation = &binding.token().installation_id;
    let record = LaunchRecord {
        generation: generation.to_string(),
        installation: installation.clone(),
        release_digest: release.package_digest().into(),
        roots: plan::ROOTS.map(str::to_owned),
        bootstrap: bootstrap(&release)?,
        nofile: 128,
        cpu_seconds: 86400,
        heap_mib: 256,
        v8_threads: 1,
        max_file_bytes: 512 * 1024 * 1024,
    };
    record.encode()?;
    let delegation = cgroup::Delegation::open(
        &storage_linux::cgroup_path().map_err(|_| WorkerError::Preparation)?,
    )?;
    let leaf = delegation.create()?;
    let leaf_name = leaf
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(WorkerError::Preparation)?;
    let mut storage =
        storage_linux::Lease::acquire(binding.owner_id(), installation, generation, leaf_name)
            .map_err(|_| WorkerError::Preparation)?;
    storage
        .attach_code(&release, &runtime)
        .map_err(|_| WorkerError::Preparation)?;
    let [package_path, runtime_path] = storage.code_paths();
    let roots = [
        plan::Binding {
            path: package_path,
            object: storage
                .package
                .as_ref()
                .ok_or(WorkerError::Preparation)?
                .0
                .try_clone()
                .map_err(|_| WorkerError::Preparation)?,
        },
        plan::Binding {
            path: storage.data_path(),
            object: storage
                .data
                .as_ref()
                .ok_or(WorkerError::Preparation)?
                .0
                .try_clone()
                .map_err(|_| WorkerError::Preparation)?,
        },
        plan::Binding {
            path: storage.temporary_path(),
            object: storage
                .temporary
                .as_ref()
                .ok_or(WorkerError::Preparation)?
                .0
                .try_clone()
                .map_err(|_| WorkerError::Preparation)?,
        },
        plan::Binding {
            path: runtime_path,
            object: storage
                .runtime
                .as_ref()
                .ok_or(WorkerError::Preparation)?
                .0
                .try_clone()
                .map_err(|_| WorkerError::Preparation)?,
        },
    ];
    let mut libraries = Vec::new();
    for (name, destination) in libraries_for(runtime.target())? {
        let relative = format!("platform/{name}");
        let expected = runtime.file(&relative).ok_or(WorkerError::Preparation)?;
        let path = roots[3].path.join(&relative);
        let object = held_file(&path, expected)?;
        libraries.push((destination, plan::Binding { path, object }));
    }
    let entry = runtime
        .file("chariox-app-domain-entry")
        .ok_or(WorkerError::Preparation)?;
    let program_path = roots[3].path.join("chariox-app-domain-entry");
    let held_entry = held_file(&program_path, entry)?;
    let program = CString::new(program_path.as_os_str().as_encoded_bytes())
        .map_err(|_| WorkerError::Preparation)?;
    let arguments = plan::arguments(&roots, &libraries)?;
    let observer = inspection::Observer::new(
        runtime
            .file("chariox-app-worker")
            .ok_or(WorkerError::Preparation)?
            .try_clone()
            .map_err(|_| WorkerError::Preparation)?,
    )?;
    let setup = [
        leaf.processes
            .try_clone()
            .map_err(|_| WorkerError::Preparation)?,
        runtime
            .file("chariox-bwrap")
            .ok_or(WorkerError::Preparation)?
            .try_clone()
            .map_err(|_| WorkerError::Preparation)?,
    ];
    let domain = domain::Domain {
        _roots: roots,
        _libraries: libraries,
        setup,
        observer,
        storage: Some(storage),
        _runtime: Some(runtime),
        _release: Some(release),
        leaf,
    };
    Ok(PreparedWorker {
        program,
        arguments,
        record,
        _objects: vec![held_entry],
        domain: Box::new(domain),
    })
}

fn held_file(path: &Path, expected: &File) -> Result<File> {
    let file = File::options()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| WorkerError::Preparation)?;
    let actual = file.metadata().map_err(|_| WorkerError::Preparation)?;
    let expected = expected.metadata().map_err(|_| WorkerError::Preparation)?;
    if !actual.is_file() || actual.dev() != expected.dev() || actual.ino() != expected.ino() {
        return Err(WorkerError::Identity);
    }
    Ok(file)
}
fn libraries_for(target: &str) -> Result<Vec<(&'static str, String)>> {
    let (loader, loader_path, directory) = match target {
        "linux-x64" => (
            "ld-linux-x86-64.so.2",
            "/lib64/ld-linux-x86-64.so.2",
            "/lib/x86_64-linux-gnu",
        ),
        "linux-arm64" => (
            "ld-linux-aarch64.so.1",
            "/lib/ld-linux-aarch64.so.1",
            "/lib/aarch64-linux-gnu",
        ),
        _ => return Err(WorkerError::Preparation),
    };
    let mut libraries = vec![(loader, loader_path.into())];
    libraries.extend(
        [
            "libc.so.6",
            "libm.so.6",
            "libstdc++.so.6",
            "libgcc_s.so.1",
            "libpthread.so.0",
            "libdl.so.2",
            "librt.so.1",
        ]
        .into_iter()
        .map(|name| (name, format!("{directory}/{name}"))),
    );
    Ok(libraries)
}
fn bootstrap(package: &VerifiedReleaseLease) -> Result<String> {
    let config = serde_json::json!({"version":1,"entry":package.manifest().runtime.entry,
        "declarations":{"tools":package.declarations().tools.iter().map(|tool| &tool.name).collect::<Vec<_>>(),
        "incomingEvents":package.declarations().events.iter().filter(|event| matches!(event.direction, EventDirection::Incoming | EventDirection::Both)).map(|event| &event.name).collect::<Vec<_>>()},
        "startupTimeoutMs":15000});
    let config = serde_json::to_string(&config).map_err(|_| WorkerError::Preparation)?;
    Ok(format!("require('node:module').createRequire('/runtime/bootstrap.cjs')('/runtime/bootstrap.cjs').start({config});"))
}
