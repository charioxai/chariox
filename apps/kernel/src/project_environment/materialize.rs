//! MP-08: Private target files and launch bindings shared by every placement.
use super::resolver::environment_error;
use super::*;
use crate::error::DaemonError;
use crate::secret::CredentialVaultStore;
use std::collections::BTreeMap;
use std::path::PathBuf;
use zeroize::Zeroizing;

/// Values are returned only to kernel launch code, never a client projection.
pub fn project_environment_launch_bindings(
    manifest: &ProjectEnvironmentManifest,
    resolved: &ResolvedProjectEnvironment,
    workspace_id: &str,
) -> Result<BTreeMap<String, Zeroizing<String>>, DaemonError> {
    let mut bindings = BTreeMap::new();
    for entry in &manifest.entries {
        if entry.workspace_id == workspace_id && entry.kind == ProjectEnvironmentEntryKind::Variable
        {
            if !resolved
                .values
                .contains_key(&(entry.workspace_id.clone(), entry.name.clone()))
            {
                continue;
            }
            if project_environment_protected_name(&entry.name) {
                return Err(environment_error(
                    "Project environment cannot override kernel launch controls",
                ));
            }
            let value = resolved
                .values
                .get(&(entry.workspace_id.clone(), entry.name.clone()))
                .ok_or_else(|| environment_error("Project environment resolution incomplete"))?;
            bindings.insert(entry.name.clone(), value.clone());
        }
    }
    Ok(bindings)
}

/// Write only referenced assignments to env files, never copy unrelated keys.
/// Target publication is staged by M28 before this operation is called.
pub fn materialize_project_environment(
    manifest: &ProjectEnvironmentManifest,
    resolved: &ResolvedProjectEnvironment,
    workspaces: &BTreeMap<String, PathBuf>,
) -> Result<(), DaemonError> {
    for ((workspace, path), value) in project_environment_files(manifest, resolved)? {
        let root = workspaces
            .get(&workspace)
            .ok_or_else(|| environment_error("materialization workspace not selected"))?;
        write_private_workspace_file(root, &path, value.as_bytes())?;
    }
    Ok(())
}

pub(super) fn project_environment_files(
    manifest: &ProjectEnvironmentManifest,
    resolved: &ResolvedProjectEnvironment,
) -> Result<BTreeMap<(String, String), Zeroizing<String>>, DaemonError> {
    manifest.validate().map_err(environment_error)?;

    let config_paths: std::collections::BTreeSet<_> = manifest
        .entries
        .iter()
        .filter(|entry| entry.kind == ProjectEnvironmentEntryKind::ConfigFile)
        .map(|entry| (&entry.workspace_id, &entry.name))
        .collect();
    if manifest.entries.iter().any(|entry| matches!(&entry.locator,
        ProjectEnvironmentLocator::EnvFile {path, ..} if config_paths.contains(&(&entry.workspace_id, path)))) {
        return Err(environment_error("configuration and environment assignment paths collide"));
    }
    let mut files: BTreeMap<(String, String), Zeroizing<String>> = BTreeMap::new();
    for entry in &manifest.entries {
        if !resolved
            .values
            .contains_key(&(entry.workspace_id.clone(), entry.name.clone()))
        {
            continue;
        }
        let value = resolved
            .values
            .get(&(entry.workspace_id.clone(), entry.name.clone()))
            .ok_or_else(|| environment_error("Project environment resolution incomplete"))?;
        let path = match (&entry.kind, &entry.locator) {
            (
                ProjectEnvironmentEntryKind::Variable,
                ProjectEnvironmentLocator::EnvFile { path, key },
            ) => {
                if value.contains(['\n', '\r']) {
                    return Err(environment_error(
                        "multiline env-file value requires configuration input",
                    ));
                }
                let output = files
                    .entry((entry.workspace_id.clone(), path.clone()))
                    .or_default();
                // Double-quoted dotenv literals with escaped interpolation and quotes.
                output.push_str(key);
                output.push_str("=\"");
                for character in value.chars() {
                    if matches!(character, '\\' | '"' | '$' | '`') {
                        output.push('\\');
                    }
                    output.push(character);
                }
                output.push_str("\"\n");
                continue;
            }
            (ProjectEnvironmentEntryKind::ConfigFile, _) => &entry.name,
            _ => continue,
        };
        if files
            .insert((entry.workspace_id.clone(), path.clone()), value.clone())
            .is_some()
        {
            return Err(environment_error(
                "duplicate environment materialization path",
            ));
        }
    }
    Ok(files)
}
/// Imported workspace-environment and supplied values become target-owned Vault locators.
/// File locators stay relative so edits on the target are picked up by its next export.
pub fn target_project_environment_manifest(
    source: &ProjectEnvironmentManifest,
    workspace_mapping: &BTreeMap<String, String>,
    vault: &dyn CredentialVaultStore,
) -> Result<ProjectEnvironmentManifest, DaemonError> {
    let mut target = source.clone();
    let service = project_environment_vault_service(&target.project_id);
    for file in &mut target.private_files {
        file.workspace_id = workspace_mapping
            .get(&file.workspace_id)
            .ok_or_else(|| environment_error("target private file workspace mapping incomplete"))?
            .clone();
    }
    for entry in &mut target.entries {
        let old_key = project_environment_vault_key(entry);
        entry.workspace_id = workspace_mapping
            .get(&entry.workspace_id)
            .ok_or_else(|| environment_error("target environment workspace mapping incomplete"))?
            .clone();
        if entry.status != ProjectEnvironmentEntryStatus::Found {
            continue;
        }
        let value = Zeroizing::new(vault.get_secret(&service, &old_key)?);
        let new_key = project_environment_vault_key(entry);
        vault.set_secret(&service, &new_key, &value)?;
        if matches!(
            entry.locator,
            ProjectEnvironmentLocator::WorkspaceEnvironment { .. }
                | ProjectEnvironmentLocator::Vault { .. }
                | ProjectEnvironmentLocator::Missing
        ) {
            entry.locator = ProjectEnvironmentLocator::Vault {
                service: service.clone(),
                key: new_key,
            };
        }
        entry.status = ProjectEnvironmentEntryStatus::Found;
    }
    target.validate().map_err(environment_error)?;
    Ok(target)
}

