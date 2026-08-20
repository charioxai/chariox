use super::*;
pub fn import_development_context(
    request: DevelopmentContextImportRequest,
) -> Result<DevelopmentContextImportResult, DaemonError> {
    import_development_context_with_budgets(
        request,
        MAX_CHECKOUT_BYTES_PER_PROJECT,
        MAX_MATERIALIZED_ENTRIES_PER_PROJECT,
    )
}

pub(super) fn import_development_context_with_budgets(
    request: DevelopmentContextImportRequest,
    maximum_project_checkout_bytes: u64,
    maximum_project_materialized_entries: u64,
) -> Result<DevelopmentContextImportResult, DaemonError> {
    validate_import_request(&request)?;
    let destination_parent = request.destination_root.parent().ok_or_else(|| {
        context_error("development context destination must have a parent directory")
    })?;
    let destination_name = request
        .destination_root
        .file_name()
        .ok_or_else(|| context_error("development context destination must have a file name"))?;
    if destination_name.len() > 255
        || destination_name.to_str().is_none()
        || !Path::new(destination_name)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(context_error(
            "development context destination name is invalid",
        ));
    }
    if request.destination_root.exists() {
        return Err(context_error(format!(
            "development context destination `{}` already exists",
            request.destination_root.display()
        )));
    }
    fs::create_dir_all(destination_parent).map_err(|error| {
        context_io_error("create development context destination parent", error)
    })?;
    let canonical_parent = fs::canonicalize(destination_parent).map_err(|error| {
        context_io_error("resolve development context destination parent", error)
    })?;
    let destination_root = canonical_parent.join(destination_name);
    if destination_root.exists() {
        return Err(context_error(format!(
            "development context destination `{}` already exists",
            destination_root.display()
        )));
    }

    let staging_root =
        create_unique_private_directory(&canonical_parent, ".tmp-chariox-context-import")?;
    let cleanup = ImportCleanup::new(staging_root.clone());
    let archive_snapshot = staging_root.join("archive.snapshot.tar.gz");
    let (archive_file, archive_size, archive_sha256) =
        snapshot_and_hash_archive(&request.archive_path, &archive_snapshot)?;
    if archive_size > MAX_PACKAGE_BYTES {
        return Err(context_error(format!(
            "development context archive is {archive_size} bytes; maximum is {MAX_PACKAGE_BYTES}"
        )));
    }
    if archive_sha256 != request.expected_archive_sha256.to_ascii_lowercase() {
        return Err(context_error(
            "development context archive digest does not match the expected digest",
        ));
    }
    let artifacts_root = staging_root.join("artifacts");
    let project_root = staging_root.join("project");
    create_private_directory(&artifacts_root)?;
    create_private_directory(&project_root)?;

    let manifest =
        extract_and_verify_archive(archive_file, &request.expected_project_id, &artifacts_root)?;
    let mut imported = Vec::with_capacity(manifest.repositories.len());
    let mut checkout_bytes = 0_u64;
    let mut materialized_entries = 0_u64;
    for repository in &manifest.repositories {
        let repository_root = project_root.join(&repository.target_directory);
        let remaining_bytes = maximum_project_checkout_bytes.saturating_sub(checkout_bytes);
        let remaining_entries =
            maximum_project_materialized_entries.saturating_sub(materialized_entries);
        let estimate = prepare_repository(
            repository,
            &artifacts_root,
            &repository_root,
            remaining_bytes,
            remaining_entries,
        )?;
        checkout_bytes = checkout_bytes.saturating_add(estimate.checkout_bytes);
        materialized_entries = materialized_entries.saturating_add(estimate.materialized_entries);
        imported.push(DevelopmentImportedRepository {
            repository_id: repository.repository_id.clone(),
            role: repository.role,
            target_directory: repository.target_directory.clone(),
            destination_path: destination_root.join(&repository.target_directory),
            head_sha: repository.head_sha.clone(),
        });
    }
    for repository in &manifest.repositories {
        materialize_prepared_repository(
            repository,
            &artifacts_root,
            &project_root.join(&repository.target_directory),
        )?;
    }
    let primary_repository_id = imported
        .iter()
        .find(|repository| repository.role == DevelopmentRepositoryRole::Primary)
        .map(|repository| repository.repository_id.clone())
        .ok_or_else(|| context_error("imported context has no primary repository"))?;
    fs::remove_dir_all(&artifacts_root)
        .map_err(|error| context_io_error("remove imported context artifacts", error))?;
    sync_directory(&project_root)?;
    publish_directory_no_clobber(&project_root, &destination_root)?;
    if let Err(error) = sync_directory(&canonical_parent) {
        let _ = fs::remove_dir_all(&destination_root);
        let _ = sync_directory(&canonical_parent);
        return Err(error);
    }
    drop(cleanup);

    Ok(DevelopmentContextImportResult {
        manifest,
        destination_root,
        primary_repository_id,
        repositories: imported,
    })
}

