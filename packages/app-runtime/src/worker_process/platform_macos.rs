//! macOS preparation composes the enrolled runtime, verified release and
//! private APFS storage. The signed launcher applies the default-deny Seatbelt
//! profile to these canonical roots before loading the embedded Node runtime.
use super::{record::LaunchRecord, storage_macos, PreparedWorker, ResourceDomain, WorkerError};
use crate::{
    installation::StageTrustBinding, release_store::VerifiedReleaseLease,
    runtime_enrollment::EnrolledRuntime,
};
use chariox_app_package::EventDirection;
use std::{
    ffi::CString,
    fs::File,
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    path::{Path, PathBuf},
};

type Result<T> = std::result::Result<T, WorkerError>;

/// `storage_root` is the kernel-owned private storage directory, never an App
/// or client value. Every other path is derived from held descriptors.
pub(super) fn prepare(
    runtime: EnrolledRuntime,
    release: VerifiedReleaseLease,
    binding: &StageTrustBinding,
    storage_root: &Path,
) -> Result<PreparedWorker> {
    if release.package_digest() != binding.package_digest() {
        return Err(WorkerError::Identity);
    }
    let generation = binding.token().generation;
    let installation = binding.token().installation_id.clone();
    let storage = storage_macos::StorageRoot::open(storage_root)
        .and_then(|root| root.prepare(binding.owner_id(), &installation, generation))
        .map_err(|error| WorkerError::Storage(error.code()))?;
    let [data, temporary] = storage.paths();
    let [data_dir, temporary_dir] = storage
        .directories()
        .map_err(|error| WorkerError::Storage(error.code()))?;
    require_identity(&data, data_dir)?;
    require_identity(&temporary, temporary_dir)?;
    let package = descriptor_path(release.payload())?;
    let runtime_root = descriptor_path(runtime.root())?;
    let program_path = runtime_root.join("chariox-app-worker");
    let launcher = runtime
        .file("chariox-app-worker")
        .ok_or(WorkerError::Preparation)?;
    let held_launcher = held_file(&program_path, launcher)?;
    let record = LaunchRecord {
        generation: generation.to_string(),
        installation,
        release_digest: release.package_digest().into(),
        roots: [&package, &data, &temporary, &runtime_root].map(|path| path_string(path)),
        bootstrap: bootstrap(&release, &runtime_root)?,
        nofile: 128,
        cpu_seconds: 86400,
        heap_mib: 256,
        v8_threads: 1,
        max_file_bytes: 512 * 1024 * 1024,
    };
    if record.roots.iter().any(String::is_empty) {
        return Err(WorkerError::Preparation);
    }
    record.encode()?;
    let program = CString::new(program_path.as_os_str().as_encoded_bytes())
        .map_err(|_| WorkerError::Preparation)?;
    let private_data = data_dir.try_clone().map_err(|_| WorkerError::Preparation)?;
    Ok(PreparedWorker {
        program,
        arguments: Vec::new(),
        record,
        _objects: vec![held_launcher],
        domain: Box::new(Domain {
            launcher: program_path,
            private_data,
            _storage: storage,
            _runtime: runtime,
            _release: release,
        }),
    })
}

/// Field drop order closes the held data descriptor before MountedStorage's
/// Drop detaches the volumes. Leases stay here until WorkerProcess joins and
/// brokers drain, as the ResourceDomain contract requires.
struct Domain {
    launcher: PathBuf,
    private_data: File,
    _storage: storage_macos::MountedStorage,
    _runtime: EnrolledRuntime,
    _release: VerifiedReleaseLease,
}

impl ResourceDomain for Domain {
    fn private_data_directory(&self) -> Result<File> {
        self.private_data
            .try_clone()
            .map_err(|_| WorkerError::Preparation)
    }
    fn verify_before_continue(&mut self, pid: libc::pid_t) -> Result<()> {
        // The launcher is its own session and process-group leader, and its
        // image is the enrolled launcher; Seatbelt applies inside that image.
        if unsafe { libc::getsid(pid) } != pid || process_path(pid)? != self.launcher {
            return Err(WorkerError::ResourceDomain);
        }
        Ok(())
    }
    // Seatbelt denies process-fork, so the launcher has no descendants. The
    // monitor owns wait authority and signals only its unreaped direct child;
    // this domain never signals a PID or group that may have been reused.
    fn terminate(&mut self, _pid: libc::pid_t) {}
    fn reap_domain_blocking(&mut self) {}
}

