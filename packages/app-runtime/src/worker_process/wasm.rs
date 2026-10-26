//! R4's verified component/cache preparation. R5 replaces the provisional
//! domain in `wasm/domain.rs` with Linux/macOS containment before release.
use super::{record::PreparedLaunch, wasm_config::WasmLaunch, PreparedWorker, WorkerError};
use crate::{
    installation::StageTrustBinding,
    private_fs::Dir,
    release_store::{
        aot_cache::{AotCache, AotError, AotRuntimeIdentity, VerifiedComponent},
        VerifiedReleaseLease,
    },
};
use sha2::{Digest, Sha256};
use std::{
    ffi::{CString, OsStr},
    fs::File,
    io::{Read, Write},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

mod compile;
mod domain;
mod snapshot;
pub mod storage;

pub struct WasmRuntime {
    executable: File,
    identity: AotRuntimeIdentity,
}
impl WasmRuntime {
    /// The path comes from the kernel's native installation, never an App or
    /// client request. The executable hash is the exact worker-build cache key.
    pub fn open(program: &Path) -> Result<Self, AotError> {
        let directory = Dir::open_owned_parent(program.parent().ok_or(AotError::Integrity)?)?;
        let mut executable =
            directory.read_file(program.file_name().ok_or(AotError::Integrity)?, true)?;
        let metadata = executable.metadata()?;
        if !metadata.is_file()
            || metadata.mode() & 0o022 != 0
            || metadata.mode() & 0o111 == 0
            || ![0, unsafe { libc::geteuid() }].contains(&metadata.uid())
        {
            return Err(AotError::Integrity);
        }
        let mut hash = Sha256::new();
        let mut buffer = [0; 64 * 1024];
        loop {
            let size = executable.read(&mut buffer)?;
            if size == 0 {
                break;
            }
            hash.update(&buffer[..size]);
        }
        let output = compile::run(
            &executable_path(std::os::fd::AsRawFd::as_raw_fd(&executable)),
            &["identity"],
            &[],
            16 * 1024,
        )?;
        let identity: AotRuntimeIdentity =
            serde_json::from_slice(&output).map_err(|_| AotError::Incompatible)?;
        if identity.worker_build != format!("sha256:{:x}", hash.finalize()) {
            return Err(AotError::Integrity);
        }
        Ok(Self {
            executable,
            identity,
        })
    }
    pub fn beside_kernel() -> Result<Self, AotError> {
        #[cfg(not(debug_assertions))]
        {
            Err(AotError::ContainmentPending)
        }
        #[cfg(debug_assertions)]
        {
            let kernel = std::env::current_exe()?;
            Self::open(
                &kernel
                    .parent()
                    .ok_or(AotError::Incompatible)?
                    .join("chariox-app-wasm-worker"),
            )
        }
    }
    pub fn identity(&self) -> &AotRuntimeIdentity {
        &self.identity
    }
    pub fn compile(&self, component: &VerifiedComponent) -> Result<Vec<u8>, AotError> {
        compile::run(
            &executable_path(std::os::fd::AsRawFd::as_raw_fd(&self.executable)),
            &["precompile-stdio", component.world()],
            component.bytes(),
            crate::release_store::aot_cache::MAX_ARTIFACT_BYTES,
        )
    }
}

impl PreparedWorker {
    /// Uses installation-derived manifest/identity and retained verified bytes.
    /// `database` is the kernel's own store, not a path in a launch request.
    pub fn prepare_wasm(
        runtime: WasmRuntime,
        release: VerifiedReleaseLease,
        binding: &StageTrustBinding,
        database: &Path,
    ) -> Result<Self, WorkerError> {
        Self::prepare_wasm_update(
            runtime,
            release,
            binding,
            database,
            None,
            Some(binding.token().generation),
        )
    }

    pub fn prepare_wasm_update(
        runtime: WasmRuntime,
        release: VerifiedReleaseLease,
        binding: &StageTrustBinding,
        database: &Path,
        migrate_from: Option<u32>,
        committed: Option<u64>,
    ) -> Result<Self, WorkerError> {
        if release.package_digest() != binding.package_digest() {
            return Err(WorkerError::Identity);
        }
        let manifest = release
            .manifest()
            .wasm()
            .ok_or(WorkerError::Preparation)?
            .clone();
        let mut migrations = Vec::new();
        let mut migration_bytes = Vec::new();
        if let Some(start) = migrate_from {
            let mut from = start;
            for step in manifest
                .migrations
                .iter()
                .flat_map(|m| &m.steps)
                .filter(|s| s.from >= start)
            {
                if step.from != from {
                    return Err(WorkerError::Preparation);
                }
                let component = release
                    .verified_migration(&step.entry)
                    .map_err(|_| WorkerError::Preparation)?;
                migrations.push(super::wasm_config::WasmMigration {
                    from: step.from,
                    to: step.to,
                    bytes: component.bytes().len(),
                    digest: component.digest().into(),
                });
                migration_bytes.extend_from_slice(component.bytes());
                from = step.to;
            }
            if migrations.is_empty() {
                return Err(WorkerError::Preparation);
            }
        }
        let component = release
            .verified_component()
            .map_err(|_| WorkerError::Preparation)?;
        let cache = AotCache::open_or_create(database).map_err(aot_error)?;
        let artifact = cache.prepare(&component, &runtime).map_err(aot_error)?;
        let generation = binding.token().generation.to_string();
        let installation = binding.token().installation_id.clone();
        let release_digest = release.package_digest().to_owned();
        let root_path = storage::root(database)?;
        let root = Dir::open_or_create_private_child(
            root_path.parent().ok_or(WorkerError::Preparation)?,
            root_path.file_name().ok_or(WorkerError::Preparation)?,
        )
        .map_err(|_| WorkerError::Preparation)?;
        let data_name = storage::installation(binding.owner_id(), &installation);
        let installation_root = match root.create_child(OsStr::new(&data_name)) {
            Ok(data) => data,
            Err(crate::private_fs::FsError::Io(error))
                if error.kind() == std::io::ErrorKind::AlreadyExists =>
            {
                root.child(OsStr::new(&data_name))
                    .map_err(|_| WorkerError::Preparation)?
            }
            Err(_) => return Err(WorkerError::Preparation),
        };
        let metadata = installation_root
            .0
            .metadata()
            .map_err(|_| WorkerError::Preparation)?;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o7777 != 0o700 {
            return Err(WorkerError::Preparation);
        }
        // R6 preopens only these existing children. The broker/snapshot data
        // handle deliberately excludes tmp and all supervisor metadata.
        let child = |name: &str| match installation_root.create_child(OsStr::new(name)) {
            Ok(dir) => Ok(dir),
            Err(crate::private_fs::FsError::Io(error))
                if error.kind() == std::io::ErrorKind::AlreadyExists =>
            {
                installation_root.child(OsStr::new(name))
            }
            Err(error) => Err(error),
        };
        let data = child("data").map_err(|_| WorkerError::Preparation)?;
        let tmp = child("tmp").map_err(|_| WorkerError::Preparation)?;
        for dir in [&data, &tmp] {
            let metadata = dir.0.metadata().map_err(|_| WorkerError::Preparation)?;
            if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o7777 != 0o700 {
                return Err(WorkerError::Preparation);
            }
            dir.sync().map_err(|_| WorkerError::Preparation)?;
        }
        installation_root
            .sync()
            .map_err(|_| WorkerError::Preparation)?;
        let installation_path = root_path.join(&data_name);
        let launch = WasmLaunch {
            generation: generation.clone(),
            installation_id: installation.clone(),
            release_digest: release_digest.clone(),
            manifest,
            installation_root: Some(
                installation_path
                    .to_str()
                    .ok_or(WorkerError::Preparation)?
                    .into(),
            ),
            cache_key: artifact.key,
            cache_public_key: cache.public_key(),
            migrations,
        };
        let encoded = serde_json::to_vec(&launch).map_err(|_| WorkerError::Preparation)?;
        if !data.try_lock().map_err(|_| WorkerError::Preparation)? {
            return Err(WorkerError::ResourceDomain);
        }
        data.sync().map_err(|_| WorkerError::Preparation)?;
        root.sync().map_err(|_| WorkerError::Preparation)?;
        let config = input_file(&root, &encoded)?;
        let source = input_file(&root, component.bytes())?;
        let migration_source = input_file(&root, &migration_bytes)?;
        eprintln!(
            "App WASM {} generation {} AOT cache {}",
            installation,
            generation,
            if artifact.cache_hit {
                "hit"
            } else {
                "compiled"
            }
        );
        let record = PreparedLaunch::Wasm {
            generation,
            installation,
            release_digest,
        };
        record.encode()?;
        let prepared = Self {
            program: CString::new(executable_path(8).as_os_str().as_encoded_bytes())
                .map_err(|_| WorkerError::Preparation)?,
            arguments: vec![CString::new("serve-supervised").unwrap()],
            record,
            _objects: vec![],
            domain: Box::new(domain::WasmDomain::new(
                [
                    config,
                    source,
                    artifact.file,
                    runtime
                        .executable
                        .try_clone()
                        .map_err(|_| WorkerError::Preparation)?,
                    migration_source,
                ],
                data,
                runtime,
                release,
            )),
        };
        let (prepared, result) = prepared.visit_private_data(|data| {
            snapshot::prepare(&installation_root, &installation_path, data, committed)
        })?;
        result?;
        Ok(prepared)
    }
}

pub fn aot_error_code(error: &AotError) -> &'static str {
    match error {
        AotError::Integrity => "app_aot_integrity",
        AotError::Incompatible => "app_aot_incompatible",
        AotError::Compile(_) => "app_aot_compile",
        AotError::Busy => "app_aot_busy",
        AotError::Limit => "app_aot_limit",
        AotError::ContainmentPending => "app_wasm_containment_pending",
        _ => "app_aot_store",
    }
}

