use super::*;

const PUBLICATION_RECEIPT_SCHEMA_VERSION: u32 = 2;
const DIRECTORY_RECEIPT_SCHEMA_VERSION: u32 = 3;
const PUBLICATION_RECEIPT_FILE: &str = ".chariox-managed-import-receipt.json";
const MATERIALIZATION_TRANSACTION_FILE: &str = ".chariox-materialization-transaction.json";
const DEFAULT_USER_WORKSPACE_ROOT: &str = "/home/chariox";
pub(crate) const MAX_PUBLICATION_RECEIPT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct MaterializationTransaction {
    materialization_root: PathBuf,
    target_directories: Vec<String>,
    #[serde(default)]
    published_target_directories: Vec<String>,
}

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
    validate_publication_receipt(
        &receipt,
        request,
        publication_id,
        &canonical_destination,
        true,
        true,
    )?;
    Ok(Some(receipt))
}

pub(crate) fn recover_pruned_development_context_publication(
    destination_parent: &Path,
    expected_project_id: &str,
    expected_repositories: &[DevelopmentSourceRepositoryBinding],
) -> Result<Option<DevelopmentContextPublicationReceipt>, DaemonError> {
    recover_pruned_development_context_publication_with_head_policy(
        destination_parent,
        expected_project_id,
        expected_repositories,
        true,
        true,
    )
}

pub(crate) fn recover_pruned_mutable_development_context_publication(
    destination_parent: &Path,
    expected_project_id: &str,
    expected_repositories: &[DevelopmentSourceRepositoryBinding],
) -> Result<Option<DevelopmentContextPublicationReceipt>, DaemonError> {
    recover_pruned_development_context_publication_with_head_policy(
        destination_parent,
        expected_project_id,
        expected_repositories,
        false,
        true,
    )
}

pub(crate) fn recover_pruned_development_context_publication_for_cleanup(
    destination_parent: &Path,
    expected_project_id: &str,
    expected_repositories: &[DevelopmentSourceRepositoryBinding],
) -> Result<Option<DevelopmentContextPublicationReceipt>, DaemonError> {
    recover_pruned_development_context_publication_with_head_policy(
        destination_parent,
        expected_project_id,
        expected_repositories,
        false,
        false,
    )
}

fn recover_pruned_development_context_publication_with_head_policy(
    destination_parent: &Path,
    expected_project_id: &str,
    expected_repositories: &[DevelopmentSourceRepositoryBinding],
    require_original_head: bool,
    require_repository_directories: bool,
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
        if ![
            PUBLICATION_RECEIPT_SCHEMA_VERSION,
            DIRECTORY_RECEIPT_SCHEMA_VERSION,
        ]
        .contains(&receipt.schema_version)
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
        validate_publication_receipt(
            &receipt,
            &validation_request,
            &publication_id,
            &destination,
            require_original_head,
            require_repository_directories,
        )?;
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
    if receipt.publication_id != publication_id {
        return Err(context_error(
            "refusing to remove a different development context publication",
        ));
    }
    let request = DevelopmentContextImportRequest {
        archive_path: PathBuf::new(),
        expected_archive_sha256: receipt.archive_sha256.clone(),
        expected_project_id: receipt.project_id.clone(),
        expected_source_repositories: None,
        destination_root: canonical_destination.clone(),
    };
    validate_publication_receipt(
        &receipt,
        &request,
        publication_id,
        &canonical_destination,
        false,
        false,
    )?;
    let materialization_root = publication_materialization_root(&receipt, &canonical_destination)?;
    if materialization_root != canonical_destination {
        for repository in &receipt.repositories {
            let path = &repository.destination_path;
            match fs::symlink_metadata(path) {
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                    return Err(context_error(
                        "refusing to remove a non-directory managed repository destination",
                    ))
                }
                Ok(_) => fs::remove_dir_all(path).map_err(|error| {
                    context_io_error("remove managed repository materialization", error)
                })?,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(context_io_error(
                        "inspect managed repository materialization",
                        error,
                    ))
                }
            }
        }
        sync_directory(&materialization_root)?;
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

