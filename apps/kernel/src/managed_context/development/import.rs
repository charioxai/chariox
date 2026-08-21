use super::*;

const PUBLICATION_RECEIPT_SCHEMA_VERSION: u32 = 2;
const PUBLICATION_RECEIPT_FILE: &str = ".chariox-managed-import-receipt.json";
pub(crate) const MAX_PUBLICATION_RECEIPT_BYTES: usize = 64 * 1024;

pub fn import_development_context(
    request: DevelopmentContextImportRequest,
) -> Result<DevelopmentContextImportResult, DaemonError> {
    import_development_context_with_options(
        request,
        MAX_CHECKOUT_BYTES_PER_PROJECT,
        MAX_MATERIALIZED_ENTRIES_PER_PROJECT,
        None,
    )
    .map(|(result, _)| result)
}

pub(crate) fn import_development_context_with_publication(
    request: DevelopmentContextImportRequest,
    publication_id: String,
) -> Result<DevelopmentContextPublicationReceipt, DaemonError> {
    validate_publication_id(&publication_id)?;
    let (_, receipt) = import_development_context_with_options(
        request,
        MAX_CHECKOUT_BYTES_PER_PROJECT,
        MAX_MATERIALIZED_ENTRIES_PER_PROJECT,
        Some(publication_id),
    )?;
    receipt.ok_or_else(|| context_error("development context publication receipt is missing"))
}

pub(crate) fn recover_development_context_publication(
    request: &DevelopmentContextImportRequest,
    publication_id: &str,
) -> Result<Option<DevelopmentContextPublicationReceipt>, DaemonError> {
    validate_import_request(request)?;
    validate_publication_id(publication_id)?;
    let destination_metadata = match fs::symlink_metadata(&request.destination_root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(context_io_error(
                "inspect published development context",
                error,
            ))
        }
    };
    if destination_metadata.file_type().is_symlink() || !destination_metadata.is_dir() {
        return Err(context_error(
            "published development context destination must be a real directory",
        ));
    }
    let canonical_destination = fs::canonicalize(&request.destination_root).map_err(|error| {
        context_io_error("resolve published development context destination", error)
    })?;
    let receipt = read_publication_receipt(&canonical_destination.join(PUBLICATION_RECEIPT_FILE))?
        .ok_or_else(|| {
            context_error("occupied development context destination has no publication receipt")
        })?;
    validate_publication_receipt(&receipt, request, publication_id, &canonical_destination)?;
    Ok(Some(receipt))
}