pub(super) fn snapshot_and_hash_archive(
    path: &Path,
    snapshot_path: &Path,
) -> Result<(File, u64, String), DaemonError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| context_io_error("inspect development context archive", error))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(context_error(
            "development context archive must be a regular file and cannot be a symlink",
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
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
    let mut source = options
        .open(path)
        .map_err(|error| context_io_error("open development context archive", error))?;
    let opened_metadata = source
        .metadata()
        .map_err(|error| context_io_error("inspect opened development context archive", error))?;
    if !opened_metadata.is_file() {
        return Err(context_error(
            "development context archive must remain a regular file",
        ));
    }
    if opened_metadata.len() > MAX_PACKAGE_BYTES {
        return Err(context_error(format!(
            "development context archive is {} bytes; maximum is {MAX_PACKAGE_BYTES}",
            opened_metadata.len()
        )));
    }
    let mut snapshot = private_create_new(snapshot_path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut hashed_bytes = 0_u64;
    let mut bounded = (&mut source).take(MAX_PACKAGE_BYTES.saturating_add(1));
    loop {
        let read = bounded
            .read(&mut buffer)
            .map_err(|error| context_io_error("hash development context archive", error))?;
        if read == 0 {
            break;
        }
        hashed_bytes = hashed_bytes.saturating_add(read as u64);
        hasher.update(&buffer[..read]);
        snapshot
            .write_all(&buffer[..read])
            .map_err(|error| context_io_error("snapshot development context archive", error))?;
    }
    if hashed_bytes > MAX_PACKAGE_BYTES {
        return Err(context_error(format!(
            "development context archive grew beyond {MAX_PACKAGE_BYTES} bytes while hashing"
        )));
    }
    drop(bounded);
    snapshot
        .sync_all()
        .map_err(|error| context_io_error("sync development context archive snapshot", error))?;
    drop(snapshot);
    let snapshot = File::open(snapshot_path)
        .map_err(|error| context_io_error("open development context archive snapshot", error))?;
    Ok((snapshot, hashed_bytes, format!("{:x}", hasher.finalize())))
}

fn validate_import_request(request: &DevelopmentContextImportRequest) -> Result<(), DaemonError> {
    if request.expected_project_id.trim().is_empty()
        || request.expected_project_id.len() > MAX_CONTEXT_IDENTIFIER_BYTES
    {
        return Err(context_error(
            "expected development context project id is invalid",
        ));
    }
    if request.expected_archive_sha256.len() != 64
        || !request
            .expected_archive_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(context_error(
            "expected development context archive digest must be SHA-256",
        ));
    }
    Ok(())
}

fn sync_directory(path: &Path) -> Result<(), DaemonError> {
    #[cfg(unix)]
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| context_io_error("sync imported project directory", error))?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn publish_directory_no_clobber(source: &Path, destination: &Path) -> Result<(), DaemonError> {
    use std::os::unix::ffi::OsStrExt;
    let source = std::ffi::CString::new(source.as_os_str().as_bytes())
        .map_err(|_| context_error("import staging path contains NUL"))?;
    let destination = std::ffi::CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| context_error("import destination path contains NUL"))?;
    let result =
        unsafe { libc::renamex_np(source.as_ptr(), destination.as_ptr(), libc::RENAME_EXCL) };
    if result == 0 {
        Ok(())
    } else {
        Err(context_io_error(
            "publish imported project",
            io::Error::last_os_error(),
        ))
    }
}

#[cfg(target_os = "linux")]
fn publish_directory_no_clobber(source: &Path, destination: &Path) -> Result<(), DaemonError> {
    use std::os::unix::ffi::OsStrExt;
    let source = std::ffi::CString::new(source.as_os_str().as_bytes())
        .map_err(|_| context_error("import staging path contains NUL"))?;
    let destination = std::ffi::CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| context_error("import destination path contains NUL"))?;
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(context_io_error(
            "publish imported project",
            io::Error::last_os_error(),
        ))
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn publish_directory_no_clobber(source: &Path, destination: &Path) -> Result<(), DaemonError> {
    if destination.exists() {
        return Err(context_error(format!(
            "development context destination `{}` already exists",
            destination.display()
        )));
    }
    fs::rename(source, destination)
        .map_err(|error| context_io_error("publish imported project", error))
}

struct ImportCleanup {
    staging_root: PathBuf,
}

impl ImportCleanup {
    fn new(staging_root: PathBuf) -> Self {
        Self { staging_root }
    }
}

impl Drop for ImportCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.staging_root);
    }
}
