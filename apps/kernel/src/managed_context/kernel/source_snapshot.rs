use std::ffi::{CString, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime};

use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::error::DaemonError;

const MAX_SOURCE_FILES: usize = 16_384;
const MAX_SOURCE_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_SOURCE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_SOURCE_DEPTH: usize = 64;
const SNAPSHOT_PARENT_DIRECTORY: &str = "chariox-kernel-context-snapshots";
const SNAPSHOT_DIRECTORY_PREFIX: &str = "chariox-kernel-context-";
const MISSING_LEASE_GRACE: Duration = Duration::from_secs(5);
const MAX_SCAVENGE_ENTRIES: usize = 1_024;
const MAX_STARTUP_SCAVENGE_REMOVALS: usize = 4;
const MAX_SCAVENGE_REMOVALS: usize = 64;
const MAX_LIVE_SNAPSHOTS: usize = 8;
const MAX_RECENT_MISSING_LEASES: usize = 8;
const PORTABLE_ENVIRONMENT_FILES: [&str; 4] = [
    "manifest.json",
    "requirements.lock",
    "package.json",
    "package-lock.json",
];

pub(super) struct KernelContextSourceSnapshot {
    _root: PrivateSnapshotRoot,
    pub(super) mcp_root: PathBuf,
    pub(super) original_mcp_root: Option<PathBuf>,
    pub(super) skill_root: PathBuf,
    pub(super) script_root: PathBuf,
    pub(super) environment_root: PathBuf,
    pub(super) original_environment_root: Option<PathBuf>,
    pub(super) connector_root: PathBuf,
    pub(super) connector_adapter_root: PathBuf,
    pub(super) credential_root: PathBuf,
    pub(super) bundled_adapter_roots: Vec<PathBuf>,
}

impl KernelContextSourceSnapshot {
    pub(super) fn capture() -> Result<Self, DaemonError> {
        let first = Self::capture_once()?;
        let second = Self::capture_once()?;
        if first.content_digest()? != second.content_digest()? {
            return Err(source_changed_error());
        }
        Ok(second)
    }

    fn capture_once() -> Result<Self, DaemonError> {
        let root = PrivateSnapshotRoot::new()?;
        let mut budget = SourceBudget::default();
        let mcp_root = root.path.join("user/mcps");
        let skill_root = root.path.join("user/skills");
        let script_root = root.path.join("user/scripts");
        let environment_root = root.path.join("user/envs");
        let connector_root = root.path.join("connectors/definitions");
        let connector_adapter_root = root.path.join("connectors/adapters");
        let credential_root = root.path.join("credentials");
        let original_mcp_root = crate::mcp::CharioxMcpRegistry::user_root();
        capture_optional_root(original_mcp_root.as_deref(), &mcp_root, &mut budget)?;
        capture_optional_root(
            crate::skill::CharioxSkillRegistry::user_root().as_deref(),
            &skill_root,
            &mut budget,
        )?;
        capture_optional_root(
            crate::script::CharioxScriptRegistry::user_root().as_deref(),
            &script_root,
            &mut budget,
        )?;
        let original_environment_root = crate::script::CharioxEnvironmentRegistry::user_root();
        capture_environment_root(
            original_environment_root.as_deref(),
            &environment_root,
            &mut budget,
        )?;
        capture_optional_root(
            crate::connector::CharioxConnectorRegistry::user_root().as_deref(),
            &connector_root,
            &mut budget,
        )?;
        capture_optional_root(
            crate::connector::CharioxConnectorAdapterRegistry::user_root().as_deref(),
            &connector_adapter_root,
            &mut budget,
        )?;
        capture_optional_root(
            crate::credential::CharioxCredentialRegistry::user_root().as_deref(),
            &credential_root,
            &mut budget,
        )?;

        let mut bundled_adapter_roots = Vec::new();
        for (index, source) in crate::connector::CharioxConnectorAdapterRegistry::bundled_roots()
            .into_iter()
            .enumerate()
        {
            let destination = root.path.join("bundled-adapters").join(index.to_string());
            capture_optional_root(Some(&source), &destination, &mut budget)?;
            bundled_adapter_roots.push(destination);
        }

        Ok(Self {
            _root: root,
            mcp_root,
            original_mcp_root,
            skill_root,
            script_root,
            environment_root,
            original_environment_root,
            connector_root,
            connector_adapter_root,
            credential_root,
            bundled_adapter_roots,
        })
    }

    fn content_digest(&self) -> Result<String, DaemonError> {
        let mut hasher = Sha256::new();
        let mut budget = SourceBudget::default();
        for (label, root) in self.roots() {
            hasher.update(label.as_bytes());
            hasher.update([0]);
            hash_snapshot_tree(root, root, &mut hasher, &mut budget, 0)?;
        }
        Ok(format!("{:x}", hasher.finalize()))
    }

    fn roots(&self) -> Vec<(String, &Path)> {
        let mut roots = vec![
            ("mcp".to_string(), self.mcp_root.as_path()),
            ("skill".to_string(), self.skill_root.as_path()),
            ("script".to_string(), self.script_root.as_path()),
            ("environment".to_string(), self.environment_root.as_path()),
            ("connector".to_string(), self.connector_root.as_path()),
            (
                "connector_adapter".to_string(),
                self.connector_adapter_root.as_path(),
            ),
            ("credential".to_string(), self.credential_root.as_path()),
        ];
        roots.extend(
            self.bundled_adapter_roots
                .iter()
                .enumerate()
                .map(|(index, root)| (format!("bundled_adapter_{index}"), root.as_path())),
        );
        roots
    }
}

#[derive(Default)]
struct SourceBudget {
    entries: usize,
    bytes: u64,
}