pub(crate) fn recover_pruned_development_context_publication(
    destination_parent: &Path,
    expected_project_id: &str,
    expected_repositories: &[DevelopmentSourceRepositoryBinding],
) -> Result<Option<DevelopmentContextPublicationReceipt>, DaemonError> {
    let expected_binding_sha256s = expected_repositories
        .iter()
        .map(source_repository_binding_sha256)
        .collect::<Vec<_>>();
    let parent_metadata = match fs::symlink_metadata(destination_parent) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(context_io_error(
                "inspect managed context publication parent",
                error,
            ))
        }
    };
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        return Err(context_error(
            "managed context publication parent must be a real directory",
        ));
    }
    let canonical_parent = fs::canonicalize(destination_parent)
        .map_err(|error| context_io_error("resolve managed context publication parent", error))?;
    let mut entries = Vec::new();
    for entry in fs::read_dir(&canonical_parent)
        .map_err(|error| context_io_error("list managed context publications", error))?
    {
        if entries.len() == 256 {
            return Err(context_error(
                "managed context publication recovery exceeds its entry limit",
            ));
        }
        entries.push(
            entry.map_err(|error| {
                context_io_error("read managed context publication entry", error)
            })?,
        );
    }
    entries.sort_by_key(|entry| entry.file_name());

    let mut matching = None;
    for entry in entries {
        let metadata = entry
            .file_type()
            .map_err(|error| context_io_error("inspect managed context publication", error))?;
        if !metadata.is_dir() || metadata.is_symlink() {
            continue;
        }
        let Some(publication_id) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if validate_publication_id(&publication_id).is_err() {
            continue;
        }
        let destination = fs::canonicalize(entry.path())
            .map_err(|error| context_io_error("resolve managed context publication", error))?;
        if destination.parent() != Some(canonical_parent.as_path()) {
            return Err(context_error(
                "managed context publication escaped its private parent",
            ));
        }
        let receipt = read_publication_receipt(&destination.join(PUBLICATION_RECEIPT_FILE))?
            .ok_or_else(|| context_error("managed context publication receipt is missing"))?;
        if receipt.project_id != expected_project_id
            || receipt.repositories.len() != expected_repositories.len()
            || !receipt
                .repositories
                .iter()
                .zip(expected_repositories)
                .all(|(repository, expected)| repository.role == expected.role)
        {
            continue;
        }
        if receipt.schema_version != PUBLICATION_RECEIPT_SCHEMA_VERSION
            || receipt.source_repository_binding_sha256s.len() != expected_repositories.len()
        {
            return Err(context_error(
                "legacy managed context publication lacks exact source repository bindings",
            ));
        }
        if receipt.source_repository_binding_sha256s != expected_binding_sha256s {
            continue;
        }
        let validation_request = DevelopmentContextImportRequest {
            archive_path: PathBuf::new(),
            expected_archive_sha256: receipt.archive_sha256.clone(),
            expected_project_id: expected_project_id.to_string(),
            expected_source_repositories: Some(expected_repositories.to_vec()),
            destination_root: destination.clone(),
        };
        validate_publication_receipt(&receipt, &validation_request, &publication_id, &destination)?;
        if matching.replace(receipt).is_some() {
            return Err(context_error(
                "managed context publication recovery is ambiguous",
            ));
        }
    }
    Ok(matching)
}

pub(crate) fn cleanup_development_context_publication(
    destination_root: &Path,
    publication_id: &str,
) -> Result<(), DaemonError> {
    validate_publication_id(publication_id)?;
    let metadata = match fs::symlink_metadata(destination_root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(context_io_error(
                "inspect failed development context publication",
                error,
            ))
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(context_error(
            "failed development context publication is not a real directory",
        ));
    }
    let canonical_destination = fs::canonicalize(destination_root).map_err(|error| {
        context_io_error("resolve failed development context publication", error)
    })?;
    let receipt = read_publication_receipt(&canonical_destination.join(PUBLICATION_RECEIPT_FILE))?
        .ok_or_else(|| {
            context_error("refusing to remove a development context publication without a receipt")
        })?;
    if receipt.publication_id != publication_id || receipt.destination_root != canonical_destination
    {
        return Err(context_error(
            "refusing to remove a different development context publication",
        ));
    }
    fs::remove_dir_all(&canonical_destination).map_err(|error| {
        context_io_error("remove failed development context publication", error)
    })?;
    #[cfg(unix)]
    if let Some(parent) = canonical_destination.parent() {
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| {
                context_io_error("sync failed development context publication parent", error)
            })?;
    }
    Ok(())
}

pub(super) fn import_development_context_with_budgets(
    request: DevelopmentContextImportRequest,
    maximum_project_checkout_bytes: u64,
    maximum_project_materialized_entries: u64,
) -> Result<DevelopmentContextImportResult, DaemonError> {
    import_development_context_with_options(
        request,
        maximum_project_checkout_bytes,
        maximum_project_materialized_entries,
        None,
    )
    .map(|(result, _)| result)
}

fn import_development_context_with_options(
    request: DevelopmentContextImportRequest,
    maximum_project_checkout_bytes: u64,
    maximum_project_materialized_entries: u64,
    publication_id: Option<String>,
) -> Result<
    (
        DevelopmentContextImportResult,
        Option<DevelopmentContextPublicationReceipt>,
    ),
    DaemonError,