fn descriptor_path(file: &File) -> Result<PathBuf> {
    let mut buffer = vec![0u8; libc::PATH_MAX as usize];
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETPATH, buffer.as_mut_ptr()) } != 0 {
        return Err(WorkerError::Preparation);
    }
    let length = buffer
        .iter()
        .position(|byte| *byte == 0)
        .ok_or(WorkerError::Preparation)?;
    buffer.truncate(length);
    let path = PathBuf::from(String::from_utf8(buffer).map_err(|_| WorkerError::Preparation)?);
    require_identity(&path, file)?;
    Ok(path)
}

fn require_identity(path: &Path, file: &File) -> Result<()> {
    let actual = std::fs::symlink_metadata(path).map_err(|_| WorkerError::Preparation)?;
    let expected = file.metadata().map_err(|_| WorkerError::Preparation)?;
    if actual.dev() != expected.dev() || actual.ino() != expected.ino() {
        return Err(WorkerError::Identity);
    }
    Ok(())
}

fn held_file(path: &Path, expected: &File) -> Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = File::options()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| WorkerError::Preparation)?;
    require_identity(path, &file)?;
    require_identity(path, expected)?;
    Ok(file)
}

fn process_path(pid: libc::pid_t) -> Result<PathBuf> {
    let mut buffer = vec![0u8; 4 * libc::PATH_MAX as usize];
    let length =
        unsafe { libc::proc_pidpath(pid, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
    if length <= 0 {
        return Err(WorkerError::ResourceDomain);
    }
    buffer.truncate(length as usize);
    Ok(PathBuf::from(
        String::from_utf8(buffer).map_err(|_| WorkerError::ResourceDomain)?,
    ))
}

fn path_string(path: &Path) -> String {
    path.to_str().map(str::to_owned).unwrap_or_default()
}

fn bootstrap(package: &VerifiedReleaseLease, runtime_root: &Path) -> Result<String> {
    let config = serde_json::json!({"version":1,"entry":package.manifest().runtime.entry,
        "declarations":{"tools":package.declarations().tools.iter().map(|tool| &tool.name).collect::<Vec<_>>(),
        "incomingEvents":package.declarations().events.iter().filter(|event| matches!(event.direction, EventDirection::Incoming | EventDirection::Both)).map(|event| &event.name).collect::<Vec<_>>()},
        "startupTimeoutMs":15000});
    bootstrap_source(&config, runtime_root)
}

/// The trusted bootstrap path is a JSON string literal, so no runtime path can
/// terminate the expression or inject code.
fn bootstrap_source(config: &serde_json::Value, runtime_root: &Path) -> Result<String> {
    let config = serde_json::to_string(config).map_err(|_| WorkerError::Preparation)?;
    let bootstrap = serde_json::to_string(&runtime_root.join("bootstrap.cjs"))
        .map_err(|_| WorkerError::Preparation)?;
    Ok(format!(
        "require('node:module').createRequire({bootstrap})({bootstrap}).start({config});"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_quotes_the_enrolled_runtime_path() {
        let source = bootstrap_source(
            &serde_json::json!({"version":1}),
            Path::new("/Library/Application Support/Chariox/AppRuntimes/a'b\"c"),
        )
        .unwrap();
        assert_eq!(
            source,
            r#"require('node:module').createRequire("/Library/Application Support/Chariox/AppRuntimes/a'b\"c/bootstrap.cjs")("/Library/Application Support/Chariox/AppRuntimes/a'b\"c/bootstrap.cjs").start({"version":1});"#
        );
    }

    #[test]
    fn descriptor_paths_resolve_to_the_held_object() {
        let root = std::env::temp_dir().join(format!("chariox-macos-path-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let held = File::open(&root).unwrap();
        assert_eq!(
            descriptor_path(&held).unwrap(),
            root.canonicalize().unwrap()
        );
        std::fs::remove_dir(&root).unwrap();
        assert!(
            descriptor_path(&held).is_err(),
            "a removed root no longer matches its identity"
        );
    }
}