impl SourceBudget {
    fn add_entry(&mut self) -> Result<(), DaemonError> {
        self.entries = self.entries.saturating_add(1);
        if self.entries > MAX_SOURCE_FILES {
            return Err(source_error(
                "kernel Extension source exceeds its entry limit",
            ));
        }
        Ok(())
    }

    fn add_file(&mut self, bytes: u64) -> Result<(), DaemonError> {
        self.bytes = self.bytes.saturating_add(bytes);
        if self.bytes > MAX_SOURCE_BYTES {
            return Err(source_error(
                "kernel Extension source exceeds its byte limit",
            ));
        }
        Ok(())
    }
}

struct PrivateSnapshotRoot {
    path: PathBuf,
    _lease: File,
}

impl PrivateSnapshotRoot {
    fn new() -> Result<Self, DaemonError> {
        let parent = snapshot_parent()?;
        scavenge_stale_snapshots(&parent, ScavengeMode::Concurrent, MISSING_LEASE_GRACE)?;
        for _ in 0..32 {
            let path = parent.join(format!(
                "{SNAPSHOT_DIRECTORY_PREFIX}{}",
                random_snapshot_suffix()
            ));
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&path) {
                Ok(()) => {
                    if let Err(error) = set_private_directory(&path) {
                        let _ = fs::remove_dir(&path);
                        return Err(error);
                    }
                    let lease = match create_snapshot_lease(&path) {
                        Ok(lease) => lease,
                        Err(error) => {
                            let _ = fs::remove_dir_all(&path);
                            return Err(error);
                        }
                    };
                    return Ok(Self {
                        path,
                        _lease: lease,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(source_io_error("create source snapshot", error)),
            }
        }
        Err(source_error(
            "could not reserve a private kernel Extension snapshot directory",
        ))
    }
}

#[derive(Clone, Copy)]
enum ScavengeMode {
    Startup,
    Concurrent,
}

pub fn scavenge_source_snapshots() {
    let result = snapshot_parent().and_then(|parent| {
        scavenge_stale_snapshots(&parent, ScavengeMode::Startup, MISSING_LEASE_GRACE)
    });
    if let Err(error) = result {
        crate::logging::warn_with_fields(
            "managed_context.snapshot.cleanup",
            "bounded startup cleanup left kernel context snapshot work for a later export",
            serde_json::json!({
                "error": error.to_string(),
            }),
        );
    }
}

fn snapshot_parent() -> Result<PathBuf, DaemonError> {
    let temporary_root = fs::canonicalize(std::env::temp_dir())
        .map_err(|error| source_io_error("resolve source snapshot parent", error))?;
    ensure_snapshot_parent(&temporary_root)
}

fn ensure_snapshot_parent(temporary_root: &Path) -> Result<PathBuf, DaemonError> {
    let parent_name = snapshot_parent_directory_name();
    let parent = temporary_root.join(&parent_name);
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(&parent) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(source_io_error("create source snapshot parent", error)),
    }
    let metadata = fs::symlink_metadata(&parent)
        .map_err(|error| source_io_error("inspect source snapshot parent", error))?;
    validate_snapshot_parent_metadata(&metadata)?;
    let canonical = fs::canonicalize(&parent)
        .map_err(|error| source_io_error("resolve source snapshot parent", error))?;
    if canonical.parent() != Some(temporary_root)
        || canonical.file_name().and_then(|name| name.to_str()) != Some(parent_name.as_str())
    {
        return Err(source_error(
            "kernel Extension snapshot parent escaped the temporary root",
        ));
    }
    Ok(canonical)
}

fn snapshot_parent_directory_name() -> String {
    #[cfg(unix)]
    {
        return format!("{SNAPSHOT_PARENT_DIRECTORY}-{}", unsafe { libc::geteuid() });
    }
    #[cfg(not(unix))]
    SNAPSHOT_PARENT_DIRECTORY.to_string()
}

fn validate_snapshot_parent_metadata(metadata: &fs::Metadata) -> Result<(), DaemonError> {
    if metadata.file_type().is_symlink()
        || opened_metadata_is_reparse_point(metadata)
        || !metadata.is_dir()
    {
        return Err(source_error(
            "kernel Extension snapshot parent must be a directory without links",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(source_error(
                "kernel Extension snapshot parent must be private and owned by the current user",
            ));
        }
    }
    Ok(())
}

fn random_snapshot_suffix() -> String {
    let mut random = [0_u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut random);
    random.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn is_snapshot_directory_name(name: &str) -> bool {
    let Some(suffix) = name.strip_prefix(SNAPSHOT_DIRECTORY_PREFIX) else {
        return false;
    };
    suffix.len() == 32
        && suffix
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn scavenge_stale_snapshots(
    parent: &Path,
    mode: ScavengeMode,
    missing_lease_grace: Duration,
) -> Result<(), DaemonError> {
    let now = SystemTime::now();
    let mut scanned = 0_usize;
    let mut removed = 0_usize;
    let mut live_snapshots = 0_usize;
    let mut recent_missing_leases = 0_usize;
    let removal_limit = match mode {
        ScavengeMode::Startup => MAX_STARTUP_SCAVENGE_REMOVALS,
        ScavengeMode::Concurrent => MAX_SCAVENGE_REMOVALS,
    };
    let entries = fs::read_dir(parent)
        .map_err(|error| source_io_error("enumerate stale source snapshots", error))?;
    for entry in entries {
        scanned = scanned.saturating_add(1);
        if scanned > MAX_SCAVENGE_ENTRIES {
            return Err(source_cleanup_backlog_error());
        }
        let entry =
            entry.map_err(|error| source_io_error("enumerate stale source snapshots", error))?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !is_snapshot_directory_name(&name) {
            continue;
        }
        let path = entry.path();
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(source_io_error("inspect stale source snapshot", error)),
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.uid() != unsafe { libc::geteuid() } {
                continue;
            }
        }
        let missing_lease_old_enough = metadata
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= missing_lease_grace);
        let _lease = match try_lock_snapshot_lease(&path)? {
            SnapshotLeaseState::Active => {
                live_snapshots = live_snapshots.saturating_add(1);
                if live_snapshots >= MAX_LIVE_SNAPSHOTS {
                    return Err(source_cleanup_backlog_error());
                }
                continue;
            }
            SnapshotLeaseState::Acquired(lease) => Some(lease),
            SnapshotLeaseState::Missing => {
                if !missing_lease_old_enough {
                    recent_missing_leases = recent_missing_leases.saturating_add(1);
                    if recent_missing_leases >= MAX_RECENT_MISSING_LEASES {
                        return Err(source_cleanup_backlog_error());
                    }
                    continue;
                }
                None
            }
        };
        if removed >= removal_limit {
            return Err(source_cleanup_backlog_error());
        }
        fs::remove_dir_all(&path)
            .map_err(|error| source_io_error("remove stale source snapshot", error))?;
        removed = removed.saturating_add(1);
    }
    Ok(())
}

enum SnapshotLeaseState {
    Active,
    Acquired(File),
    Missing,
}

fn create_snapshot_lease(root: &Path) -> Result<File, DaemonError> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lease = options
        .open(root.join(".lease"))
        .map_err(|error| source_io_error("create source snapshot lease", error))?;
    fs2::FileExt::try_lock_exclusive(&lease)
        .map_err(|error| source_io_error("lock source snapshot lease", error))?;
    Ok(lease)
}

fn try_lock_snapshot_lease(root: &Path) -> Result<SnapshotLeaseState, DaemonError> {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let lease = match options.open(root.join(".lease")) {
        Ok(lease) => lease,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(SnapshotLeaseState::Missing)
        }
        Err(error) => return Err(source_io_error("open source snapshot lease", error)),
    };
    let metadata = lease
        .metadata()
        .map_err(|error| source_io_error("inspect source snapshot lease", error))?;
    if !metadata.is_file() || opened_metadata_is_reparse_point(&metadata) {
        return Err(source_error("source snapshot lease is not a regular file"));
    }
    match fs2::FileExt::try_lock_exclusive(&lease) {
        Ok(()) => Ok(SnapshotLeaseState::Acquired(lease)),
        Err(error) if lock_is_contended(&error) => Ok(SnapshotLeaseState::Active),
        Err(error) => Err(source_io_error("lock source snapshot lease", error)),
    }
}