> {
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

    let staging_root = match publication_id.as_deref() {
        Some(publication_id) => {
            create_publication_staging_directory(&canonical_parent, publication_id)?
        }
        None => create_unique_private_directory(&canonical_parent, ".tmp-chariox-context-import")?,
    };
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

    let manifest = extract_and_verify_archive(
        archive_file,
        &request.expected_project_id,
        request.expected_source_repositories.as_deref(),
        &artifacts_root,
    )?;
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
    let result = DevelopmentContextImportResult {
        manifest,
        destination_root: destination_root.clone(),
        primary_repository_id,
        repositories: imported,
    };
    let publication_receipt =
        publication_id.map(|publication_id| DevelopmentContextPublicationReceipt {
            schema_version: PUBLICATION_RECEIPT_SCHEMA_VERSION,
            publication_id,
            archive_sha256: request.expected_archive_sha256.to_ascii_lowercase(),
            project_id: request.expected_project_id.clone(),
            destination_root: destination_root.clone(),
            primary_repository_id: result.primary_repository_id.clone(),
            source_repository_binding_sha256s: result
                .manifest
                .repositories
                .iter()
                .map(|repository| repository.source_binding_sha256.clone())
                .collect(),
            repositories: result.repositories.clone(),
        });
    if let Some(receipt) = &publication_receipt {
        let bytes = serde_json::to_vec(receipt)
            .map_err(|error| context_error(format!("serialize import receipt: {error}")))?;
        if bytes.len() > MAX_PUBLICATION_RECEIPT_BYTES {
            return Err(context_error(
                "development context publication receipt exceeds its size limit",
            ));
        }
        write_private_file(&project_root.join(PUBLICATION_RECEIPT_FILE), &bytes)?;
    }
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

    Ok((result, publication_receipt))
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

fn validate_publication_id(publication_id: &str) -> Result<(), DaemonError> {
    if publication_id.is_empty()
        || publication_id.len() > 128
        || !publication_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(context_error(
            "development context publication id is invalid",
        ));
    }
    Ok(())
}

pub(crate) fn cleanup_development_context_publication_staging(
    destination_root: &Path,
    publication_id: &str,
) -> Result<(), DaemonError> {
    validate_publication_id(publication_id)?;
    let parent = destination_root.parent().ok_or_else(|| {
        context_error("development context publication destination has no parent")
    })?;
    let canonical_parent = match fs::canonicalize(parent) {
        Ok(parent) => parent,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(context_io_error(
                "resolve publication import staging parent",
                error,
            ))
        }
    };
    if canonical_parent != parent {
        return Err(context_error(
            "development context publication staging parent binding changed",
        ));
    }
    remove_publication_staging_directory(&publication_staging_path(
        &canonical_parent,
        publication_id,
    ))
}

fn create_publication_staging_directory(
    canonical_parent: &Path,
    publication_id: &str,
) -> Result<PathBuf, DaemonError> {
    let staging = publication_staging_path(canonical_parent, publication_id);
    remove_publication_staging_directory(&staging)?;
    fs::create_dir(&staging)
        .map_err(|error| context_io_error("create publication import staging", error))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&staging, fs::Permissions::from_mode(0o700))
            .map_err(|error| context_io_error("secure publication import staging", error))?;
    }
    sync_directory(canonical_parent)?;
    Ok(staging)
}

fn publication_staging_path(parent: &Path, publication_id: &str) -> PathBuf {
    parent.join(format!(
        ".tmp-chariox-context-import-{publication_id}.staging"
    ))
}

fn remove_publication_staging_directory(path: &Path) -> Result<(), DaemonError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(context_io_error(
                "inspect publication import staging",
                error,
            ))
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(context_error(
            "publication import staging must be a real directory",
        ));
    }
    fs::remove_dir_all(path)
        .map_err(|error| context_io_error("remove publication import staging", error))?;
    if let Some(parent) = path.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}