fn write_private_workspace_file(
    root: &std::path::Path,
    path: &str,
    value: &[u8],
) -> Result<(), DaemonError> {
    write_private_workspace_file_owned(root, path, value, WorkspaceFilePublication::Replace)?
        .commit();
    Ok(())
}

pub(super) enum WorkspaceFilePublication {
    Create,
    Replace,
    ReuseMatching,
}

pub(super) fn write_private_workspace_file_owned(
    root: &std::path::Path,
    path: &str,
    value: &[u8],
    publication: WorkspaceFilePublication,
) -> Result<super::materialization_transaction::MaterializedFile, DaemonError> {
    relative_environment_path(path).map_err(environment_error)?;
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::os::unix::fs::OpenOptionsExt;
        let mut parent = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(root)
            .map_err(|_| environment_error("target workspace unavailable"))?;
        let parts: Vec<_> = path.split('/').collect();
        for part in &parts[..parts.len() - 1] {
            let component = std::ffi::CString::new(*part)
                .map_err(|_| environment_error("invalid environment path"))?;
            let result = unsafe { libc::mkdirat(parent.as_raw_fd(), component.as_ptr(), 0o700) };
            if result != 0
                && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists
            {
                return Err(environment_error("create target config directory failed"));
            }
            let fd = unsafe {
                libc::openat(
                    parent.as_raw_fd(),
                    component.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                return Err(environment_error(
                    "target config directory is not owned or safe",
                ));
            }
            parent = unsafe { std::fs::File::from_raw_fd(fd) };
        }
        let name = std::ffi::CString::new(*parts.last().unwrap())
            .map_err(|_| environment_error("invalid environment path"))?;
        if matches!(publication, WorkspaceFilePublication::ReuseMatching) {
            if let Some(file) = reuse_matching_config(&parent, &name, value)? {
                return Ok(file);
            }
        }
        let temporary =
            std::ffi::CString::new(format!(".chariox-env-{}", rand::random::<u64>())).unwrap();
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                temporary.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err(environment_error("create target config file failed"));
        }
        let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
        let cleanup_parent = parent
            .try_clone()
            .map_err(|_| environment_error("target config directory unavailable"))?;
        let result = (|| {
            file.write_all(value)
                .and_then(|_| file.sync_all())
                .map_err(|_| environment_error("write target config file failed"))?;
            let published = unsafe {
                if !matches!(publication, WorkspaceFilePublication::Replace) {
                    libc::linkat(
                        parent.as_raw_fd(),
                        temporary.as_ptr(),
                        parent.as_raw_fd(),
                        name.as_ptr(),
                        0,
                    )
                } else {
                    libc::renameat(
                        parent.as_raw_fd(),
                        temporary.as_ptr(),
                        parent.as_raw_fd(),
                        name.as_ptr(),
                    )
                }
            };
            if published != 0 {
                return Err(environment_error(
                    "target config file exists or cannot be published",
                ));
            }
            let owned =
                super::materialization_transaction::MaterializedFile::new(parent, name, &file)?;
            owned.sync()?;
            Ok(owned)
        })();
        unsafe { libc::unlinkat(cleanup_parent.as_raw_fd(), temporary.as_ptr(), 0) };
        result
    }
    #[cfg(not(unix))]
    {
        let _ = (root, value, publication);
        Err(environment_error(
            "private Project environment materialization unsupported on this platform",
        ))
    }
}

