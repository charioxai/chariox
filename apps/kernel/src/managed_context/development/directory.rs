//! Directory workspaces use the same checked file objects as Git overlays.
//! They never initialize Git or synthesize commits at either endpoint.
use super::*;

pub(super) fn export_directory(
    selection: &DevelopmentRepositorySelection,
    worktree: &Path,
    staging_root: &Path,
    repository_ids: &mut BTreeSet<String>,
    target_directories: &mut BTreeSet<String>,
    budget: &mut ManifestMemoryBudget,
) -> Result<
    (
        DevelopmentRepositoryManifest,
        RepositoryMaterializationEstimate,
    ),
    DaemonError,
> {
    let logical_name = export::repository_logical_name(worktree, None);
    let repository_id =
        export::unique_repository_id(None, "directory", &logical_name, repository_ids);
    let target_directory =
        export::unique_target_directory(&logical_name, &repository_id, target_directories);
    budget.consume(logical_name.len() + target_directory.len() + 2048)?;
    let root = open_root(worktree)?;
    let (entries, directories, size) =
        snapshot(worktree, &root, &repository_id, staging_root, budget)?;
    let verification = staging_root.join("directory-verification");
    create_private_directory(&verification)?;
    let verified = snapshot(
        worktree,
        &root,
        &repository_id,
        &verification,
        &mut ManifestMemoryBudget::new(),
    )?;
    fs::remove_dir_all(&verification)
        .map_err(|error| context_io_error("remove directory verification", error))?;
    if (&entries, &directories, size) != (&verified.0, &verified.1, verified.2)
        || fs::symlink_metadata(worktree.join(".git")).is_ok()
    {
        return Err(context_error(
            "directory workspace changed while exporting; retry",
        ));
    }
    let estimate = charge_overlay_materialization(
        RepositoryMaterializationEstimate {
            checkout_bytes: directories.len() as u64 * 4096,
            materialized_entries: directories.len() as u64,
        },
        &entries,
        MAX_CHECKOUT_BYTES_PER_REPOSITORY,
        MAX_MATERIALIZED_ENTRIES_PER_REPOSITORY,
    )?;
    Ok((
        DevelopmentRepositoryManifest {
            workspace_kind: DevelopmentWorkspaceKind::Directory,
            directories,
            repository_id,
            source_binding_sha256: source_repository_binding_sha256(
                &DevelopmentSourceRepositoryBinding {
                    role: selection.role,
                    workspace_id: selection.workspace_id.clone(),
                    worktree_id: selection.worktree_id.clone(),
                },
            ),
            logical_name,
            role: selection.role,
            target_directory,
            // Legacy Git fields remain empty for schema-3 directory workspaces.
            head_sha: String::new(),
            branch: None,
            upstream: None,
            origin_url: None,
            bundle_path: String::new(),
            bundle_sha256: String::new(),
            bundle_size_bytes: 0,
            overlay: entries,
            overlay_size_bytes: size,
        },
        estimate,
    ))
}