fn read_publication_receipt(
    path: &Path,
) -> Result<Option<DevelopmentContextPublicationReceipt>, DaemonError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(context_io_error("open import publication receipt", error)),
    };
    let metadata = file
        .metadata()
        .map_err(|error| context_io_error("inspect import publication receipt", error))?;
    if !metadata.is_file() || metadata.len() > MAX_PUBLICATION_RECEIPT_BYTES as u64 {
        return Err(context_error(
            "development context publication receipt must be a bounded regular file",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_PUBLICATION_RECEIPT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| context_io_error("read import publication receipt", error))?;
    if bytes.len() > MAX_PUBLICATION_RECEIPT_BYTES {
        return Err(context_error(
            "development context publication receipt exceeds its size limit",
        ));
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| context_error("development context publication receipt is invalid"))
}

fn validate_publication_receipt(
    receipt: &DevelopmentContextPublicationReceipt,
    request: &DevelopmentContextImportRequest,
    publication_id: &str,
    canonical_destination: &Path,
) -> Result<(), DaemonError> {
    if receipt.schema_version != PUBLICATION_RECEIPT_SCHEMA_VERSION
        || receipt.publication_id != publication_id
        || receipt.archive_sha256 != request.expected_archive_sha256.to_ascii_lowercase()
        || receipt.project_id != request.expected_project_id
        || receipt.destination_root != canonical_destination
        || receipt.repositories.is_empty()
        || receipt.repositories.len() > MAX_REPOSITORIES
        || receipt.source_repository_binding_sha256s.len() != receipt.repositories.len()
        || receipt
            .source_repository_binding_sha256s
            .iter()
            .any(|digest| {
                digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
    {
        return Err(context_error(
            "development context publication receipt does not match the import",
        ));
    }
    if let Some(expected) = request.expected_source_repositories.as_deref() {
        let expected_binding_sha256s = expected
            .iter()
            .map(source_repository_binding_sha256)
            .collect::<Vec<_>>();
        if receipt.source_repository_binding_sha256s != expected_binding_sha256s {
            return Err(context_error(
                "development context publication source repositories do not match the import",
            ));
        }
    }
    let mut repository_ids = BTreeSet::new();
    let mut target_directories = BTreeSet::new();
    let mut primary_ids = Vec::new();
    for repository in &receipt.repositories {
        validate_publication_id(&repository.repository_id)?;
        validate_git_oid(&repository.head_sha)?;
        let target = Path::new(&repository.target_directory);
        if repository.target_directory.is_empty()
            || repository.target_directory.len() > 255
            || !target
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
            || target.components().count() != 1
            || !repository_ids.insert(repository.repository_id.clone())
            || !target_directories.insert(repository.target_directory.clone())
            || repository.destination_path
                != canonical_destination.join(&repository.target_directory)
        {
            return Err(context_error(
                "development context publication receipt has invalid repository mappings",
            ));
        }
        let repository_metadata = fs::symlink_metadata(&repository.destination_path)
            .map_err(|error| context_io_error("inspect published repository destination", error))?;
        if repository_metadata.file_type().is_symlink() || !repository_metadata.is_dir() {
            return Err(context_error(
                "published repository destination must be a real directory",
            ));
        }
        if git_text_isolated(&repository.destination_path, &["rev-parse", "HEAD"])?
            != repository.head_sha
        {
            return Err(context_error(
                "published repository head does not match its receipt",
            ));
        }
        if repository.role == DevelopmentRepositoryRole::Primary {
            primary_ids.push(repository.repository_id.as_str());
        }
    }
    if primary_ids.as_slice() != [receipt.primary_repository_id.as_str()] {
        return Err(context_error(
            "development context publication receipt has invalid primary repository mapping",
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