#[cfg(test)]
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
    ensure_path_absent(&request.destination_root, "development context destination")?;
    fs::create_dir_all(destination_parent).map_err(|error| {
        context_io_error("create development context destination parent", error)
    })?;
    let canonical_parent = fs::canonicalize(destination_parent).map_err(|error| {
        context_io_error("resolve development context destination parent", error)
    })?;
    let control_destination_root = canonical_parent.join(destination_name);
    ensure_path_absent(&control_destination_root, "development context destination")?;
    let materialization_root = if publication_id.is_some() {
        managed_materialization_root_for_control(&control_destination_root)?
            .unwrap_or_else(|| control_destination_root.clone())
    } else {
        control_destination_root.clone()
    };
    if materialization_root != control_destination_root {
        validate_real_directory(&materialization_root, "default user workspace root")?;
    }

    let staging_root = match publication_id.as_deref() {
        Some(publication_id) => {
            create_publication_staging_directory(&canonical_parent, publication_id)?
        }
        None => create_unique_private_directory(&canonical_parent, ".tmp-chariox-context-import")?,
    };
    let mut cleanup = ImportCleanup::new(staging_root.clone());
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
    if materialization_root != control_destination_root {
        for repository in &manifest.repositories {
            let destination = materialization_root.join(&repository.target_directory);
            ensure_path_absent(&destination, "managed repository destination")?;
        }
        write_materialization_transaction(
            &staging_root,
            &materialization_root,
            &manifest.repositories,
        )?;
    }
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
            workspace_kind: repository.workspace_kind,
            repository_id: repository.repository_id.clone(),
            role: repository.role,
            target_directory: repository.target_directory.clone(),
            destination_path: materialization_root.join(&repository.target_directory),
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
        destination_root: control_destination_root.clone(),
        primary_repository_id,
        repositories: imported,
    };
    let publication_receipt =
        publication_id.map(|publication_id| DevelopmentContextPublicationReceipt {
            schema_version: if result
                .repositories
                .iter()
                .any(|repository| !repository.workspace_kind.is_git())
            {
                DIRECTORY_RECEIPT_SCHEMA_VERSION
            } else {
                PUBLICATION_RECEIPT_SCHEMA_VERSION
            },
            publication_id,
            archive_sha256: request.expected_archive_sha256.to_ascii_lowercase(),
            project_id: request.expected_project_id.clone(),
            destination_root: control_destination_root.clone(),
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
    let mut published_materializations = Vec::new();
    if materialization_root != control_destination_root {
        for repository in &result.manifest.repositories {
            let source = project_root.join(&repository.target_directory);
            let destination = materialization_root.join(&repository.target_directory);
            if let Err(error) = publish_directory_no_clobber(&source, &destination) {
                rollback_materializations(&published_materializations);
                return Err(error);
            }
            published_materializations.push(destination);
            if let Err(error) = update_materialization_transaction(
                &staging_root,
                &materialization_root,
                &result.manifest.repositories,
                &published_materializations,
            ) {
                rollback_materializations(&published_materializations);
                return Err(error);
            }
        }
        if let Err(error) = sync_directory(&materialization_root) {
            rollback_materializations(&published_materializations);
            return Err(error);
        }
    }
    if let Err(error) = sync_directory(&project_root) {
        rollback_materializations(&published_materializations);
        return Err(error);
    }
    if let Err(error) = publish_directory_no_clobber(&project_root, &control_destination_root) {
        rollback_materializations(&published_materializations);
        return Err(error);
    }
    if let Err(error) = sync_directory(&canonical_parent) {
        let _ = fs::remove_dir_all(&control_destination_root);
        let _ = sync_directory(&canonical_parent);
        rollback_materializations(&published_materializations);
        return Err(error);
    }
    cleanup.commit();
    drop(cleanup);

    Ok((result, publication_receipt))
}

fn managed_materialization_root_for_control(
    control_destination: &Path,
) -> Result<Option<PathBuf>, DaemonError> {
    if control_destination.parent().and_then(Path::file_name)
        != Some(std::ffi::OsStr::new("managed-context-workspaces"))
    {
        return Ok(None);
    }
    let root = PathBuf::from(DEFAULT_USER_WORKSPACE_ROOT);
    validate_real_directory(&root, "default user workspace root")?;
    let canonical = fs::canonicalize(&root)
        .map_err(|error| context_io_error("resolve default user workspace root", error))?;
    if canonical == control_destination
        || control_destination.starts_with(&canonical)
        || canonical.starts_with(
            control_destination
                .parent()
                .ok_or_else(|| context_error("managed context control root has no parent"))?,
        )
    {
        return Err(context_error(
            "default user workspace root must remain separate from managed context control state",
        ));
    }
    Ok(Some(canonical))
}

fn validate_real_directory(path: &Path, label: &str) -> Result<(), DaemonError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| context_io_error("inspect managed materialization root", error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(context_error(format!("{label} must be a real directory")));
    }
    Ok(())
}