fn snapshot(
    worktree: &Path,
    root: &File,
    repository_id: &str,
    staging: &Path,
    budget: &mut ManifestMemoryBudget,
) -> Result<(Vec<DevelopmentOverlayEntry>, Vec<String>, u64), DaemonError> {
    let patterns = match open_relative(root, Path::new(".charioxignore")) {
        Ok(file) => {
            overlay::parse_context_ignore_patterns(read_bounded(file, MAX_CONTEXT_IGNORE_BYTES)?.0)?
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(context_io_error("open directory ignore rules", error)),
    };
    let mut pending = vec![PathBuf::new()];
    let mut directories = BTreeSet::new();
    let mut files = BTreeSet::new();
    let mut scanned = 0;
    while let Some(relative) = pending.pop() {
        for entry in fs::read_dir(worktree.join(&relative))
            .map_err(|error| context_io_error("enumerate workspace directory", error))?
        {
            let entry =
                entry.map_err(|error| context_io_error("read workspace directory entry", error))?;
            scanned += 1;
            if scanned > MAX_MATERIALIZED_ENTRIES_PER_REPOSITORY {
                return Err(context_error(
                    "directory workspace exceeds its entry budget",
                ));
            }
            let path = relative.join(entry.file_name());
            let path_text = path
                .to_str()
                .ok_or_else(|| context_error("directory workspace entry is not UTF-8"))?
                .to_string();
            validate_relative_path(&path_text)?;
            if overlay::context_force_excluded_path(&path_text)
                || overlay::user_ignore_pattern_matches_any(&patterns, &path_text)
            {
                continue;
            }
            // Resolve every component under the retained root, without following
            // symlinks, even if a directory was replaced during enumeration.
            let file = open_relative(root, &path)
                .map_err(|error| context_io_error("open directory workspace entry", error))?;
            let metadata = file
                .metadata()
                .map_err(|error| context_io_error("inspect directory workspace entry", error))?;
            budget.consume(path_text.len() + 256)?;
            if directories.len() + files.len() >= MAX_OVERLAY_FILES_PER_REPOSITORY {
                return Err(context_error("directory workspace has too many paths"));
            }
            if metadata.is_dir() {
                directories.insert(path_text);
                pending.push(path);
            } else if metadata.is_file() {
                files.insert(path_text);
            } else {
                return Err(context_error("directory workspace contains a special file"));
            }
        }
    }
    let mut stored = BTreeSet::new();
    let mut size = 0;
    let mut entries = Vec::new();
    for path in files {
        let file = open_relative(root, Path::new(&path))
            .map_err(|error| context_io_error("open workspace file", error))?;
        let (bytes, metadata) = read_bounded(file, MAX_OVERLAY_FILE_BYTES)?;
        overlay::validate_overlay_file_bytes(&path, &bytes)?;
        let state = overlay::store_overlay_object(
            repository_id,
            bytes,
            file_is_executable(&metadata),
            staging,
            &mut stored,
            &mut size,
        )?;
        budget.consume(overlay::file_state_manifest_bytes(&state))?;
        entries.push(DevelopmentOverlayEntry {
            path,
            index: DevelopmentFileState::Absent,
            worktree: state,
        });
    }
    Ok((entries, directories.into_iter().collect(), size))
}

fn read_bounded(mut file: File, limit: u64) -> Result<(Vec<u8>, fs::Metadata), DaemonError> {
    let metadata = file
        .metadata()
        .map_err(|error| context_io_error("inspect workspace file", error))?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(context_error(
            "workspace file must be a bounded regular file",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    (&mut file)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| context_io_error("read workspace file", error))?;
    if bytes.len() as u64 > limit {
        return Err(context_error("workspace file grew beyond its size budget"));
    }
    Ok((bytes, metadata))
}

#[cfg(unix)]
fn open_root(path: &Path) -> Result<File, DaemonError> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|error| context_io_error("open workspace root", error))
}

#[cfg(unix)]
fn open_relative(root: &File, path: &Path) -> io::Result<File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    let mut parent = root.try_clone()?;
    let mut parts = path.components().peekable();
    while let Some(component) = parts.next() {
        let Component::Normal(name) = component else {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        };
        let name = std::ffi::CString::new(name.as_bytes())
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        let flags = libc::O_RDONLY
            | libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | libc::O_NONBLOCK
            | if parts.peek().is_some() {
                libc::O_DIRECTORY
            } else {
                0
            };
        // SAFETY: parent owns a live descriptor, name is NUL-terminated, and a
        // successful openat transfers one new owned descriptor to File.
        let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        parent = unsafe { File::from_raw_fd(fd) };
    }
    Ok(parent)
}

#[cfg(not(unix))]
fn open_root(_: &Path) -> Result<File, DaemonError> {
    Err(context_error(
        "directory workspace export requires a Unix host",
    ))
}
#[cfg(not(unix))]
fn open_relative(_: &File, _: &Path) -> io::Result<File> {
    Err(io::Error::from(io::ErrorKind::Unsupported))
}
