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
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

mod domain;

pub struct WasmRuntime {
    program: PathBuf,
    _executable: File,
    identity: AotRuntimeIdentity,
}
impl WasmRuntime {
    /// The path comes from the kernel's native installation, never an App or
    /// client request. The executable hash is the exact worker-build cache key.
    pub fn open(program: &Path) -> Result<Self, AotError> {
        let mut executable = File::options()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(program)?;
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
        let output = Command::new(program)
            .arg("identity")
            .stdin(Stdio::null())
            .output()?;
        if !output.status.success() || output.stdout.len() > 16 * 1024 {
            return Err(AotError::Incompatible);
        }
        let identity: AotRuntimeIdentity =
            serde_json::from_slice(&output.stdout).map_err(|_| AotError::Incompatible)?;
        if identity.worker_build != format!("sha256:{:x}", hash.finalize()) {
            return Err(AotError::Integrity);
        }
        Ok(Self {
            program: program.to_owned(),
            _executable: executable,
            identity,
        })
    }
    pub fn beside_kernel() -> Result<Self, AotError> {
        let kernel = std::env::current_exe()?;
        Self::open(
            &kernel
                .parent()
                .ok_or(AotError::Incompatible)?
                .join("chariox-app-wasm-worker"),
        )
    }
    pub fn identity(&self) -> &AotRuntimeIdentity {
        &self.identity
    }
    pub fn compile(&self, component: &VerifiedComponent) -> Result<Vec<u8>, AotError> {
        let mut child = Command::new(&self.program)
            .args(["precompile-stdio", component.world()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        if let Err(error) = child
            .stdin
            .take()
            .ok_or(AotError::Incompatible)?
            .write_all(component.bytes())
        {
            if child.id() > 1 {
                let _ = child.kill();
            }
            let _ = child.wait();
            return Err(error.into());
        }
        let output = child.wait_with_output()?;
        if !output.status.success() {
            return Err(AotError::Compile(
                String::from_utf8_lossy(&output.stderr[..output.stderr.len().min(4096)]).into(),
            ));
        }
        Ok(output.stdout)
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
        if release.package_digest() != binding.package_digest() {
            return Err(WorkerError::Identity);
        }
        let manifest = release
            .manifest()
            .wasm()
            .ok_or(WorkerError::Preparation)?
            .clone();
        if manifest.migrations.is_some() {
            // R6 owns migration-world execution. Never pretend an App-world
            // startup has migrated user data.
            return Err(WorkerError::Storage("app_wasm_migration_unavailable"));
        }
        let component = release
            .verified_component()
            .map_err(|_| WorkerError::Preparation)?;
        let cache = AotCache::open_or_create(database).map_err(aot_error)?;
        let artifact = cache
            .prepare(&component, runtime.identity(), |component| {
                runtime.compile(component)
            })
            .map_err(aot_error)?;
        let generation = binding.token().generation.to_string();
        let installation = binding.token().installation_id.clone();
        let release_digest = release.package_digest().to_owned();
        let launch = WasmLaunch {
            generation: generation.clone(),
            installation_id: installation.clone(),
            release_digest: release_digest.clone(),
            manifest,
            cache_key: artifact.key,
        };
        let encoded = serde_json::to_vec(&launch).map_err(|_| WorkerError::Preparation)?;
        let root_name = crate::release_store::root_for_database(database)
            .map_err(|_| WorkerError::Preparation)?;
        let root_name = root_name
            .file_name()
            .and_then(OsStr::to_str)
            .ok_or(WorkerError::Preparation)?;
        let root = Dir::open_or_create_private_child(
            database.parent().ok_or(WorkerError::Preparation)?,
            OsStr::new(&root_name.replace("app-releases-", "app-wasm-state-")),
        )
        .map_err(|_| WorkerError::Preparation)?;
        let data_name = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&(binding.owner_id(), &installation))
                    .map_err(|_| WorkerError::Preparation)?
            )
        );
        let data = match root.create_child(OsStr::new(&data_name)) {
            Ok(data) => data,
            Err(crate::private_fs::FsError::Io(error))
                if error.kind() == std::io::ErrorKind::AlreadyExists =>
            {
                root.child(OsStr::new(&data_name))
                    .map_err(|_| WorkerError::Preparation)?
            }
            Err(_) => return Err(WorkerError::Preparation),
        };
        let metadata = data.0.metadata().map_err(|_| WorkerError::Preparation)?;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o7777 != 0o700 {
            return Err(WorkerError::Preparation);
        }
        if !data.try_lock().map_err(|_| WorkerError::Preparation)? {
            return Err(WorkerError::ResourceDomain);
        }
        data.sync().map_err(|_| WorkerError::Preparation)?;
        root.sync().map_err(|_| WorkerError::Preparation)?;
        let config = input_file(&root, &encoded)?;
        let source = input_file(&root, component.bytes())?;
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
        Ok(Self {
            program: CString::new(runtime.program.as_os_str().as_encoded_bytes())
                .map_err(|_| WorkerError::Preparation)?,
            arguments: vec![CString::new("serve-supervised").unwrap()],
            record,
            _objects: vec![],
            domain: Box::new(domain::WasmDomain::new(
                [config, source, artifact.file],
                data,
                runtime,
                release,
            )),
        })
    }
}

fn aot_error(error: AotError) -> WorkerError {
    eprintln!("App WASM preparation: {error}");
    WorkerError::Preparation
}
fn input_file(root: &Dir, bytes: &[u8]) -> Result<File, WorkerError> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = format!(
        ".launch-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let name = OsStr::new(&name);
    root.write_new(name, bytes)
        .map_err(|_| WorkerError::Preparation)?;
    let file = root
        .read_file(name, true)
        .map_err(|_| WorkerError::Preparation);
    let cleanup = root.remove_file(name).map_err(|_| WorkerError::Preparation);
    cleanup?;
    file
}