fn write_materialization_transaction(
    staging_root: &Path,
    materialization_root: &Path,
    repositories: &[DevelopmentRepositoryManifest],
) -> Result<(), DaemonError> {
    let transaction = MaterializationTransaction {
        materialization_root: materialization_root.to_path_buf(),
        target_directories: repositories
            .iter()
            .map(|repository| repository.target_directory.clone())
            .collect(),
        published_target_directories: Vec::new(),
    };
    write_materialization_transaction_value(staging_root, &transaction)
}

fn update_materialization_transaction(
    staging_root: &Path,
    materialization_root: &Path,
    repositories: &[DevelopmentRepositoryManifest],
    published_paths: &[PathBuf],
) -> Result<(), DaemonError> {
    let transaction = MaterializationTransaction {
        materialization_root: materialization_root.to_path_buf(),
        target_directories: repositories
            .iter()
            .map(|repository| repository.target_directory.clone())
            .collect(),
        published_target_directories: published_paths
            .iter()
            .map(|path| {
                path.file_name()
                    .and_then(OsStr::to_str)
                    .map(str::to_string)
                    .ok_or_else(|| context_error("materialized repository path is not valid UTF-8"))
            })
            .collect::<Result<Vec<_>, _>>()?,
    };
    write_materialization_transaction_value(staging_root, &transaction)
}

fn write_materialization_transaction_value(
    staging_root: &Path,
    transaction: &MaterializationTransaction,
) -> Result<(), DaemonError> {
    let bytes = serde_json::to_vec(transaction).map_err(|error| {
        context_error(format!("serialize materialization transaction: {error}"))
    })?;
    crate::config::write_private_file(&staging_root.join(MATERIALIZATION_TRANSACTION_FILE), &bytes)
        .map_err(|error| context_io_error("persist materialization transaction", error))
}

fn rollback_materializations(paths: &[PathBuf]) {
    for path in paths.iter().rev() {
        let _ = fs::remove_dir_all(path);
    }
}

fn publication_materialization_root(
    receipt: &DevelopmentContextPublicationReceipt,
    control_destination: &Path,
) -> Result<PathBuf, DaemonError> {
    if receipt.destination_root != control_destination {
        return Err(context_error(
            "managed context publication control root does not match its receipt",
        ));
    }
    if receipt.repositories.iter().all(|repository| {
        super::export::validate_repository_basename(&repository.target_directory).is_ok()
            && repository.destination_path == control_destination.join(&repository.target_directory)
    }) {
        return Ok(control_destination.to_path_buf());
    }
    let Some(configured) = managed_materialization_root_for_control(control_destination)? else {
        return Err(context_error(
            "managed context publication materialization root is unavailable",
        ));
    };
    if receipt.repositories.iter().all(|repository| {
        super::export::validate_repository_basename(&repository.target_directory).is_ok()
            && repository.destination_path == configured.join(&repository.target_directory)
    }) {
        return Ok(configured);
    }
    Err(context_error(
        "managed context publication materialization root does not match the trusted user workspace root",
    ))
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
    cleanup_materialization_transaction(path)?;
    fs::remove_dir_all(path)
        .map_err(|error| context_io_error("remove publication import staging", error))?;
    if let Some(parent) = path.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}