/// MP-08 / MP-10 / MP-11: Reuse an identical mounted config without replacing its inode.
/// Keep its old permissions until commit, so failed leased installs restore source state.
#[cfg(unix)]
fn reuse_matching_config(
    parent: &std::fs::File,
    name: &std::ffi::CString,
    value: &[u8],
) -> Result<Option<super::materialization_transaction::MaterializedFile>, DaemonError> {
    use std::io::Read;
    use std::os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{MetadataExt, PermissionsExt},
    };
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        if std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
            return Ok(None);
        }
        return Err(environment_error("mounted configuration is not safe"));
    }
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    let metadata = file
        .metadata()
        .map_err(|_| environment_error("mounted configuration unavailable"))?;
    // chmod must not affect a second path through a hard link.
    if !metadata.is_file() || metadata.nlink() != 1 || metadata.len() != value.len() as u64 {
        return Err(environment_error(
            "mounted configuration differs from selected input",
        ));
    }
    let mut current = Zeroizing::new(Vec::new());
    (&mut file)
        .take(value.len() as u64 + 1)
        .read_to_end(&mut current)
        .map_err(|_| environment_error("mounted configuration unavailable"))?;
    if current.as_slice() != value {
        return Err(environment_error(
            "mounted configuration differs from selected input",
        ));
    }
    let after_read = file
        .metadata()
        .map_err(|_| environment_error("mounted configuration unavailable"))?;
    if after_read.len() != metadata.len()
        || after_read.mtime() != metadata.mtime()
        || after_read.mtime_nsec() != metadata.mtime_nsec()
    {
        return Err(environment_error(
            "mounted configuration changed during reconciliation",
        ));
    }
    // MP-08 / MP-10 / MP-11: a shared checkout may belong to the publishing
    // kernel. Group write access does not authorize chmod on its config inode.
    #[cfg(target_os = "linux")]
    if metadata.uid() != unsafe { libc::geteuid() } && unsafe { libc::geteuid() } != 0 {
        return replace_matching_foreign_config(parent, name, value, &metadata).map(Some);
    }
    let rollback_file = file
        .try_clone()
        .map_err(|_| environment_error("mounted configuration unavailable"))?;
    let mut owned = super::materialization_transaction::MaterializedFile::new(
        parent
            .try_clone()
            .map_err(|_| environment_error("target config directory unavailable"))?,
        name.clone(),
        &file,
    )?;
    // Retain the descriptor before changing permissions; Drop never reopens a replacement.
    owned.preserve_permissions(rollback_file);
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
        .and_then(|_| file.sync_all())
        .map_err(|_| environment_error("private mounted configuration permissions failed"))?;
    owned.sync()?;
    Ok(Some(owned))
}

/// MP-08 / MP-10 / MP-11: make the matching config target-owned while retaining
/// its original inode for rollback. Never replace a differing source/target file.
#[cfg(target_os = "linux")]
fn replace_matching_foreign_config(
    parent: &std::fs::File,
    name: &std::ffi::CString,
    value: &[u8],
    original: &std::fs::Metadata,
) -> Result<super::materialization_transaction::MaterializedFile, DaemonError> {
    use std::io::Write;
    use std::os::fd::{AsRawFd, FromRawFd};
    let temporary =
        std::ffi::CString::new(format!(".chariox-env-{}", rand::random::<u64>())).unwrap();
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            temporary.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err(environment_error(
            "create private mounted configuration failed",
        ));
    }
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    let mut staged = super::materialization_transaction::MaterializedFile::new(
        parent
            .try_clone()
            .map_err(|_| environment_error("target config directory unavailable"))?,
        temporary.clone(),
        &file,
    )?;
    let written = file.write_all(value).and_then(|_| file.sync_all());
    // Even a partial write belongs to this transaction's failure cleanup.
    staged.refresh_identity(&file)?;
    written.map_err(|_| environment_error("write private mounted configuration failed"))?;
    staged.exchange_matching(name, original)?;
    staged.sync()?;
    Ok(staged)
}