fn lock_is_contended(error: &std::io::Error) -> bool {
    let expected = fs2::lock_contended_error();
    match (error.raw_os_error(), expected.raw_os_error()) {
        (Some(actual), Some(expected)) => actual == expected,
        _ => error.kind() == expected.kind(),
    }
}

impl Drop for PrivateSnapshotRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn capture_optional_root(
    source: Option<&Path>,
    destination: &Path,
    budget: &mut SourceBudget,
) -> Result<(), DaemonError> {
    create_private_directory(destination)?;
    let Some(source) = source else {
        return Ok(());
    };
    capture_source_tree(source, destination, budget)
}

fn captured_environment_names(root: &Path) -> Result<Vec<String>, DaemonError> {
    let registry = crate::script::CharioxEnvironmentRegistry::new(vec![root.to_path_buf()]);
    let mut names = Vec::new();
    for environment in registry.list()? {
        crate::mcp::validate_registry_name(&environment.name, "environment name")?;
        names.push(environment.name);
    }
    names.sort();
    names.dedup();
    Ok(names)
}

#[cfg(unix)]
fn capture_environment_root(
    source: Option<&Path>,
    destination: &Path,
    budget: &mut SourceBudget,
) -> Result<(), DaemonError> {
    create_private_directory(destination)?;
    let Some(source) = source else {
        return Ok(());
    };
    let Some(root) = unix::open_directory_chain(source)? else {
        return Ok(());
    };
    unix::copy_environment_definitions(&root, destination, budget)?;
    for name in captured_environment_names(destination)? {
        unix::copy_environment_package(&root, destination, &name, budget)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn capture_environment_root(
    source: Option<&Path>,
    destination: &Path,
    budget: &mut SourceBudget,
) -> Result<(), DaemonError> {
    create_private_directory(destination)?;
    let Some(source) = source else {
        return Ok(());
    };
    let metadata = match fs::symlink_metadata(source) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(source_io_error("inspect environment registry", error)),
    };
    if metadata.file_type().is_symlink()
        || opened_metadata_is_reparse_point(&metadata)
        || !metadata.is_dir()
    {
        return Err(source_error(
            "environment registry must be a directory without reparse points",
        ));
    }
    let definitions = bounded_directory_entries(source, budget)?;
    for entry in definitions
        .into_iter()
        .filter(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("json"))
    {
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| source_error("environment registry path is not UTF-8"))?;
        validate_snapshot_component(name)?;
        copy_selected_fallback_file(&entry.path(), &destination.join(name), budget)?;
    }
    for name in captured_environment_names(destination)? {
        let source_package = source.join(".portable").join(&name);
        let metadata = match fs::symlink_metadata(&source_package) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(source_io_error("inspect portable environment", error)),
        };
        if metadata.file_type().is_symlink()
            || opened_metadata_is_reparse_point(&metadata)
            || !metadata.is_dir()
        {
            return Err(source_error(
                "portable environment must be a directory without reparse points",
            ));
        }
        budget.add_entry()?;
        let destination_package = destination.join(".portable").join(&name);
        create_private_directory(&destination_package)?;
        for file_name in PORTABLE_ENVIRONMENT_FILES {
            let source_file = source_package.join(file_name);
            match fs::symlink_metadata(&source_file) {
                Ok(_) => {
                    budget.add_entry()?;
                    copy_selected_fallback_file(
                        &source_file,
                        &destination_package.join(file_name),
                        budget,
                    )?
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(source_io_error("inspect portable environment input", error))
                }
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
fn capture_source_tree(
    source: &Path,
    destination: &Path,
    budget: &mut SourceBudget,
) -> Result<(), DaemonError> {
    let Some(root) = unix::open_directory_chain(source)? else {
        return Ok(());
    };
    unix::copy_directory(&root, destination, budget, 0)
}

#[cfg(not(unix))]
fn capture_source_tree(
    source: &Path,
    destination: &Path,
    budget: &mut SourceBudget,
) -> Result<(), DaemonError> {
    let metadata = match fs::symlink_metadata(source) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(source_io_error("inspect source root", error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(source_error(
            "kernel Extension source root must be a directory without reparse points",
        ));
    }
    copy_directory_fallback(source, destination, budget, 0)
}

#[cfg(not(unix))]
fn copy_directory_fallback(
    source: &Path,
    destination: &Path,
    budget: &mut SourceBudget,
    depth: usize,
) -> Result<(), DaemonError> {
    if depth > MAX_SOURCE_DEPTH {
        return Err(source_error(
            "kernel Extension source exceeds its directory depth limit",
        ));
    }
    let mut entries = bounded_directory_entries(source, budget)?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| source_error("kernel Extension source path is not UTF-8"))?;
        validate_snapshot_component(name)?;
        let source_path = entry.path();
        let destination_path = destination.join(name);
        let metadata = fs::symlink_metadata(&source_path)
            .map_err(|error| source_io_error("inspect source snapshot entry", error))?;
        if metadata.file_type().is_symlink() || opened_metadata_is_reparse_point(&metadata) {
            return Err(source_error(
                "kernel Extension source contains a symlink or reparse point",
            ));
        }
        if metadata.is_dir() {
            create_private_directory(&destination_path)?;
            copy_directory_fallback(&source_path, &destination_path, budget, depth + 1)?;
        } else if metadata.is_file() {
            let bytes = read_bounded_source_file(&source_path)?;
            budget.add_file(bytes.len() as u64)?;
            write_private_snapshot_file(&destination_path, &bytes, false)?;
        } else {
            return Err(source_error(
                "kernel Extension source contains a special file",
            ));
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn bounded_directory_entries(
    source: &Path,
    budget: &mut SourceBudget,
) -> Result<Vec<fs::DirEntry>, DaemonError> {
    let mut entries = Vec::new();
    for entry in
        fs::read_dir(source).map_err(|error| source_io_error("enumerate source snapshot", error))?
    {
        budget.add_entry()?;
        entries.push(entry.map_err(|error| source_io_error("enumerate source snapshot", error))?);
    }
    Ok(entries)
}

#[cfg(not(unix))]
fn copy_selected_fallback_file(
    source: &Path,
    destination: &Path,
    budget: &mut SourceBudget,
) -> Result<(), DaemonError> {
    let metadata = fs::symlink_metadata(source)
        .map_err(|error| source_io_error("inspect selected source input", error))?;
    if metadata.file_type().is_symlink()
        || opened_metadata_is_reparse_point(&metadata)
        || !metadata.is_file()
    {
        return Err(source_error(
            "selected source input must be a regular file without reparse points",
        ));
    }
    let bytes = read_bounded_source_file(source)?;
    budget.add_file(bytes.len() as u64)?;
    write_private_snapshot_file(destination, &bytes, false)
}

fn hash_snapshot_tree(
    root: &Path,
    directory: &Path,
    hasher: &mut Sha256,
    budget: &mut SourceBudget,
    depth: usize,
) -> Result<(), DaemonError> {
    if depth > MAX_SOURCE_DEPTH {
        return Err(source_error(
            "kernel Extension snapshot exceeds its directory depth limit",
        ));
    }
    let mut entries = fs::read_dir(directory)
        .map_err(|error| source_io_error("enumerate private source snapshot", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| source_io_error("enumerate private source snapshot", error))?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        budget.add_entry()?;
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .map_err(|_| source_error("private source snapshot path escaped its root"))?;
        let relative = relative
            .to_str()
            .ok_or_else(|| source_error("private source snapshot path is not UTF-8"))?;
        hasher.update(relative.as_bytes());
        hasher.update([0]);
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| source_io_error("inspect private source snapshot", error))?;
        if metadata.file_type().is_symlink() || (!metadata.is_dir() && !metadata.is_file()) {
            return Err(source_error(
                "private kernel Extension snapshot contains an unsafe entry",
            ));
        }
        if metadata.is_dir() {
            hasher.update(b"directory\0");
            hash_snapshot_tree(root, &path, hasher, budget, depth + 1)?;
        } else {
            let bytes = read_bounded_source_file(&path)?;
            budget.add_file(bytes.len() as u64)?;
            hasher.update(b"file\0");
            hasher.update((bytes.len() as u64).to_le_bytes());
            hasher.update(Sha256::digest(&bytes));
            hasher.update([u8::from(is_executable(&metadata))]);
        }
    }
    Ok(())
}

fn read_bounded_source_file(path: &Path) -> Result<Vec<u8>, DaemonError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options
        .open(path)
        .map_err(|error| source_io_error("open source snapshot file", error))?;
    let metadata = file
        .metadata()
        .map_err(|error| source_io_error("inspect source snapshot file", error))?;
    if !metadata.is_file()
        || metadata.len() > MAX_SOURCE_FILE_BYTES
        || opened_metadata_is_reparse_point(&metadata)
    {
        return Err(source_error(
            "kernel Extension source file must be a bounded regular file",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_SOURCE_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| source_io_error("read source snapshot file", error))?;
    if bytes.len() as u64 > MAX_SOURCE_FILE_BYTES {
        return Err(source_error(
            "kernel Extension source file exceeds its size limit",
        ));
    }
    Ok(bytes)
}

fn create_private_directory(path: &Path) -> Result<(), DaemonError> {
    fs::create_dir_all(path)
        .map_err(|error| source_io_error("create source snapshot path", error))?;
    set_private_directory(path)
}

fn set_private_directory(path: &Path) -> Result<(), DaemonError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| source_io_error("secure source snapshot directory", error))?;
    }
    Ok(())
}

fn write_private_snapshot_file(
    path: &Path,
    bytes: &[u8],
    executable: bool,
) -> Result<(), DaemonError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(if executable { 0o700 } else { 0o600 });
    }
    let mut file = options
        .open(path)
        .map_err(|error| source_io_error("create source snapshot file", error))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| source_io_error("write source snapshot file", error))?;
    Ok(())
}