fn cleanup_materialization_transaction(staging_root: &Path) -> Result<(), DaemonError> {
    let marker = staging_root.join(MATERIALIZATION_TRANSACTION_FILE);
    let bytes = match fs::read(&marker) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(context_io_error("read materialization transaction", error)),
    };
    if bytes.len() > MAX_PUBLICATION_RECEIPT_BYTES {
        return Err(context_error(
            "materialization transaction exceeds its size limit",
        ));
    }
    let transaction: MaterializationTransaction = serde_json::from_slice(&bytes)
        .map_err(|_| context_error("materialization transaction is invalid"))?;
    validate_real_directory(
        &transaction.materialization_root,
        "materialization transaction root",
    )?;
    for target in &transaction.published_target_directories {
        super::export::validate_repository_basename(target)?;
        let destination = transaction.materialization_root.join(target);
        match fs::symlink_metadata(&destination) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(context_error(
                    "materialization transaction destination is not a real directory",
                ))
            }
            Ok(_) => fs::remove_dir_all(&destination)
                .map_err(|error| context_io_error("roll back materialized repository", error))?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(context_io_error(
                    "inspect materialization transaction destination",
                    error,
                ))
            }
        }
    }
    sync_directory(&transaction.materialization_root)
}

fn ensure_path_absent(path: &Path, label: &str) -> Result<(), DaemonError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(context_error(format!(
            "{label} `{}` already exists",
            path.display()
        ))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(context_io_error(
            "inspect path before managed context materialization",
            error,
        )),
    }
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
    require_original_head: bool,
    require_repository_directories: bool,
) -> Result<(), DaemonError> {
    let materialization_root = publication_materialization_root(receipt, canonical_destination)?;
    if ![
        PUBLICATION_RECEIPT_SCHEMA_VERSION,
        DIRECTORY_RECEIPT_SCHEMA_VERSION,
    ]
    .contains(&receipt.schema_version)
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
        super::export::validate_repository_basename(&repository.target_directory)?;
        if repository.workspace_kind.is_git() {
            validate_git_oid(&repository.head_sha)?;
        } else if receipt.schema_version != DIRECTORY_RECEIPT_SCHEMA_VERSION
            || !repository.head_sha.is_empty()
        {
            return Err(context_error(
                "directory publication receipt contains invalid Git metadata",
            ));
        }
        if !repository_ids.insert(repository.repository_id.clone())
            || !target_directories.insert(repository.target_directory.to_lowercase())
            || repository.destination_path
                != materialization_root.join(&repository.target_directory)
        {
            return Err(context_error(
                "development context publication receipt has invalid repository mappings",
            ));
        }
        if require_repository_directories {
            let repository_metadata =
                fs::symlink_metadata(&repository.destination_path).map_err(|error| {
                    context_io_error("inspect published repository destination", error)
                })?;
            if repository_metadata.file_type().is_symlink() || !repository_metadata.is_dir() {
                return Err(context_error(
                    "published repository destination must be a real directory",
                ));
            }
            if require_original_head
                && repository.workspace_kind.is_git()
                && git_text_isolated(&repository.destination_path, &["rev-parse", "HEAD"])?
                    != repository.head_sha
            {
                return Err(context_error(
                    "published repository head does not match its receipt",
                ));
            }
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
    committed: bool,
}

impl ImportCleanup {
    fn new(staging_root: PathBuf) -> Self {
        Self {
            staging_root,
            committed: false,
        }
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for ImportCleanup {
    fn drop(&mut self) {
        if !self.committed {
            let _ = cleanup_materialization_transaction(&self.staging_root);
        }
        let _ = fs::remove_dir_all(&self.staging_root);
    }
}