fn input_file(root: &Dir, bytes: &[u8]) -> Result<File, WorkerError> {
    use std::{
        io::{Seek, SeekFrom},
        os::fd::{AsRawFd, FromRawFd},
    };
    #[cfg(target_os = "linux")]
    let mut file = {
        let name = CString::new("chariox-app-input").unwrap();
        let fd = unsafe {
            libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING)
        };
        if fd < 0 {
            return Err(WorkerError::Preparation);
        }
        let _ = root;
        unsafe { File::from_raw_fd(fd) }
    };
    #[cfg(target_os = "macos")]
    let mut file = {
        // Unlink before writing: a crash can leave only an empty random name,
        // never a launch config or a component and never a PID-reuse collision.
        let name = format!(".launch-{:032x}", rand::random::<u128>());
        let name = OsStr::new(&name);
        let file = root
            .create_private_file(name)
            .map_err(|_| WorkerError::Preparation)?;
        root.remove_file(name)
            .map_err(|_| WorkerError::Preparation)?;
        file
    };
    file.write_all(bytes)
        .map_err(|_| WorkerError::Preparation)?;
    file.seek(SeekFrom::Start(0))
        .map_err(|_| WorkerError::Preparation)?;
    #[cfg(target_os = "linux")]
    if unsafe {
        libc::fcntl(
            file.as_raw_fd(),
            libc::F_ADD_SEALS,
            libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL,
        )
    } != 0
    {
        return Err(WorkerError::Preparation);
    }
    Ok(file)
}

fn executable_path(descriptor: i32) -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        PathBuf::from(format!("/proc/self/fd/{descriptor}"))
    }
    #[cfg(target_os = "macos")]
    {
        PathBuf::from(format!("/dev/fd/{descriptor}"))
    }
}

fn aot_error(error: AotError) -> WorkerError {
    eprintln!("App WASM preparation failed: {error}");
    WorkerError::Storage(aot_error_code(&error))
}