fn validate_snapshot_component(name: &str) -> Result<(), DaemonError> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\0'])
        || name.chars().any(char::is_control)
    {
        return Err(source_error(
            "kernel Extension source contains an unsafe path component",
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn is_executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(windows)]
fn opened_metadata_is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn opened_metadata_is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

fn source_changed_error() -> DaemonError {
    DaemonError::ManagedContext {
        code: "kernel_context_source_changed",
        operation: "kernel_context.snapshot",
        message: "kernel Extension registries changed during snapshot; retry the export"
            .to_string(),
        retryable: true,
    }
}

fn source_cleanup_backlog_error() -> DaemonError {
    DaemonError::ManagedContext {
        code: "kernel_context_cleanup_backlog",
        operation: "kernel_context.snapshot.cleanup",
        message:
            "kernel Extension snapshot cleanup exceeded its bounded pass; retry startup or export"
                .to_string(),
        retryable: true,
    }
}

fn source_error(message: impl Into<String>) -> DaemonError {
    DaemonError::ManagedContext {
        code: "invalid_kernel_context_source",
        operation: "kernel_context.snapshot",
        message: message.into(),
        retryable: false,
    }
}

fn source_io_error(operation: &'static str, error: std::io::Error) -> DaemonError {
    DaemonError::ManagedContext {
        code: "kernel_context_source_unavailable",
        operation,
        message: error.to_string(),
        retryable: true,
    }
}

#[cfg(unix)]
mod unix {
    use std::ffi::CStr;
    use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    use super::*;

    pub(super) fn open_directory_chain(path: &Path) -> Result<Option<OwnedFd>, DaemonError> {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|error| source_io_error("resolve source snapshot root", error))?
                .join(path)
        };
        let root_name = CString::new("/").expect("root path contains no NUL");
        let root_fd = unsafe {
            libc::open(
                root_name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        };
        if root_fd < 0 {
            return Err(source_io_error(
                "open source filesystem root",
                std::io::Error::last_os_error(),
            ));
        }
        let mut current = unsafe { OwnedFd::from_raw_fd(root_fd) };
        for component in absolute.components() {
            match component {
                Component::RootDir | Component::CurDir => continue,
                Component::Normal(name) => {
                    let name = CString::new(name.as_bytes())
                        .map_err(|_| source_error("kernel Extension source path contains NUL"))?;
                    let next = unsafe {
                        libc::openat(
                            current.as_raw_fd(),
                            name.as_ptr(),
                            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                        )
                    };
                    if next < 0 {
                        let error = std::io::Error::last_os_error();
                        if error.kind() == std::io::ErrorKind::NotFound {
                            return Ok(None);
                        }
                        if matches!(
                            error.raw_os_error(),
                            Some(libc::ELOOP) | Some(libc::ENOTDIR)
                        ) {
                            return Err(source_error(
                                "kernel Extension source root contains a symlink or non-directory component",
                            ));
                        }
                        return Err(source_io_error(
                            "open source directory without links",
                            error,
                        ));
                    }
                    current = unsafe { OwnedFd::from_raw_fd(next) };
                }
                _ => {
                    return Err(source_error(
                        "kernel Extension source root is not an absolute normal path",
                    ))
                }
            }
        }
        Ok(Some(current))
    }

    pub(super) fn copy_directory(
        source: &OwnedFd,
        destination: &Path,
        budget: &mut SourceBudget,
        depth: usize,
    ) -> Result<(), DaemonError> {
        if depth > MAX_SOURCE_DEPTH {
            return Err(source_error(
                "kernel Extension source exceeds its directory depth limit",
            ));
        }
        let names = read_directory_names(source)?;
        for name in names {
            budget.add_entry()?;
            let name_text = name
                .to_str()
                .ok_or_else(|| source_error("kernel Extension source path is not UTF-8"))?;
            validate_snapshot_component(name_text)?;
            let c_name = CString::new(name.as_bytes())
                .map_err(|_| source_error("kernel Extension source path contains NUL"))?;
            let mut before = std::mem::MaybeUninit::<libc::stat>::uninit();
            let status = unsafe {
                libc::fstatat(
                    source.as_raw_fd(),
                    c_name.as_ptr(),
                    before.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            };
            if status != 0 {
                return Err(source_io_error(
                    "inspect source snapshot entry",
                    std::io::Error::last_os_error(),
                ));
            }
            let before = unsafe { before.assume_init() };
            let file_type = before.st_mode & libc::S_IFMT;
            let destination_path = destination.join(&name);
            if file_type == libc::S_IFDIR {
                let child = open_at(
                    source,
                    &c_name,
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )?;
                ensure_same_identity(&before, &child)?;
                create_private_directory(&destination_path)?;
                copy_directory(&child, &destination_path, budget, depth + 1)?;
            } else if file_type == libc::S_IFREG {
                copy_regular_entry(source, &c_name, &before, &destination_path, budget)?;
            } else if file_type == libc::S_IFLNK {
                return Err(source_error("kernel Extension source contains a symlink"));
            } else {
                return Err(source_error(
                    "kernel Extension source contains a special file",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn copy_environment_definitions(
        source: &OwnedFd,
        destination: &Path,
        budget: &mut SourceBudget,
    ) -> Result<(), DaemonError> {
        for name in read_directory_names(source)? {
            let Some(name_text) = name.to_str() else {
                continue;
            };
            if Path::new(name_text)
                .extension()
                .and_then(|value| value.to_str())
                != Some("json")
            {
                continue;
            }
            budget.add_entry()?;
            validate_snapshot_component(name_text)?;
            let c_name = CString::new(name.as_bytes())
                .map_err(|_| source_error("environment definition path contains NUL"))?;
            let before = stat_at(source, &c_name)?.ok_or_else(source_changed_error)?;
            if before.st_mode & libc::S_IFMT != libc::S_IFREG {
                return Err(source_error(
                    "environment definition must be a regular file without links",
                ));
            }
            copy_regular_entry(source, &c_name, &before, &destination.join(name), budget)?;
        }
        Ok(())
    }

    pub(super) fn copy_environment_package(
        source: &OwnedFd,
        destination: &Path,
        environment_name: &str,
        budget: &mut SourceBudget,
    ) -> Result<(), DaemonError> {
        let portable_name = CString::new(".portable").expect("static path contains no NUL");
        let Some(portable) = open_optional_directory_at(source, &portable_name)? else {
            return Ok(());
        };
        let environment_name = CString::new(environment_name)
            .map_err(|_| source_error("portable environment name contains NUL"))?;
        let Some(environment) = open_optional_directory_at(&portable, &environment_name)? else {
            return Ok(());
        };
        budget.add_entry()?;
        let destination_package = destination.join(".portable").join(
            environment_name
                .to_str()
                .map_err(|_| source_error("portable environment name is not valid UTF-8"))?,
        );
        create_private_directory(&destination_package)?;
        for file_name in PORTABLE_ENVIRONMENT_FILES {
            let c_name = CString::new(file_name).expect("static path contains no NUL");
            let Some(before) = stat_at(&environment, &c_name)? else {
                continue;
            };
            if before.st_mode & libc::S_IFMT != libc::S_IFREG {
                return Err(source_error(
                    "portable environment input must be a regular file without links",
                ));
            }
            budget.add_entry()?;
            copy_regular_entry(
                &environment,
                &c_name,
                &before,
                &destination_package.join(file_name),
                budget,
            )?;
        }
        Ok(())
    }

    fn stat_at(source: &OwnedFd, name: &CString) -> Result<Option<libc::stat>, DaemonError> {
        let mut state = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                source.as_raw_fd(),
                name.as_ptr(),
                state.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } == 0
        {
            return Ok(Some(unsafe { state.assume_init() }));
        }
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::NotFound {
            Ok(None)
        } else {
            Err(source_io_error("inspect selected source input", error))
        }
    }

    fn open_optional_directory_at(
        source: &OwnedFd,
        name: &CString,
    ) -> Result<Option<OwnedFd>, DaemonError> {
        let Some(before) = stat_at(source, name)? else {
            return Ok(None);
        };
        if before.st_mode & libc::S_IFMT != libc::S_IFDIR {
            return Err(source_error(
                "portable environment path must be a directory without links",
            ));
        }
        let opened = open_at(
            source,
            name,
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )?;
        ensure_same_identity(&before, &opened)?;
        Ok(Some(opened))
    }

    fn copy_regular_entry(
        source: &OwnedFd,
        name: &CString,
        before: &libc::stat,
        destination: &Path,
        budget: &mut SourceBudget,
    ) -> Result<(), DaemonError> {
        let child = open_at(
            source,
            name,
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )?;
        ensure_same_identity(before, &child)?;
        let initial = fstat(&child)?;
        if initial.st_size < 0 || initial.st_size as u64 > MAX_SOURCE_FILE_BYTES {
            return Err(source_error(
                "kernel Extension source file exceeds its size limit",
            ));
        }
        let executable = initial.st_mode & 0o111 != 0;
        let mut file = File::from(child);
        let mut bytes = Vec::with_capacity(initial.st_size as usize);
        Read::by_ref(&mut file)
            .take(MAX_SOURCE_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| source_io_error("read source snapshot file", error))?;
        if bytes.len() as u64 > MAX_SOURCE_FILE_BYTES {
            return Err(source_error(
                "kernel Extension source file exceeds its size limit",
            ));
        }
        let final_state = fstat_file(&file)?;
        if !same_file_state(&initial, &final_state) {
            return Err(source_changed_error());
        }
        budget.add_file(bytes.len() as u64)?;
        write_private_snapshot_file(destination, &bytes, executable)
    }

    fn read_directory_names(source: &OwnedFd) -> Result<Vec<OsString>, DaemonError> {
        let duplicated = duplicate_cloexec(source)?;
        let duplicated = duplicated.into_raw_fd();
        let directory = unsafe { libc::fdopendir(duplicated) };
        if directory.is_null() {
            unsafe { libc::close(duplicated) };
            return Err(source_io_error(
                "open source directory stream",
                std::io::Error::last_os_error(),
            ));
        }
        let mut names = Vec::new();
        loop {
            set_errno(0);
            let entry = unsafe { libc::readdir(directory) };
            if entry.is_null() {
                let error = errno();
                unsafe { libc::closedir(directory) };
                if error == 0 {
                    break;
                }
                return Err(source_io_error(
                    "read source directory stream",
                    std::io::Error::from_raw_os_error(error),
                ));
            }
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if name == b"." || name == b".." {
                continue;
            }
            names.push(OsString::from_vec(name.to_vec()));
            if names.len() > MAX_SOURCE_FILES {
                unsafe { libc::closedir(directory) };
                return Err(source_error(
                    "kernel Extension source entry count exceeds its limit",
                ));
            }
        }
        names.sort();
        Ok(names)
    }

    pub(super) fn duplicate_cloexec(source: &OwnedFd) -> Result<OwnedFd, DaemonError> {
        let duplicated = unsafe { libc::fcntl(source.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
        if duplicated < 0 {
            return Err(source_io_error(
                "duplicate source directory descriptor",
                std::io::Error::last_os_error(),
            ));
        }
        Ok(unsafe { OwnedFd::from_raw_fd(duplicated) })
    }

    fn open_at(source: &OwnedFd, name: &CString, flags: i32) -> Result<OwnedFd, DaemonError> {
        let fd = unsafe { libc::openat(source.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(source_io_error(
                "open source snapshot entry without links",
                std::io::Error::last_os_error(),
            ));
        }
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    fn ensure_same_identity(before: &libc::stat, opened: &OwnedFd) -> Result<(), DaemonError> {
        let after = fstat(opened)?;
        if before.st_dev != after.st_dev
            || before.st_ino != after.st_ino
            || before.st_mode & libc::S_IFMT != after.st_mode & libc::S_IFMT
        {
            return Err(source_changed_error());
        }
        Ok(())
    }

    fn fstat(fd: &OwnedFd) -> Result<libc::stat, DaemonError> {
        let mut state = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe { libc::fstat(fd.as_raw_fd(), state.as_mut_ptr()) } != 0 {
            return Err(source_io_error(
                "inspect opened source snapshot entry",
                std::io::Error::last_os_error(),
            ));
        }
        Ok(unsafe { state.assume_init() })
    }

    fn fstat_file(file: &File) -> Result<libc::stat, DaemonError> {
        let mut state = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe { libc::fstat(file.as_raw_fd(), state.as_mut_ptr()) } != 0 {
            return Err(source_io_error(
                "reinspect opened source snapshot file",
                std::io::Error::last_os_error(),
            ));
        }
        Ok(unsafe { state.assume_init() })
    }

    fn same_file_state(left: &libc::stat, right: &libc::stat) -> bool {
        left.st_dev == right.st_dev
            && left.st_ino == right.st_ino
            && left.st_size == right.st_size
            && left.st_mtime == right.st_mtime
            && left.st_mtime_nsec == right.st_mtime_nsec
            && left.st_ctime == right.st_ctime
            && left.st_ctime_nsec == right.st_ctime_nsec
    }

    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
    fn errno() -> i32 {
        unsafe { *libc::__error() }
    }

    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
    fn set_errno(value: i32) {
        unsafe { *libc::__error() = value }
    }

    #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "freebsd")))]
    fn errno() -> i32 {
        unsafe { *libc::__errno_location() }
    }

    #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "freebsd")))]
    fn set_errno(value: i32) {
        unsafe { *libc::__errno_location() = value }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_snapshot_scavenging_is_exact_and_preserves_active_leases() {
        let parent = fs::canonicalize(std::env::temp_dir())
            .expect("temporary root should canonicalize")
            .join(format!(
                "chariox-kernel-source-scavenge-{}-{}",
                std::process::id(),
                rand::random::<u64>()
            ));
        fs::create_dir_all(&parent).expect("scavenge parent should create");
        let stale = parent.join(format!("{SNAPSHOT_DIRECTORY_PREFIX}{}", "0".repeat(32)));
        fs::create_dir(&stale).expect("stale snapshot should create");
        let stale_lease = create_snapshot_lease(&stale).expect("stale lease should create");
        drop(stale_lease);
        fs::write(stale.join("secret.json"), "private").expect("stale file should write");
        let active = parent.join(format!("{SNAPSHOT_DIRECTORY_PREFIX}{}", "1".repeat(32)));
        fs::create_dir(&active).expect("active snapshot should create");
        let lease = create_snapshot_lease(&active).expect("active lease should create");
        assert!(matches!(
            try_lock_snapshot_lease(&active).expect("active lease should inspect"),
            SnapshotLeaseState::Active
        ));
        let creating = parent.join(format!("{SNAPSHOT_DIRECTORY_PREFIX}{}", "3".repeat(32)));
        fs::create_dir(&creating).expect("creating snapshot should create");
        let near_match = parent.join(format!("{SNAPSHOT_DIRECTORY_PREFIX}not-an-id"));
        fs::create_dir(&near_match).expect("near match should create");
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let outside = parent.join("outside");
            fs::create_dir(&outside).expect("outside should create");
            symlink(
                &outside,
                parent.join(format!("{SNAPSHOT_DIRECTORY_PREFIX}{}", "2".repeat(32))),
            )
            .expect("snapshot link should create");
        }

        scavenge_stale_snapshots(&parent, ScavengeMode::Startup, MISSING_LEASE_GRACE)
            .expect("scavenge should succeed");
        assert!(!stale.exists());
        assert!(near_match.exists());
        assert!(active.exists());
        assert!(creating.exists());
        drop(lease);
        scavenge_stale_snapshots(&parent, ScavengeMode::Concurrent, Duration::ZERO)
            .expect("released snapshot should scavenge");
        assert!(!active.exists());
        assert!(!creating.exists());
        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn snapshot_parent_creation_is_private_per_user_and_race_safe() {
        let root = fs::canonicalize(std::env::temp_dir())
            .expect("temporary root should canonicalize")
            .join(format!(
                "chariox-kernel-source-parent-{}-{}",
                std::process::id(),
                rand::random::<u64>()
            ));
        fs::create_dir(&root).expect("test temporary root should create");
        let canonical_root = fs::canonicalize(&root).expect("test temporary root should resolve");
        let mut workers = Vec::new();
        for _ in 0..8 {
            let root = canonical_root.clone();
            workers.push(std::thread::spawn(move || ensure_snapshot_parent(&root)));
        }
        let parents = workers
            .into_iter()
            .map(|worker| {
                worker
                    .join()
                    .expect("snapshot parent worker should not panic")
                    .expect("snapshot parent should create")
            })
            .collect::<Vec<_>>();
        assert!(parents.windows(2).all(|pair| pair[0] == pair[1]));
        let metadata = fs::metadata(&parents[0]).expect("snapshot parent metadata should read");
        validate_snapshot_parent_metadata(&metadata).expect("snapshot parent should be private");
        #[cfg(unix)]
        assert!(parents[0]
            .file_name()
            .and_then(|name| name.to_str())
            .expect("snapshot parent name should be UTF-8")
            .ends_with(&format!("-{}", unsafe { libc::geteuid() })));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn startup_scavenger_bounds_work_and_cleans_recent_orphans_on_retry() {
        let parent = fs::canonicalize(std::env::temp_dir())
            .expect("temporary root should canonicalize")
            .join(format!(
                "chariox-kernel-source-startup-scavenge-{}-{}",
                std::process::id(),
                rand::random::<u64>()
            ));
        fs::create_dir_all(&parent).expect("scavenge parent should create");
        let mut orphans = Vec::new();
        for index in 0..MAX_STARTUP_SCAVENGE_REMOVALS + 3 {
            let path = parent.join(format!("{SNAPSHOT_DIRECTORY_PREFIX}{index:032x}"));
            fs::create_dir(&path).expect("orphan snapshot should create");
            let lease = create_snapshot_lease(&path).expect("orphan lease should create");
            drop(lease);
            fs::write(path.join("plaintext.json"), "secret-canary")
                .expect("orphan payload should write");
            orphans.push(path);
        }

        let error = scavenge_stale_snapshots(&parent, ScavengeMode::Startup, Duration::ZERO)
            .expect_err("first bounded pass should report its backlog");
        assert!(matches!(
            error,
            DaemonError::ManagedContext {
                code: "kernel_context_cleanup_backlog",
                retryable: true,
                ..
            }
        ));
        scavenge_stale_snapshots(&parent, ScavengeMode::Startup, Duration::ZERO)
            .expect("second startup pass should finish cleanup");
        assert!(orphans.iter().all(|path| !path.exists()));
        let _ = fs::remove_dir_all(parent);
    }

    #[cfg(unix)]
    #[test]
    fn duplicated_directory_descriptor_is_close_on_exec() {
        use std::os::fd::AsRawFd;

        let root = fs::canonicalize(std::env::temp_dir()).expect("temporary root should resolve");
        let directory = unix::open_directory_chain(&root)
            .expect("directory should open")
            .expect("temporary root should exist");
        let duplicated = unix::duplicate_cloexec(&directory).expect("descriptor should duplicate");
        let flags = unsafe { libc::fcntl(duplicated.as_raw_fd(), libc::F_GETFD) };
        assert!(flags >= 0);
        assert_ne!(flags & libc::FD_CLOEXEC, 0);
    }

    #[test]
    fn private_source_snapshot_freezes_bytes_and_rejects_links_and_growth() {
        let _guard = crate::env_lock::lock();
        let root = fs::canonicalize(std::env::temp_dir())
            .expect("temporary root should canonicalize")
            .join(format!(
                "chariox-kernel-source-snapshot-{}-{}",
                std::process::id(),
                rand::random::<u64>()
            ));
        let isolation = root.join("capabilities");
        let home = root.join("home");
        std::env::set_var("CHARIOX_CAPABILITY_ISOLATION_ROOT", &isolation);
        std::env::set_var("CHARIOX_HOME", &home);
        let mcp_root = isolation.join("user/mcps");
        fs::create_dir_all(&mcp_root).expect("MCP root should create");
        fs::write(mcp_root.join("a.json"), b"first").expect("source should write");

        let snapshot =
            KernelContextSourceSnapshot::capture_once().expect("snapshot should capture");
        fs::write(mcp_root.join("a.json"), b"second").expect("source should change");
        fs::write(mcp_root.join("a.json"), b"first").expect("source should restore");
        assert_eq!(
            fs::read(snapshot.mcp_root.join("a.json")).expect("snapshot should read"),
            b"first"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            fs::remove_file(mcp_root.join("a.json")).expect("source should remove");
            symlink(root.join("outside"), mcp_root.join("a.json")).expect("link should create");
            assert!(KernelContextSourceSnapshot::capture_once().is_err());
            fs::remove_file(mcp_root.join("a.json")).expect("link should remove");

            fs::remove_dir_all(&mcp_root).expect("source root should remove");
            let outside_root = root.join("outside-root");
            fs::create_dir_all(&outside_root).expect("outside root should create");
            symlink(&outside_root, &mcp_root).expect("root link should create");
            assert!(KernelContextSourceSnapshot::capture_once().is_err());
            fs::remove_file(&mcp_root).expect("root link should remove");
            fs::create_dir_all(&mcp_root).expect("source root should recreate");
        }

        let oversized = mcp_root.join("large.json");
        File::create(&oversized)
            .and_then(|file| file.set_len(MAX_SOURCE_FILE_BYTES + 1))
            .expect("oversized sparse source should create");
        assert!(KernelContextSourceSnapshot::capture_once().is_err());

        std::env::remove_var("CHARIOX_CAPABILITY_ISOLATION_ROOT");
        std::env::remove_var("CHARIOX_HOME");
        fs::remove_dir_all(root).expect("test root should clean");
    }
}
